//! Push-link liveness and redialing.
//!
//! # What counts as alive
//!
//! The server's SSE response emits a `: keep-alive-text` comment every **5
//! seconds** from the response wrapper, on its own timer, whether or not the
//! application event loop is still alive. The named `Heartbeat` event is only
//! sent every **60 seconds**. Using the event as the liveness signal would
//! declare a perfectly healthy connection dead about once a minute, so liveness
//! here is "any bytes at all" — comment lines included — and the threshold is
//! three keep-alive periods of silence ([`SILENCE_TIMEOUT`]).
//!
//! # The two ways a link dies
//!
//! - **Clean end of response.** When the server's broadcast receiver falls
//!   behind, its event loop `break`s and the response ends with no error event.
//!   A clean end is therefore a disconnect, not "we are finished".
//! - **Network drop.** Nothing arrives and nothing closes; only the missing
//!   keep-alive comments reveal it, hence the byte-silence timeout.
//!
//! Both lead to the same place: report [`Event::LinkDown`], wait, and redial.
//!
//! # Time is a parameter
//!
//! [`Liveness`] never reads a clock: every call takes the instant to judge, so
//! the 15-second window is testable in microseconds (`tests/link_liveness.rs`).
//! [`spawn_with_delay`] injects the redial delay for the same reason
//! (`tests/link_wire.rs`); [`spawn`] is the production entry point.

use std::time::{Duration, Instant};

use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::watch;
use tokio::time;

use crate::api::{ApiClient, ApiError};
use crate::model::{self, StreamEvent};
use crate::sse;
use crate::state::Event;

/// The response wrapper's keep-alive period.
pub const KEEP_ALIVE_PERIOD: Duration = Duration::from_secs(5);

/// Three missed keep-alive comments: the link is dead.
pub const SILENCE_TIMEOUT: Duration = Duration::from_secs(15);

/// Wait before redialing, so an unreachable server is not hammered and the user
/// can still type into the message box while the link is down.
pub const RECONNECT_DELAY: Duration = Duration::from_secs(1);

/// Byte watchdog for one connection.
///
/// The clock is injected: callers pass the instant they want judged. In the
/// client those instants come from the read loop; in tests they are made up.
#[derive(Debug, Clone)]
pub struct Liveness {
    deadline: Instant,
}

impl Liveness {
    /// Start a window that expires [`SILENCE_TIMEOUT`] after `now`.
    pub fn new(now: Instant) -> Self {
        Self {
            deadline: now + SILENCE_TIMEOUT,
        }
    }

    /// Some bytes arrived (an event, a comment line, half a frame — any of them).
    pub fn saw_bytes(&mut self, now: Instant) {
        self.deadline = now + SILENCE_TIMEOUT;
    }

    /// When the current silence becomes a disconnect.
    pub fn deadline(&self) -> Instant {
        self.deadline
    }

    /// Whether the link has been silent for the whole window by `now`.
    pub fn expired(&self, now: Instant) -> bool {
        now >= self.deadline
    }
}

/// Why one connection stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ending {
    /// The token was replaced or cleared: reconnect with whatever is current.
    TokenChanged,
    /// The stream rejected the token. The 401 is already reported; there is no
    /// point calling this a network drop on top of `TokenExpired`.
    Unauthorized,
    /// The response ended, or never opened. The server's silent death and an
    /// unreachable server both land here.
    Closed,
    /// No bytes for [`SILENCE_TIMEOUT`]: a network drop.
    Silent,
    /// The event channel is gone; the process is shutting down.
    Stop,
}

/// Own the push connection for the process lifetime: connect when a token
/// exists, report the link down when it dies, redial, and follow token changes.
pub fn spawn(tx: UnboundedSender<Event>, api: ApiClient, tokens: watch::Receiver<Option<String>>) {
    spawn_with_delay(tx, api, tokens, RECONNECT_DELAY);
}

