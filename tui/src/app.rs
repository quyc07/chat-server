//! Wiring: tokio owns the loop that connects terminal input, HTTP results, and
//! the pure state machine.
//!
//! Every fallible interaction is reported back as an [`Event`]; the loop itself
//! only stops on `Effect::Quit` (or an unrecoverable terminal error), so an
//! unreachable server leaves the client alive on the same screen.

use std::io;
use std::time::Duration;

use crossterm::event::{
    self, Event as TerminalEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
};
use tokio::sync::mpsc::{self, UnboundedSender};
use tokio::time::{self, Instant, MissedTickBehavior};

use crate::api::{ApiClient, ApiError};
use crate::render;
use crate::state::{self, Effect, Event, Key, Machine, PAGE_LIMIT};

const TICK: Duration = Duration::from_secs(1);
const LOGOUT_TIMEOUT: Duration = Duration::from_secs(2);

pub async fn run(server: String) -> io::Result<()> {
    let mut machine = Machine::new();
    let mut api = ApiClient::new(server);
    let (tx, mut rx) = mpsc::unbounded_channel::<Event>();

    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, &mut machine, &mut api, &tx, &mut rx).await;
    ratatui::restore();
    result
}

async fn event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    machine: &mut Machine,
    api: &mut ApiClient,
    tx: &UnboundedSender<Event>,
    rx: &mut mpsc::UnboundedReceiver<Event>,
) -> io::Result<()> {
    spawn_input(tx.clone());
    spawn_ticker(tx.clone());

    if let Ok(size) = terminal.size() {
        machine.handle(Event::Resize {
            width: size.width,
            height: size.height,
        });
    }

    loop {
        terminal.draw(|frame| render::draw(frame, machine.state()))?;

        let Some(event) = rx.recv().await else {
            return Ok(());
        };
        let effects = machine.handle(event);
        sync_token(api, machine.state());

        for effect in effects {
            match effect {
                Effect::Login { name, password } => {
                    let mut api = api.clone();
                    let tx = tx.clone();
                    tokio::spawn(async move {
                        let event = match api.login(&name, &password).await {
                            Ok(token) => Event::LoginSucceeded { token },
                            Err(error) => Event::LoginFailed(error.message()),
                        };
                        let _ = tx.send(event);
                    });
                }
                Effect::Register { name, password } => {
                    let mut api = api.clone();
                    let tx = tx.clone();
                    tokio::spawn(async move {
                        let event = match api.register(&name, &password).await {
                            Ok(_id) => Event::RegisterSucceeded,
                            Err(error) => Event::RegisterFailed(error.message()),
                        };
                        let _ = tx.send(event);
                    });
                }
                Effect::LoadHistory { page } => {
                    let api = api.clone();
                    let tx = tx.clone();
                    tokio::spawn(async move {
                        let event = match api.history(page, PAGE_LIMIT).await {
                            Ok(items) => Event::ConversationsLoaded { page, items },
                            Err(ApiError::Unauthorized(_)) => Event::TokenExpired,
                            Err(error) => Event::ConversationsFailed(error.message()),
                        };
                        let _ = tx.send(event);
                    });
                }
                Effect::LoadMessages { peer } => {
                    let api = api.clone();
                    let tx = tx.clone();
                    tokio::spawn(async move {
                        let conversation = crate::model::ConversationId::User(peer);
                        let event = match api.messages(peer).await {
                            Ok(items) => Event::MessagesLoaded {
                                conversation,
                                items,
                            },
                            Err(ApiError::Unauthorized(_)) => Event::TokenExpired,
                            Err(error) => Event::MessagesFailed {
                                conversation,
                                message: error.message(),
                            },
                        };
                        let _ = tx.send(event);
                    });
                }
                Effect::Relogin { name, password } => {
                    let mut api = api.clone();
                    let tx = tx.clone();
                    tokio::spawn(async move {
                        let event = match api.login(&name, &password).await {
                            Ok(token) => Event::ReloginSucceeded { token },
                            Err(error) => Event::ReloginFailed(error.message()),
                        };
                        let _ = tx.send(event);
                    });
                }
                Effect::Logout { token } => {
                    // Best effort: never keep the user waiting on a dead server.
                    let _ = time::timeout(LOGOUT_TIMEOUT, api.logout_with(&token)).await;
                    api.set_token(None);
                }
                Effect::FindUsers { query } => {
                    let api = api.clone();
                    let tx = tx.clone();
                    tokio::spawn(async move {
                        let event = match api.find_users(&query).await {
                            Ok(users) => Event::UsersFound { query, users },
                            Err(ApiError::Unauthorized(_)) => Event::TokenExpired,
                            Err(error) => Event::UserSearchFailed(error.message()),
                        };
                        let _ = tx.send(event);
                    });
                }
                Effect::SendFriendRequest { target } => {
                    let api = api.clone();
                    let tx = tx.clone();
                    tokio::spawn(async move {
                        let target_name = target.name.clone();
                        let event = match api.send_friend_request(target.id, None).await {
                            Ok(outcome) => Event::FriendRequestSent {
                                target: target_name.clone(),
                                outcome,
                            },
                            Err(ApiError::Unauthorized(_)) => Event::TokenExpired,
                            Err(error) => Event::FriendRequestSendFailed {
                                target: target_name,
                                message: error.message(),
                            },
                        };
                        let _ = tx.send(event);
                    });
                }
                Effect::LoadFriendRequests => {
                    let api = api.clone();
                    let tx = tx.clone();
                    tokio::spawn(async move {
                        let event = match api.friend_requests().await {
                            Ok(items) => Event::FriendRequestsLoaded(items),
                            Err(ApiError::Unauthorized(_)) => Event::TokenExpired,
                            Err(error) => Event::FriendRequestsFailed(error.message()),
                        };
                        let _ = tx.send(event);
                    });
                }
                Effect::LoadFriends => {
                    let api = api.clone();
                    let tx = tx.clone();
                    tokio::spawn(async move {
                        let event = match api.friends().await {
                            Ok(items) => Event::FriendsLoaded(items),
                            Err(ApiError::Unauthorized(_)) => Event::TokenExpired,
                            Err(error) => Event::FriendsFailed(error.message()),
                        };
                        let _ = tx.send(event);
                    });
                }
                Effect::ReviewFriendRequest { request, decision } => {
                    let api = api.clone();
                    let tx = tx.clone();
                    tokio::spawn(async move {
                        let request_id = request.id;
                        let event = match api.review_friend_request(request.id, decision).await {
                            Ok(outcome) => Event::FriendRequestReviewed {
                                request_id,
                                decision,
                                outcome,
                            },
                            Err(ApiError::Unauthorized(_)) => Event::TokenExpired,
                            Err(error) => Event::FriendRequestReviewFailed(error.message()),
                        };
                        let _ = tx.send(event);
                    });
                }
                Effect::Quit => return Ok(()),
            }
        }

        if machine.state().quit {
            return Ok(());
        }
    }
}

