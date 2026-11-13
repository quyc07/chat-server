//! Timing tests for the push link's liveness watchdog.
//!
//! The clock is injected: the watchdog never reads a clock itself, every call
//! takes the instant to judge. That is what lets the 15-second threshold be
//! exercised in microseconds instead of real seconds — nothing here sleeps, and
//! nothing here needs a runtime.
//!
//! The signal under test is the one the server actually sends. The SSE response
//! wrapper writes a `: keep-alive-text` comment every 5 seconds on its own timer,
//! independent of the application event loop; the named `Heartbeat` event only
//! arrives every 60 seconds. Liveness therefore means "any bytes at all", and
//! three keep-alive periods of silence is a dead link.

use std::time::{Duration, Instant};

use chat_tui::link::{KEEP_ALIVE_PERIOD, Liveness, SILENCE_TIMEOUT};

fn at_seconds(base: Instant, seconds: u64) -> Instant {
    base + Duration::from_secs(seconds)
}

#[test]
fn three_keep_alive_periods_is_the_silence_threshold() {
    assert_eq!(KEEP_ALIVE_PERIOD, Duration::from_secs(5));
    assert_eq!(SILENCE_TIMEOUT, Duration::from_secs(15));
    assert_eq!(SILENCE_TIMEOUT, KEEP_ALIVE_PERIOD * 3);
}

#[test]
fn keep_alive_comments_every_five_seconds_keep_the_link_alive_past_a_heartbeat_period() {
    // A healthy link sends only keep-alive comments for the first minute: the
    // named `Heartbeat` event is not due until 60s. Treating that event as the
    // signal would declare a disconnect roughly every minute.
    let start = Instant::now();
    let mut link = Liveness::new(start);

    for second in (5..=120).step_by(5) {
        let now = at_seconds(start, second);
        assert!(
            !link.expired(now),
            "the link was declared dead at {second}s although keep-alives were arriving"
        );
        link.saw_bytes(now);
    }

    assert!(!link.expired(at_seconds(start, 134)));
    assert!(
        link.expired(at_seconds(start, 135)),
        "three missed keep-alives must be a dead link"
    );
}

#[test]
fn fourteen_seconds_of_silence_is_not_yet_a_disconnect() {
    let start = Instant::now();
    let link = Liveness::new(start);

    assert!(!link.expired(at_seconds(start, 14)));
    assert_eq!(link.deadline(), at_seconds(start, 15));
}

#[test]
fn the_window_restarts_from_the_newest_byte_not_from_the_connection() {
    let start = Instant::now();
    let mut link = Liveness::new(start);

    // One keep-alive comment at 10s, then nothing.
    let last = at_seconds(start, 10);
    link.saw_bytes(last);

    assert!(
        !link.expired(at_seconds(start, 24)),
        "14s after the newest byte is still inside the window"
    );
    assert!(
        link.expired(at_seconds(start, 25)),
        "15s after the newest byte is the disconnect"
    );
}

#[test]
fn bytes_of_any_kind_count() {
    // The seam takes bytes, not events: a comment line is as good as a heartbeat
    // event, which is exactly the correction this ticket carries.
    let start = Instant::now();
    let mut link = Liveness::new(start);

    link.saw_bytes(at_seconds(start, 3));
    link.saw_bytes(at_seconds(start, 14));
    link.saw_bytes(at_seconds(start, 20));

    assert!(!link.expired(at_seconds(start, 34)));
    assert!(link.expired(at_seconds(start, 35)));
}