/// [`spawn`] with the redial wait injected. A test passes a tiny delay so the
/// redial is exercised without spending real time; the production client always
/// goes through [`spawn`].
pub fn spawn_with_delay(
    tx: UnboundedSender<Event>,
    api: ApiClient,
    mut tokens: watch::Receiver<Option<String>>,
    redial: Duration,
) {
    tokio::spawn(async move {
        let mut current = tokens.borrow().clone();
        loop {
            let Some(token) = current.clone() else {
                // No session: nothing to connect to. A token change is the only
                // thing that can wake this up.
                if tokens.changed().await.is_err() {
                    return;
                }
                current = tokens.borrow().clone();
                continue;
            };

            let ending = stream_once(&tx, &api, &token, &mut tokens).await;
            if tx.is_closed() {
                return;
            }
            match ending {
                Ending::Stop => return,
                Ending::TokenChanged => {
                    // A login replaced the token; redial right away with it.
                    current = tokens.borrow().clone();
                    continue;
                }
                // The session is still valid: tell the UI its silence is not the
                // other side being quiet, then wait for the redial.
                Ending::Closed | Ending::Silent => {
                    let _ = tx.send(Event::LinkDown);
                }
                // A 401 already sent `TokenExpired`; wait for the token to change.
                Ending::Unauthorized => {}
            }

            // Wait out the redial delay, but wake early if the token changes (a
            // new login must not sit out the old token's retry schedule).
            let woke = tokio::select! {
                _ = time::sleep(redial) => false,
                changed = tokens.changed() => changed.is_err(),
            };
            if woke {
                return;
            }
            let next = tokens.borrow().clone();
            if next != current {
                current = next;
            }
        }
    });
}

/// One connection's lifetime: open it, read chunks until it dies or the token
/// changes, and frame what arrives.
async fn stream_once(
    tx: &UnboundedSender<Event>,
    api: &ApiClient,
    token: &str,
    tokens: &mut watch::Receiver<Option<String>>,
) -> Ending {
    let mut response = match api.open_stream(token).await {
        Ok(response) => response,
        Err(ApiError::Unauthorized(_)) => {
            // The stream rejected the token: treat it like any other 401 so the
            // user is sent back to the login screen instead of retrying forever.
            let _ = tx.send(Event::TokenExpired);
            return Ending::Unauthorized;
        }
        // Unreachable server, 5xx, unusable body. The link is down either way.
        Err(_) => return Ending::Closed,
    };

    // The response is open and the wrapper has started its keep-alive timer.
    if tx.send(Event::LinkUp).is_err() {
        return Ending::Stop;
    }

    let mut parser = sse::Parser::new();
    let mut liveness = Liveness::new(Instant::now());
    loop {
        // The decision is made here, against a clock we pass in, so a silence
        // that started while a chunk future was still pending cannot be missed.
        if liveness.expired(Instant::now()) {
            return Ending::Silent;
        }
        let deadline = time::Instant::from_std(liveness.deadline());

        tokio::select! {
            changed = tokens.changed() => {
                return match changed {
                    Ok(()) => Ending::TokenChanged,
                    Err(_) => Ending::Stop,
                };
            }
            // The timer firing only re-runs the loop, where `expired` decides.
            _ = time::sleep_until(deadline) => {}
            chunk = response.chunk() => {
                let now = Instant::now();
                let chunk = match chunk {
                    Ok(Some(chunk)) => chunk,
                    // Clean end, transport error, or unusable body: all dead.
                    Ok(None) | Err(_) => return Ending::Closed,
                };
                // Any bytes count as liveness, keep-alive comments included.
                liveness.saw_bytes(now);
                let frames = match parser.push(&chunk) {
                    Ok(frames) => frames,
                    Err(_) => return Ending::Closed,
                };
                for frame in frames {
                    match model::stream_event(&frame.event, &frame.data) {
                        Ok(Some(StreamEvent::MessageReceived(message))) => {
                            if tx.send(Event::MessageReceived(message)).is_err() {
                                return Ending::Stop;
                            }
                        }
                        // Heartbeats are not the liveness signal; the bytes that
                        // carry them already reset the watchdog above.
                        Ok(Some(StreamEvent::Heartbeat)) | Ok(None) => {}
                        // One malformed frame never takes the whole stream down.
                        Err(_) => {}
                    }
                }
            }
        }
    }
}