fn sync_token(api: &mut ApiClient, state: &state::State) {
    if api.token() != state.token() {
        api.set_token(state.token().map(str::to_string));
    }
}

fn spawn_ticker(tx: UnboundedSender<Event>) {
    tokio::spawn(async move {
        let start = Instant::now() + TICK;
        let mut interval = time::interval_at(start, TICK);
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if tx.send(Event::Tick(TICK)).is_err() {
                break;
            }
        }
    });
}

fn spawn_input(tx: UnboundedSender<Event>) {
    // crossterm's read is blocking, so it gets a dedicated thread. The process
    // exits with the runtime; the thread is detached on purpose.
    std::thread::spawn(move || {
        loop {
            match event::read() {
                Ok(TerminalEvent::Key(key)) => {
                    if key.kind != KeyEventKind::Press {
                        continue;
                    }
                    if let Some(key) = map_key(key)
                        && tx.send(Event::Key(key)).is_err()
                    {
                        break;
                    }
                }
                Ok(TerminalEvent::Resize(width, height)) => {
                    if tx.send(Event::Resize { width, height }).is_err() {
                        break;
                    }
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
    });
}

fn map_key(key: KeyEvent) -> Option<Key> {
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        return match key.code {
            KeyCode::Char('c') => Some(Key::Quit),
            KeyCode::Char('r') => Some(Key::ToggleMode),
            KeyCode::Char('f') => Some(Key::Friends),
            _ => None,
        };
    }
    match key.code {
        KeyCode::Char(c) => Some(Key::Char(c)),
        KeyCode::Backspace => Some(Key::Backspace),
        KeyCode::Tab => Some(Key::Tab),
        KeyCode::Enter => Some(Key::Enter),
        KeyCode::Up => Some(Key::Up),
        KeyCode::Down => Some(Key::Down),
        KeyCode::PageUp => Some(Key::PageUp),
        KeyCode::PageDown => Some(Key::PageDown),
        KeyCode::Esc => Some(Key::Esc),
        KeyCode::F(2) => Some(Key::ToggleMode),
        _ => None,
    }
}
