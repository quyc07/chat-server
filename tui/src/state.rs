//! Pure session state machine.
//!
//! This module never performs I/O and never draws. Feed it [`Event`]s derived
//! from keyboard input, HTTP results, ticks, and resizes; it returns the state to
//! render plus the [`Effect`]s the wiring layer must execute. That split is what
//! makes login transitions, list ordering, unread markers, and re-login timing
//! testable without a terminal or a server.

use std::fmt;
use std::time::Duration;

use crate::model::{
    Conversation, ConversationId, FoundUser, Friend, FriendDecision, FriendRequest,
    FriendRequestStatus, Message, MessageView, ReviewOutcome, SendOutcome, StreamChat, Unread,
};

/// The server caps `PageReq.limit` at 50; the client asks for as much as allowed.
pub const PAGE_LIMIT: u64 = 50;

/// How long to wait after the latest incoming message before re-pulling the
/// conversation list. The list's order and unread counts are server logic; the
/// client only refreshes them, never recomputes them, and this delay keeps a
/// burst of messages from turning into a burst of requests.
pub const REFRESH_DEBOUNCE: Duration = Duration::from_millis(1500);

/// How many wrapped lines one PageUp/PageDown moves the message pane.
pub const MESSAGE_SCROLL_LINES: u16 = 10;

/// The JWT lives for one hour on the server. Replace it well before expiry,
/// because `/token/renew` cannot rescue an expired token.
pub const RELOGIN_AFTER: Duration = Duration::from_secs(50 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Login,
    Conversations,
    Friends,
}

/// The status line's wording while the push link is being redialed.
pub const CONNECTION_LOST_STATUS: &str = "连接断开，正在重连";

/// Whether the push connection is delivering bytes.
///
/// Silence is otherwise invisible on screen: the pane simply stops updating, so
/// a down link owns the status line until it is back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Connection {
    /// No session, or no connection attempted yet. A first successful connect
    /// is not a reconnect and must not trigger a catch-up.
    #[default]
    Idle,
    /// The last connection opened and is being watched for bytes.
    Live,
    /// The connection died; a redial is already scheduled.
    Down,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LoginMode {
    #[default]
    Login,
    Register,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Focus {
    #[default]
    Name,
    Password,
}

#[derive(Debug, Clone, Default)]
pub struct LoginForm {
    pub mode: LoginMode,
    pub name: String,
    pub password: String,
    pub focus: Focus,
    /// A submit is on the wire; ignore another Enter until it settles.
    pub busy: bool,
}

/// A high-level key. Keeping crossterm out of the state machine keeps the seam
/// pure and makes tests read like user actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Backspace,
    Tab,
    Enter,
    Up,
    Down,
    /// Switch between login and register (Ctrl-R / F2).
    ToggleMode,
    Esc,
    /// Ctrl-C, always quits.
    Quit,
    /// Scroll the message pane up/down by a page.
    PageUp,
    PageDown,
    /// Open the friends screen from the conversation list (Ctrl-F).
    Friends,
}

/// How an outcome should read on the status line.
///
/// The server reports "already a friend", "request already waiting", and even a
/// forbidden review as meaningful text rather than ordinary failures. Those are
/// [`Notice::Info`]: the status line must not paint them as errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    Info(String),
    Error(String),
}

/// Which friends section owns the keyboard. `Tab` cycles in this order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FriendsFocus {
    #[default]
    Search,
    Results,
    Requests,
    Friends,
}

impl FriendsFocus {
    fn next(self) -> Self {
        match self {
            FriendsFocus::Search => FriendsFocus::Results,
            FriendsFocus::Results => FriendsFocus::Requests,
            FriendsFocus::Requests => FriendsFocus::Friends,
            FriendsFocus::Friends => FriendsFocus::Search,
        }
    }
}

/// The friends screen: one query box and three lists, all decided here.
#[derive(Debug, Clone, Default)]
pub struct FriendsState {
    pub focus: FriendsFocus,
    /// The submitted search text, kept trimmed so a stale reply can be dropped.
    pub query: String,
    pub results: Vec<FoundUser>,
    pub selected_result: usize,
    pub searching: bool,
    /// Only requests whose status is still waiting; approved/rejected ones are
    /// history the UI does not need.
    pub requests: Vec<FriendRequest>,
    pub selected_request: usize,
    pub friends: Vec<Friend>,
    pub selected_friend: usize,
    pub loading_requests: bool,
    pub loading_friends: bool,
    pub notice: Option<Notice>,
}

/// Something that happened, translated from the outside world.
#[derive(Debug, Clone)]
pub enum Event {
    Key(Key),
    LoginSucceeded {
        token: String,
    },
    LoginFailed(String),
    RegisterSucceeded,
    RegisterFailed(String),
    ConversationsLoaded {
        page: u64,
        items: Vec<Conversation>,
    },
    ConversationsFailed(String),
    ReloginSucceeded {
        token: String,
    },
    ReloginFailed(String),
    /// An authenticated request was rejected with 401: the token is gone.
    TokenExpired,
    /// Elapsed wall-clock time since the previous tick.
    Tick(Duration),
    Resize {
        width: u16,
        height: u16,
    },
    /// History for one conversation arrived. `conversation` guards against a
    /// response that lands after the user already opened another one.
    MessagesLoaded {
        conversation: ConversationId,
        items: Vec<Message>,
    },
    MessagesFailed {
        conversation: ConversationId,
        message: String,
    },
    /// One optimistic send was accepted: the server assigned `mid`. `time` is
    /// the client's wall clock when the reply landed; the server's own timestamp
    /// arrives with the next history load or push.
    MessageSent {
        conversation: ConversationId,
        local_id: u64,
        mid: i64,
        time: chrono::NaiveDateTime,
    },
    /// One optimistic send was rejected. `message` is already human-readable.
    MessageSendFailed {
        conversation: ConversationId,
        local_id: u64,
        message: String,
    },
    UsersFound {
        query: String,
        users: Vec<FoundUser>,
    },
    UserSearchFailed(String),
    FriendRequestSent {
        target: String,
        outcome: SendOutcome,
    },
    FriendRequestSendFailed {
        target: String,
        message: String,
    },
    FriendRequestsLoaded(Vec<FriendRequest>),
    FriendRequestsFailed(String),
    FriendsLoaded(Vec<Friend>),
    FriendsFailed(String),
    FriendRequestReviewed {
        request_id: i32,
        decision: FriendDecision,
        outcome: ReviewOutcome,
    },
    FriendRequestReviewFailed(String),
    /// One chat message arrived on the event stream. The server has already
    /// filtered it to messages addressed to or sent by this user.
    Chat(StreamChat),
    /// `GET /user/detail/{uid}` answered for a peer whose conversation was not
    /// in the list yet.
    PeerLoaded {
        uid: i32,
        name: String,
    },
    /// The debounced conversation-list refresh finished. The rows are the
    /// server's authority on order, previews, and unread counts.
    ConversationsRefreshed {
        items: Vec<Conversation>,
    },
    /// A push connection is open and delivering bytes. The first one after
    /// login is not a reconnect; any one after a [`Event::LinkDown`] is.
    LinkUp,
    /// The push connection died: a clean end of response (the server's silent
    /// death) or byte silence (a network drop). A redial is already scheduled.
    LinkDown,
    /// The catch-up history request after a reconnect answered. Merged into the
    /// pane by `mid` instead of replacing it, because the stream may already
    /// have delivered newer messages while the request was in flight.
    CatchUpLoaded {
        conversation: ConversationId,
        items: Vec<Message>,
    },
    /// The catch-up history request failed; the open pane keeps what it has.
    CatchUpFailed {
        conversation: ConversationId,
        message: String,
    },
}

/// Work the wiring layer must perform. The state machine has already committed
/// to the state change; effects are the I/O that follows from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    Login {
        name: String,
        password: String,
    },
    Register {
        name: String,
        password: String,
    },
    LoadHistory {
        page: u64,
    },
    Relogin {
        name: String,
        password: String,
    },
    /// Best-effort server-side logout. The local token is already cleared.
    Logout {
        token: String,
    },
    /// Fetch the direct-message history for one peer (`GET /user/{peer}/history`).
    LoadMessages {
        peer: i32,
    },
    /// `PUT /user/{peer}/send`. `local_id` correlates the optimistic row with the
    /// [`Event::MessageSent`] / [`Event::MessageSendFailed`] that follows.
    SendMessage {
        peer: i32,
        local_id: u64,
        text: String,
    },
    FindUsers {
        query: String,
    },
    SendFriendRequest {
        target: FoundUser,
    },
    LoadFriendRequests,
    LoadFriends,
    ReviewFriendRequest {
        request: FriendRequest,
        decision: FriendDecision,
    },
    /// Re-pull the first page of the conversation list after stream activity.
    RefreshConversations,
    /// Look up a peer's display name for a conversation the list does not know.
    LoadPeer {
        uid: i32,
    },
    /// Re-fetch the open conversation's history after a reconnect. The answer
    /// merges into the pane by message id instead of replacing it.
    CatchUpMessages {
        peer: i32,
    },
    Quit,
}

/// Whether the current window can show the full layout, a compact one, or only a
/// one-line degradation. Every layout decision lives here, not in rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutMode {
    Full,
    Compact,
    Minimal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Viewport {
    pub width: u16,
    pub height: u16,
}

impl Default for Viewport {
    fn default() -> Self {
        // A conventional terminal size until the real one is reported.
        Self {
            width: 80,
            height: 24,
        }
    }
}

/// Credentials held only in process memory. The password is retained so the
/// 50-minute re-login can replace the token without asking the user again.
pub struct Session {
    token: String,
    /// The caller's own id, read out of the JWT payload. `None` only when the
    /// token is not a decodable JWT, which the server never sends.
    uid: Option<i32>,
    name: String,
    password: String,
}

impl Session {
    pub fn token(&self) -> &str {
        &self.token
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn uid(&self) -> Option<i32> {
        self.uid
    }

    fn password(&self) -> &str {
        &self.password
    }
}

impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Never let a stray debug print leak the password.
        f.debug_struct("Session")
            .field("token", &"***")
            .field("name", &self.name)
            .field("password", &"***")
            .finish()
    }
}

/// Delivery state of one optimistically appended message.
///
/// Sending and Failed rows have no server id yet (or never will); Confirmed rows
/// carry the id the server assigned, which is what makes the row the real
/// message rather than a duplicate of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Delivery {
    Sending,
    Confirmed {
        mid: i64,
        time: chrono::NaiveDateTime,
    },
    Failed {
        message: String,
    },
}

/// A message the user sent that is not part of the server's history yet.
///
/// Pending rows live next to [`State::messages`] instead of inside it so the
/// server's history stays exactly what the server said, while the optimistic row
/// can still be reconciled (or marked failed) without being duplicated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingMessage {
    /// Correlates the row with its HTTP result; unique within the process.
    pub local_id: u64,
    pub conversation: ConversationId,
    pub body: String,
    pub delivery: Delivery,
}

impl PendingMessage {
    /// How the row reads in the message pane. The label is the state's decision,
    /// not the render layer's.
    pub fn view(&self, me: &str, now: chrono::NaiveDateTime) -> MessageView {
        let (time_label, failed) = match &self.delivery {
            Delivery::Sending => ("发送中…".to_string(), false),
            Delivery::Confirmed { time, .. } => (crate::model::time_label(*time, now), false),
            Delivery::Failed { .. } => ("发送失败".to_string(), true),
        };
        MessageView {
            sender: me.to_string(),
            mine: true,
            time_label,
            body: self.body.clone(),
            failed,
        }
    }
}

#[derive(Debug)]
pub struct State {
    pub screen: Screen,
    pub form: LoginForm,
    /// Status-line text: server errors, connection failures, or a transient hint.
    pub status: Option<String>,
    pub conversations: Vec<Conversation>,
    pub selected: usize,
    pub loading: bool,
    pub has_more: bool,
    pub friends: FriendsState,
    pub viewport: Viewport,
    pub quit: bool,
    /// The conversation whose history is in the right-hand pane.
    pub active: Option<ConversationId>,
    /// History of [`State::active`], oldest first. Replaced wholesale on switch.
    pub messages: Vec<Message>,
    pub messages_loading: bool,
    /// Lines scrolled up from the bottom of the message pane. Zero follows the
    /// newest message; the render layer clamps it to the content height.
    pub message_scroll_up: u16,
    /// The single-line message box. Always the target of typed characters on the
    /// conversations screen.
    pub input: String,
    /// Optimistic sends for every conversation, in the order they were typed.
    /// Only the active conversation's rows are shown.
    pub pending: Vec<PendingMessage>,
    /// Whether the push connection is delivering bytes.
    pub connection: Connection,
    session: Option<Session>,
    next_page: u64,
    /// Monotonic id for [`PendingMessage::local_id`].
    next_local_id: u64,
}

impl State {
    fn new() -> Self {
        Self {
            screen: Screen::Login,
            form: LoginForm::default(),
            status: None,
            conversations: Vec::new(),
            selected: 0,
            loading: false,
            has_more: false,
            friends: FriendsState::default(),
            viewport: Viewport::default(),
            quit: false,
            active: None,
            messages: Vec::new(),
            messages_loading: false,
            message_scroll_up: 0,
            input: String::new(),
            pending: Vec::new(),
            connection: Connection::Idle,
            session: None,
            next_page: 1,
            next_local_id: 0,
        }
    }

    pub fn token(&self) -> Option<&str> {
        self.session.as_ref().map(Session::token)
    }

    pub fn user_name(&self) -> Option<&str> {
        self.session.as_ref().map(Session::name)
    }

    /// The logged-in user's own id. This, not the name, decides authorship.
    pub fn user_uid(&self) -> Option<i32> {
        self.session.as_ref().and_then(Session::uid)
    }

    pub fn is_logged_in(&self) -> bool {
        self.session.is_some()
    }

    pub fn selected_conversation(&self) -> Option<&Conversation> {
        self.conversations.get(self.selected)
    }

    /// The conversation open in the right-hand pane, if it is still in the list.
    pub fn active_conversation(&self) -> Option<&Conversation> {
        let active = self.active?;
        self.conversations.iter().find(|row| row.id == active)
    }

    /// Presentation of the open conversation's history, classified here so the
    /// render layer makes no decisions of its own.
    ///
    /// Optimistic rows follow the confirmed history because they are the newest
    /// thing the user did; rows belonging to another conversation are skipped.
    pub fn message_views(&self, now: chrono::NaiveDateTime) -> Vec<MessageView> {
        let me = self.user_uid();
        let name = self.user_name().unwrap_or_default();
        let mut views: Vec<MessageView> = self
            .messages
            .iter()
            .map(|message| message.view(me, now))
            .collect();
        if let Some(active) = self.active {
            views.extend(
                self.pending
                    .iter()
                    .filter(|row| row.conversation == active)
                    .map(|row| row.view(name, now)),
            );
        }
        views
    }

    /// How the current window should degrade. Rendering only follows this.
    pub fn layout_mode(&self) -> LayoutMode {
        if self.viewport.width < 40 || self.viewport.height < 8 {
            LayoutMode::Minimal
        } else if self.viewport.width < 70 || self.viewport.height < 16 {
            LayoutMode::Compact
        } else {
            LayoutMode::Full
        }
    }

    /// What the status line must show this frame.
    ///
    /// A down push link outranks ordinary hints: without it, silence on screen
    /// reads as "the other side is quiet" instead of "we are reconnecting".
    /// Ordinary messages are appended, not replaced, so a send failure during
    /// the outage still reaches the user.
    pub fn status_line(&self) -> Option<String> {
        if self.screen == Screen::Conversations && self.connection == Connection::Down {
            return Some(match self.status.as_deref() {
                Some(message) => format!("{CONNECTION_LOST_STATUS} · {message}"),
                None => CONNECTION_LOST_STATUS.to_string(),
            });
        }
        self.status.clone()
    }
}

#[derive(Debug)]
pub struct Machine {
    state: State,
    /// Time since the current token was installed.
    since_login: Duration,
    /// A re-login request is on the wire; never queue a second one.
    relogin_inflight: bool,
    /// Time left before the debounced conversation-list refresh fires.
    refresh_due: Option<Duration>,
}

impl Default for Machine {
    fn default() -> Self {
        Self::new()
    }
}

impl Machine {
    pub fn new() -> Self {
        Self {
            state: State::new(),
            since_login: Duration::ZERO,
            relogin_inflight: false,
            refresh_due: None,
        }
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    pub fn handle(&mut self, event: Event) -> Vec<Effect> {
        match event {
            Event::Key(key) => self.handle_key(key),
            Event::LoginSucceeded { token } => self.login_succeeded(token),
            Event::LoginFailed(message) => {
                if self.state.screen == Screen::Login {
                    self.state.form.busy = false;
                    self.state.status = Some(message);
                }
                Vec::new()
            }
            Event::RegisterSucceeded => self.register_succeeded(),
            Event::RegisterFailed(message) => {
                if self.state.screen == Screen::Login {
                    self.state.form.busy = false;
                    self.state.status = Some(message);
                }
                Vec::new()
            }
            Event::ConversationsLoaded { page, items } => {
                // A response can land after the session ended (token rejected,
                // user quit). Never let it refill the list or clear the message
                // that explains why the user is back on the login screen.
                if self.state.screen != Screen::Conversations || !self.state.is_logged_in() {
                    return Vec::new();
                }
                self.conversations_loaded(page, items)
            }
            Event::ConversationsFailed(message) => {
                if self.state.screen != Screen::Conversations || !self.state.is_logged_in() {
                    return Vec::new();
                }
                self.state.loading = false;
                self.state.status = Some(message);
                Vec::new()
            }
            Event::ReloginSucceeded { token } => self.relogin_succeeded(token),
            Event::ReloginFailed(message) => self.relogin_failed(message),
            Event::TokenExpired => self.token_expired(),
            Event::Tick(elapsed) => self.tick(elapsed),
            Event::Resize { width, height } => {
                self.state.viewport = Viewport { width, height };
                Vec::new()
            }
            Event::MessagesLoaded {
                conversation,
                items,
            } => self.messages_loaded(conversation, items),
            Event::MessagesFailed {
                conversation,
                message,
            } => self.messages_failed(conversation, message),
            Event::MessageSent {
                conversation,
                local_id,
                mid,
                time,
            } => self.message_sent(conversation, local_id, mid, time),
            Event::MessageSendFailed {
                conversation,
                local_id,
                message,
            } => self.message_send_failed(conversation, local_id, message),
            Event::UsersFound { query, users } => self.users_found(query, users),
            Event::UserSearchFailed(message) => self.user_search_failed(message),
            Event::FriendRequestSent { target, outcome } => {
                self.friend_request_sent(target, outcome)
            }
            Event::FriendRequestSendFailed { target, message } => {
                self.friend_request_send_failed(target, message)
            }
            Event::FriendRequestsLoaded(items) => self.friend_requests_loaded(items),
            Event::FriendRequestsFailed(message) => self.friend_requests_failed(message),
            Event::FriendsLoaded(items) => self.friends_loaded(items),
            Event::FriendsFailed(message) => self.friends_failed(message),
            Event::FriendRequestReviewed {
                request_id,
                decision,
                outcome,
            } => self.friend_request_reviewed(request_id, decision, outcome),
            Event::FriendRequestReviewFailed(message) => self.friend_request_review_failed(message),
            Event::Chat(chat) => self.chat_received(chat),
            Event::PeerLoaded { uid, name } => self.peer_loaded(uid, name),
            Event::ConversationsRefreshed { items } => self.conversations_refreshed(items),
            Event::LinkUp => self.link_up(),
            Event::LinkDown => self.link_down(),
            Event::CatchUpLoaded {
                conversation,
                items,
            } => self.catch_up_loaded(conversation, items),
            Event::CatchUpFailed {
                conversation,
                message,
            } => self.catch_up_failed(conversation, message),
        }
    }

    fn handle_key(&mut self, key: Key) -> Vec<Effect> {
        match self.state.screen {
            Screen::Login => self.handle_login_key(key),
            Screen::Conversations => self.handle_conversations_key(key),
            Screen::Friends => self.handle_friends_key(key),
        }
    }

    fn handle_login_key(&mut self, key: Key) -> Vec<Effect> {
        match key {
            Key::Quit | Key::Esc => self.quit(),
            Key::Char(c) if !self.state.form.busy => {
                match self.state.form.focus {
                    Focus::Name => self.state.form.name.push(c),
                    Focus::Password => self.state.form.password.push(c),
                }
                Vec::new()
            }
            Key::Backspace if !self.state.form.busy => {
                match self.state.form.focus {
                    Focus::Name => {
                        self.state.form.name.pop();
                    }
                    Focus::Password => {
                        self.state.form.password.pop();
                    }
                }
                Vec::new()
            }
            Key::Tab | Key::Up | Key::Down => {
                self.state.form.focus = match self.state.form.focus {
                    Focus::Name => Focus::Password,
                    Focus::Password => Focus::Name,
                };
                Vec::new()
            }
            Key::ToggleMode if !self.state.form.busy => {
                self.state.form.mode = match self.state.form.mode {
                    LoginMode::Login => LoginMode::Register,
                    LoginMode::Register => LoginMode::Login,
                };
                self.state.status = None;
                Vec::new()
            }
            Key::Enter => self.submit_login(),
            _ => Vec::new(),
        }
    }

    fn handle_conversations_key(&mut self, key: Key) -> Vec<Effect> {
        match key {
            // `q` is no longer a quit key here: the message box owns every
            // printable character, so a message may start with q.
            Key::Quit | Key::Esc => self.quit(),
            Key::Friends => self.open_friends(),
            Key::Up => {
                self.state.selected = self.state.selected.saturating_sub(1);
                self.open_selected()
            }
            Key::Down => {
                if self.state.selected + 1 < self.state.conversations.len() {
                    self.state.selected += 1;
                    self.open_selected()
                } else if self.state.has_more && !self.state.loading {
                    self.load_next_page()
                } else {
                    Vec::new()
                }
            }
            // Enter is for sending. Ctrl-R (mapped as `ToggleMode`, its login-screen
            // meaning) retries the history load instead.
            Key::Enter => self.send_message(),
            Key::ToggleMode => self.reload_selected(),
            Key::Char(c) => {
                self.state.input.push(c);
                Vec::new()
            }
            Key::Backspace => {
                // `String::pop` removes one Unicode scalar, so CJK edits never
                // split a multi-byte character.
                self.state.input.pop();
                Vec::new()
            }
            Key::PageUp => {
                self.state.message_scroll_up = self
                    .state
                    .message_scroll_up
                    .saturating_add(MESSAGE_SCROLL_LINES);
                Vec::new()
            }
            Key::PageDown => {
                self.state.message_scroll_up = self
                    .state
                    .message_scroll_up
                    .saturating_sub(MESSAGE_SCROLL_LINES);
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    fn open_friends(&mut self) -> Vec<Effect> {
        if self.state.screen != Screen::Conversations || !self.state.is_logged_in() {
            return Vec::new();
        }
        self.state.screen = Screen::Friends;
        self.state.friends.focus = FriendsFocus::Search;
        self.state.friends.notice = None;
        self.state.friends.loading_requests = true;
        self.state.friends.loading_friends = true;
        vec![Effect::LoadFriendRequests, Effect::LoadFriends]
    }

    fn handle_friends_key(&mut self, key: Key) -> Vec<Effect> {
        match key {
            Key::Quit => self.quit(),
            Key::Esc => {
                self.state.screen = Screen::Conversations;
                self.state.friends.notice = None;
                Vec::new()
            }
            Key::Tab => {
                self.state.friends.focus = self.state.friends.focus.next();
                Vec::new()
            }
            Key::Up => {
                self.move_friend_selection(false);
                Vec::new()
            }
            Key::Down => {
                self.move_friend_selection(true);
                Vec::new()
            }
            Key::Enter => self.friends_enter(),
            Key::Backspace if self.state.friends.focus == FriendsFocus::Search => {
                self.state.friends.query.pop();
                Vec::new()
            }
            Key::Char(c) => self.friends_char(c),
            _ => Vec::new(),
        }
    }

    fn friends_char(&mut self, c: char) -> Vec<Effect> {
        match self.state.friends.focus {
            FriendsFocus::Search => {
                self.state.friends.query.push(c);
                Vec::new()
            }
            FriendsFocus::Requests => match c {
                'a' => self.review_selected(FriendDecision::Approve),
                'r' => self.review_selected(FriendDecision::Reject),
                _ => Vec::new(),
            },
            FriendsFocus::Results | FriendsFocus::Friends => Vec::new(),
        }
    }

    fn friends_enter(&mut self) -> Vec<Effect> {
        match self.state.friends.focus {
            FriendsFocus::Search => {
                let query = self.state.friends.query.trim().to_string();
                if query.is_empty() {
                    self.state.friends.notice =
                        Some(Notice::Info("请输入要搜索的用户名".to_string()));
                    return Vec::new();
                }
                self.state.friends.query = query.clone();
                self.state.friends.searching = true;
                self.state.friends.notice = None;
                vec![Effect::FindUsers { query }]
            }
            FriendsFocus::Results => match self
                .state
                .friends
                .results
                .get(self.state.friends.selected_result)
            {
                Some(target) => vec![Effect::SendFriendRequest {
                    target: target.clone(),
                }],
                None => Vec::new(),
            },
            FriendsFocus::Requests => self.review_selected(FriendDecision::Approve),
            FriendsFocus::Friends => Vec::new(),
        }
    }

    fn review_selected(&mut self, decision: FriendDecision) -> Vec<Effect> {
        match self
            .state
            .friends
            .requests
            .get(self.state.friends.selected_request)
        {
            Some(request) => vec![Effect::ReviewFriendRequest {
                request: request.clone(),
                decision,
            }],
            None => Vec::new(),
        }
    }

    fn move_friend_selection(&mut self, down: bool) {
        let friends = &mut self.state.friends;
        let (selected, len) = match friends.focus {
            FriendsFocus::Results => (&mut friends.selected_result, friends.results.len()),
            FriendsFocus::Requests => (&mut friends.selected_request, friends.requests.len()),
            FriendsFocus::Friends => (&mut friends.selected_friend, friends.friends.len()),
            FriendsFocus::Search => return,
        };
        if len == 0 {
            *selected = 0;
        } else if down {
            if *selected + 1 < len {
                *selected += 1;
            }
        } else {
            *selected = selected.saturating_sub(1);
        }
    }

    fn users_found(&mut self, query: String, users: Vec<FoundUser>) -> Vec<Effect> {
        if !self.friends_active() || query != self.state.friends.query {
            return Vec::new();
        }
        self.state.friends.searching = false;
        self.state.friends.results = users;
        self.state.friends.selected_result = 0;
        self.state.friends.notice = if self.state.friends.results.is_empty() {
            Some(Notice::Info(format!("没有找到匹配“{query}”的用户")))
        } else {
            None
        };
        Vec::new()
    }

    fn user_search_failed(&mut self, message: String) -> Vec<Effect> {
        if !self.friends_active() {
            return Vec::new();
        }
        self.state.friends.searching = false;
        self.state.friends.notice = Some(Notice::Error(message));
        Vec::new()
    }

    fn friend_request_sent(&mut self, target: String, outcome: SendOutcome) -> Vec<Effect> {
        if !self.friends_active() {
            return Vec::new();
        }
        let (notice, effects) = match outcome {
            SendOutcome::Sent => (
                Notice::Info(format!("已向 {target} 发送好友请求")),
                Vec::new(),
            ),
            // The server answers 201 for these; they are information, not errors.
            SendOutcome::AlreadyFriend => (
                Notice::Info(format!("{target} 已经是你的好友")),
                vec![Effect::LoadFriends],
            ),
            SendOutcome::AlreadyWaiting => (
                Notice::Info(format!("已向 {target} 发送过好友请求，正在等待对方处理")),
                Vec::new(),
            ),
            SendOutcome::Forbidden(reason) => (
                Notice::Info(readable(
                    reason,
                    format!("暂时无法向 {target} 发送好友请求"),
                )),
                Vec::new(),
            ),
        };
        self.state.friends.notice = Some(notice);
        effects
    }

    fn friend_request_send_failed(&mut self, target: String, message: String) -> Vec<Effect> {
        if !self.friends_active() {
            return Vec::new();
        }
        self.state.friends.notice = Some(Notice::Error(format!(
            "向 {target} 发送好友请求失败：{message}"
        )));
        Vec::new()
    }

    fn friend_requests_loaded(&mut self, items: Vec<FriendRequest>) -> Vec<Effect> {
        if !self.friends_active() {
            return Vec::new();
        }
        // The endpoint returns every inbound request, whatever its status. Only
        // waiting ones are actionable.
        let mut pending: Vec<FriendRequest> = items
            .into_iter()
            .filter(|request| request.status == FriendRequestStatus::Wait)
            .collect();
        pending.sort_by(|a, b| b.create_time.cmp(&a.create_time));
        self.state.friends.requests = pending;
        clamp(
            &mut self.state.friends.selected_request,
            self.state.friends.requests.len(),
        );
        self.state.friends.loading_requests = false;
        Vec::new()
    }

    fn friend_requests_failed(&mut self, message: String) -> Vec<Effect> {
        if !self.friends_active() {
            return Vec::new();
        }
        self.state.friends.loading_requests = false;
        self.state.friends.notice = Some(Notice::Error(message));
        Vec::new()
    }

    fn friends_loaded(&mut self, items: Vec<Friend>) -> Vec<Effect> {
        if !self.friends_active() {
            return Vec::new();
        }
        self.state.friends.friends = items;
        clamp(
            &mut self.state.friends.selected_friend,
            self.state.friends.friends.len(),
        );
        self.state.friends.loading_friends = false;
        Vec::new()
    }

    fn friends_failed(&mut self, message: String) -> Vec<Effect> {
        if !self.friends_active() {
            return Vec::new();
        }
        self.state.friends.loading_friends = false;
        self.state.friends.notice = Some(Notice::Error(message));
        Vec::new()
    }

    fn friend_request_reviewed(
        &mut self,
        request_id: i32,
        decision: FriendDecision,
        outcome: ReviewOutcome,
    ) -> Vec<Effect> {
        if !self.friends_active() {
            return Vec::new();
        }
        match outcome {
            ReviewOutcome::Reviewed => {
                let name = self
                    .state
                    .friends
                    .requests
                    .iter()
                    .find(|request| request.id == request_id)
                    .map(|request| request.request_name.clone())
                    .unwrap_or_else(|| "该用户".to_string());
                self.state
                    .friends
                    .requests
                    .retain(|request| request.id != request_id);
                clamp(
                    &mut self.state.friends.selected_request,
                    self.state.friends.requests.len(),
                );
                let action = match decision {
                    FriendDecision::Approve => "已通过",
                    FriendDecision::Reject => "已拒绝",
                };
                self.state.friends.notice =
                    Some(Notice::Info(format!("{action} {name} 的好友请求")));
                match decision {
                    // Approval creates the relationship server-side; pull the
                    // authoritative list instead of guessing locally.
                    FriendDecision::Approve => vec![Effect::LoadFriends],
                    FriendDecision::Reject => Vec::new(),
                }
            }
            ReviewOutcome::Forbidden(reason) => {
                self.state.friends.notice = Some(Notice::Info(readable(
                    reason,
                    "无权处理该好友请求".to_string(),
                )));
                Vec::new()
            }
        }
    }

    fn friend_request_review_failed(&mut self, message: String) -> Vec<Effect> {
        if !self.friends_active() {
            return Vec::new();
        }
        self.state.friends.notice = Some(Notice::Error(format!("处理好友请求失败：{message}")));
        Vec::new()
    }

    fn friends_active(&self) -> bool {
        self.state.screen == Screen::Friends && self.state.is_logged_in()
    }

    fn submit_login(&mut self) -> Vec<Effect> {
        if self.state.form.busy {
            return Vec::new();
        }
        if self.state.form.name.trim().is_empty() || self.state.form.password.is_empty() {
            self.state.status = Some("请输入用户名和密码".to_string());
            return Vec::new();
        }
        self.state.form.busy = true;
        self.state.status = None;
        let name = self.state.form.name.clone();
        let password = self.state.form.password.clone();
        match self.state.form.mode {
            LoginMode::Login => vec![Effect::Login { name, password }],
            LoginMode::Register => vec![Effect::Register { name, password }],
        }
    }

    fn login_succeeded(&mut self, token: String) -> Vec<Effect> {
        if self.state.screen != Screen::Login {
            return Vec::new();
        }
        let name = self.state.form.name.clone();
        let password = std::mem::take(&mut self.state.form.password);
        // The login response has no uid; the JWT payload does. Read it once per
        // token, here, so the rest of the state machine never parses a JWT.
        let uid = crate::model::uid_from_token(&token).ok();
        self.state.session = Some(Session {
            token,
            uid,
            name,
            password,
        });
        self.state.screen = Screen::Conversations;
        self.state.form.busy = false;
        self.state.status = None;
        self.state.conversations.clear();
        self.state.selected = 0;
        self.clear_open_conversation();
        self.state.pending.clear();
        self.state.friends = FriendsState::default();
        self.state.next_page = 2;
        self.state.loading = true;
        self.state.connection = Connection::Idle;
        self.since_login = Duration::ZERO;
        self.relogin_inflight = false;
        self.refresh_due = None;
        vec![Effect::LoadHistory { page: 1 }]
    }

    fn register_succeeded(&mut self) -> Vec<Effect> {
        if self.state.screen != Screen::Login || self.state.form.mode != LoginMode::Register {
            return Vec::new();
        }
        // Registration does not log in on the server, so immediately log in with
        // the credentials still held in memory. From now on the account exists,
        // so the form must retry as a login rather than registering again.
        self.state.form.mode = LoginMode::Login;
        self.state.form.busy = true;
        self.state.status = Some("注册成功，正在登录…".to_string());
        let name = self.state.form.name.clone();
        let password = self.state.form.password.clone();
        vec![Effect::Login { name, password }]
    }

    fn conversations_loaded(&mut self, page: u64, items: Vec<Conversation>) -> Vec<Effect> {
        let received = items.len();
        self.state.loading = false;
        if page <= 1 {
            self.state.conversations = items;
            self.state.selected = 0;
        } else {
            merge_conversations(&mut self.state.conversations, items);
            self.state.selected = self
                .state
                .selected
                .min(self.state.conversations.len().saturating_sub(1));
        }
        sort_conversations(&mut self.state.conversations);
        // Sorting (and merging) can move rows; keep the open conversation under
        // the highlight instead of switching to whatever now sits at its index.
        if let Some(active) = self.state.active
            && let Some(index) = self
                .state
                .conversations
                .iter()
                .position(|row| row.id == active)
        {
            self.state.selected = index;
        }
        self.state.has_more = received as u64 >= PAGE_LIMIT;
        self.state.next_page = page + 1;
        self.state.status = None;
        let active_still_listed = self
            .state
            .active
            .is_some_and(|active| self.state.conversations.iter().any(|row| row.id == active));
        if active_still_listed {
            Vec::new()
        } else {
            // First page after login, or the open conversation vanished: open the
            // highlighted row and ask for its history.
            self.open_selected()
        }
    }

    /// Open the row under the highlight when it differs from the open
    /// conversation. The old history is dropped immediately so the pane never
    /// shows one peer's messages under another peer's name.
    fn open_selected(&mut self) -> Vec<Effect> {
        let Some(conversation) = self.state.conversations.get(self.state.selected).cloned() else {
            self.clear_open_conversation();
            return Vec::new();
        };
        if self.state.active == Some(conversation.id) {
            return Vec::new();
        }
        self.clear_open_conversation();
        self.state.active = Some(conversation.id);
        self.state.status = None;
        match conversation.id {
            ConversationId::User(peer) => {
                self.state.messages_loading = true;
                vec![Effect::LoadMessages { peer }]
            }
            ConversationId::Group(_) => {
                // Group chat is phase two in the spec. A direct-message history
                // request for a group id cannot succeed, so explain instead.
                self.state.status = Some("群聊功能暂未实现".to_string());
                Vec::new()
            }
        }
    }

    /// Re-fetch the highlighted conversation even when it is already open
    /// (Enter retries a failed load).
    fn reload_selected(&mut self) -> Vec<Effect> {
        self.state.active = None;
        self.open_selected()
    }

    fn messages_loaded(
        &mut self,
        conversation: ConversationId,
        items: Vec<Message>,
    ) -> Vec<Effect> {
        if self.state.screen != Screen::Conversations || !self.state.is_logged_in() {
            return Vec::new();
        }
        if self.state.active != Some(conversation) {
            // The user opened another conversation while this was in flight.
            return Vec::new();
        }
        // A confirmed optimistic row that the server's history now contains is no
        // longer optimistic; drop it instead of drawing the same message twice.
        self.state.pending.retain(|row| {
            !(row.conversation == conversation
                && matches!(row.delivery, Delivery::Confirmed { mid, .. }
                    if items.iter().any(|item| item.mid == mid)))
        });
        self.state.messages = items;
        self.state.messages_loading = false;
        self.state.message_scroll_up = 0;
        self.state.status = None;
        Vec::new()
    }

    fn messages_failed(&mut self, conversation: ConversationId, message: String) -> Vec<Effect> {
        if self.state.screen != Screen::Conversations || !self.state.is_logged_in() {
            return Vec::new();
        }
        if self.state.active != Some(conversation) {
            return Vec::new();
        }
        // A non-friend peer is a 404 with the server's own reason; show it in the
        // status line rather than crashing or emptying the list.
        self.state.messages_loading = false;
        self.state.status = Some(format!("无法加载聊天记录：{message}"));
        Vec::new()
    }

    /// A message arrived on the push stream. The server has already filtered it
    /// to messages addressed to or sent by this user, so there is no filter here
    /// — only the dedupe against rows that already exist and the decision where
    /// the message belongs.
    fn chat_received(&mut self, chat: StreamChat) -> Vec<Effect> {
        if !self.state.is_logged_in() {
            return Vec::new();
        }
        let me = self.state.user_uid();
        let conversation = chat.conversation(me);
        let mine = chat.is_mine(me);
        let from_uid = chat.from_uid;
        let mid = chat.mid;
        let preview = crate::model::excerpt(&chat.body);
        let time = chat.time;

        if self.state.active == Some(conversation) {
            // The pane already shows this message when the confirmed history has
            // the mid, or when the send path already promoted the optimistic row
            // to Confirmed with it. Both must win over the stream copy, or the
            // sender sees their own message twice.
            let already_shown = self.state.messages.iter().any(|message| message.mid == mid)
                || self.state.pending.iter().any(|row| {
                    row.conversation == conversation
                        && matches!(
                            row.delivery,
                            Delivery::Confirmed { mid: confirmed, .. } if confirmed == mid
                        )
                });
            if already_shown {
                // The pane is right, but the list summary and the debounced
                // authoritative refresh must still catch up with my own echo.
                if mine {
                    self.update_row_summary(conversation, mid, preview, time);
                    self.realign_selected();
                    self.schedule_refresh();
                }
                return Vec::new();
            }
            // The echo can beat the HTTP send result, in which case the optimistic
            // row still has no mid and cannot be found by one. Consume the oldest
            // matching send instead; the late `MessageSent` then finds no row to
            // confirm, which the send path already treats as reconciled.
            if mine {
                let optimistic = self.state.pending.iter().position(|row| {
                    row.conversation == conversation
                        && matches!(row.delivery, Delivery::Sending)
                        && row.body == chat.body
                });
                if let Some(index) = optimistic {
                    self.state.pending.remove(index);
                }
            }
            let from_name = if mine {
                self.state.user_name().unwrap_or_default().to_string()
            } else {
                self.state
                    .active_conversation()
                    .map(|row| row.title.clone())
                    .unwrap_or_else(|| format!("用户 {from_uid}"))
            };
            let message = chat.into_message(from_name);
            self.state.messages.push(message);
            self.update_row_summary(conversation, mid, preview, time);
            self.realign_selected();
            // The list is the server's to order; refresh it (debounced) instead
            // of trusting the local summary forever.
            self.schedule_refresh();
            return Vec::new();
        }

        let mut effects = Vec::new();
        match self
            .state
            .conversations
            .iter()
            .position(|row| row.id == conversation)
        {
            Some(index) => {
                // Two copies of the same event, or an older message: the row
                // already reflects it.
                if self.state.conversations[index].mid >= mid {
                    return Vec::new();
                }
                let row = &mut self.state.conversations[index];
                if !mine {
                    row.unread = bumped(row.unread);
                }
                row.mid = mid;
                row.preview = preview;
                row.time = time;
            }
            None => {
                // A conversation the list has never shown. Add it immediately so
                // the message is visible, and ask for the peer's real name; a
                // group has no name endpoint in this phase.
                let title = match conversation {
                    ConversationId::User(uid) => {
                        effects.push(Effect::LoadPeer { uid });
                        format!("用户 {uid}")
                    }
                    ConversationId::Group(gid) => format!("群聊 #{gid}"),
                };
                self.state.conversations.push(Conversation {
                    id: conversation,
                    title,
                    mid,
                    preview,
                    time,
                    unread: if mine { Unread::None } else { Unread::Count(1) },
                    last_sender: None,
                });
            }
        }
        sort_conversations(&mut self.state.conversations);
        self.realign_selected();
        self.schedule_refresh();
        effects
    }

    /// The peer lookup for a brand-new conversation answered.
    fn peer_loaded(&mut self, uid: i32, name: String) -> Vec<Effect> {
        if !self.state.is_logged_in() {
            return Vec::new();
        }
        if let Some(row) = self
            .state
            .conversations
            .iter_mut()
            .find(|row| row.id == ConversationId::User(uid))
        {
            row.title = name;
        }
        Vec::new()
    }

    /// The debounced refresh answered. Rows the server returned replace the
    /// local copies outright: order, preview, and unread are its logic. Rows
    /// outside the first page, or seen only on the stream, are kept.
    fn conversations_refreshed(&mut self, items: Vec<Conversation>) -> Vec<Effect> {
        if self.state.screen != Screen::Conversations || !self.state.is_logged_in() {
            return Vec::new();
        }
        for item in items {
            match self
                .state
                .conversations
                .iter()
                .position(|row| row.id == item.id)
            {
                Some(index) => self.state.conversations[index] = item,
                None => self.state.conversations.push(item),
            }
        }
        sort_conversations(&mut self.state.conversations);
        self.realign_selected();
        Vec::new()
    }

    /// Keep the highlighted index on the open conversation after the order
    /// changed, so the list never moves the highlight to a different peer.
    fn realign_selected(&mut self) {
        if let Some(active) = self.state.active
            && let Some(index) = self
                .state
                .conversations
                .iter()
                .position(|row| row.id == active)
        {
            self.state.selected = index;
        }
        clamp(&mut self.state.selected, self.state.conversations.len());
    }

    /// Start (or restart) the debounce window for a list refresh.
    fn schedule_refresh(&mut self) {
        self.refresh_due = Some(REFRESH_DEBOUNCE);
    }

    /// Refresh the last-message summary of a row that is already in the list.
    fn update_row_summary(
        &mut self,
        conversation: ConversationId,
        mid: i64,
        preview: String,
        time: chrono::NaiveDateTime,
    ) {
        if let Some(row) = self
            .state
            .conversations
            .iter_mut()
            .find(|row| row.id == conversation)
        {
            row.mid = mid;
            row.preview = preview;
            row.time = time;
        }
    }

    /// Take the message box's text and send it to the open direct conversation.
    ///
    /// The row is appended before the request leaves, so the flow never waits for
    /// the server. `local_id` is what the HTTP result carries back; without it a
    /// late reply could not find the row it belongs to.
    fn send_message(&mut self) -> Vec<Effect> {
        if self.state.screen != Screen::Conversations || !self.state.is_logged_in() {
            return Vec::new();
        }
        let Some(ConversationId::User(peer)) = self.state.active else {
            // Nothing is open, or it is a group, which phase two owns.
            return Vec::new();
        };
        // Whitespace-only is empty: the server rejects a blank body anyway.
        let text = self.state.input.trim().to_string();
        if text.is_empty() {
            return Vec::new();
        }
        let local_id = self.state.next_local_id;
        self.state.next_local_id += 1;
        self.state.input.clear();
        self.state.status = None;
        self.state.pending.push(PendingMessage {
            local_id,
            conversation: ConversationId::User(peer),
            body: text.clone(),
            delivery: Delivery::Sending,
        });
        vec![Effect::SendMessage {
            peer,
            local_id,
            text,
        }]
    }

    /// The server accepted a send: give the optimistic row the id it now has.
    fn message_sent(
        &mut self,
        conversation: ConversationId,
        local_id: u64,
        mid: i64,
        time: chrono::NaiveDateTime,
    ) -> Vec<Effect> {
        if self.state.screen != Screen::Conversations || !self.state.is_logged_in() {
            return Vec::new();
        }
        let Some(row) = self
            .state
            .pending
            .iter_mut()
            .find(|row| row.local_id == local_id && row.conversation == conversation)
        else {
            // The row was already reconciled (a history reload that included it).
            return Vec::new();
        };
        row.delivery = Delivery::Confirmed { mid, time };
        if self.state.active == Some(conversation) {
            self.state.status = None;
        }
        Vec::new()
    }

    /// The server rejected a send: mark the row, put the text back in the box, and
    /// explain why on the status line. Nothing is lost, so a retry is one Enter.
    fn message_send_failed(
        &mut self,
        conversation: ConversationId,
        local_id: u64,
        message: String,
    ) -> Vec<Effect> {
        if self.state.screen != Screen::Conversations || !self.state.is_logged_in() {
            return Vec::new();
        }
        let Some(row) = self
            .state
            .pending
            .iter_mut()
            .find(|row| row.local_id == local_id && row.conversation == conversation)
        else {
            return Vec::new();
        };
        let body = row.body.clone();
        row.delivery = Delivery::Failed {
            message: message.clone(),
        };
        // Never overwrite text the user started typing after the send went out,
        // and never drop one peer's draft into another peer's box.
        if self.state.active == Some(conversation) && self.state.input.is_empty() {
            self.state.input = body;
        }
        self.state.status = Some(format!("发送失败：{message}"));
        Vec::new()
    }

    /// Drop everything private to the open conversation. Used whenever the
    /// session ends so another account never sees the previous history.
    ///
    /// The draft is dropped too: carrying it across a conversation switch could
    /// send it to the wrong person. Pending rows for the old conversation are
    /// kept (they still have to settle) but not shown while another is open.
    fn clear_open_conversation(&mut self) {
        self.state.active = None;
        self.state.messages.clear();
        self.state.messages_loading = false;
        self.state.message_scroll_up = 0;
        self.state.input.clear();
    }

    fn load_next_page(&mut self) -> Vec<Effect> {
        self.state.loading = true;
        vec![Effect::LoadHistory {
            page: self.state.next_page,
        }]
    }

    fn relogin_succeeded(&mut self, token: String) -> Vec<Effect> {
        let Some(session) = self.state.session.as_mut() else {
            return Vec::new();
        };
        // A new token carries its own claims; re-read the uid once, as at login.
        session.uid = crate::model::uid_from_token(&token).ok();
        session.token = token;
        self.relogin_inflight = false;
        self.since_login = Duration::ZERO;
        self.state.status = None;
        Vec::new()
    }

    fn relogin_failed(&mut self, message: String) -> Vec<Effect> {
        let Some(session) = self.state.session.take() else {
            return Vec::new();
        };
        // Keep the user name; the password must be re-entered.
        self.state.form.name = session.name;
        self.state.form.password.clear();
        self.state.form.busy = false;
        self.state.form.mode = LoginMode::Login;
        self.state.form.focus = Focus::Password;
        self.state.screen = Screen::Login;
        self.state.status = Some(format!("自动续登失败：{message}，请重新登录"));
        self.state.conversations.clear();
        self.state.selected = 0;
        self.clear_open_conversation();
        self.state.pending.clear();
        self.state.loading = false;
        self.state.has_more = false;
        self.state.friends = FriendsState::default();
        self.state.connection = Connection::Idle;
        self.since_login = Duration::ZERO;
        self.relogin_inflight = false;
        self.refresh_due = None;
        Vec::new()
    }

    fn token_expired(&mut self) -> Vec<Effect> {
        let Some(session) = self.state.session.take() else {
            return Vec::new();
        };
        self.state.form.name = session.name;
        self.state.form.password.clear();
        self.state.form.busy = false;
        self.state.form.mode = LoginMode::Login;
        self.state.form.focus = Focus::Password;
        self.state.screen = Screen::Login;
        self.state.status = Some("登录已过期，请重新登录".to_string());
        self.state.conversations.clear();
        self.state.selected = 0;
        self.clear_open_conversation();
        self.state.pending.clear();
        self.state.loading = false;
        self.state.has_more = false;
        self.state.friends = FriendsState::default();
        self.state.connection = Connection::Idle;
        self.since_login = Duration::ZERO;
        self.relogin_inflight = false;
        self.refresh_due = None;
        Vec::new()
    }

    fn tick(&mut self, elapsed: Duration) -> Vec<Effect> {
        // The debounced list refresh is independent of the re-login timer.
        let mut effects = self.tick_refresh(elapsed);
        if !matches!(self.state.screen, Screen::Conversations | Screen::Friends)
            || !self.state.is_logged_in()
            || self.relogin_inflight
        {
            return effects;
        }
        self.since_login += elapsed;
        if self.since_login < RELOGIN_AFTER {
            return effects;
        }
        let Some(session) = self.state.session.as_ref() else {
            return effects;
        };
        let name = session.name.clone();
        let password = session.password().to_string();
        self.relogin_inflight = true;
        effects.push(Effect::Relogin { name, password });
        effects
    }

    /// Count down the debounce window and ask for one refresh when it closes.
    fn tick_refresh(&mut self, elapsed: Duration) -> Vec<Effect> {
        let Some(remaining) = self.refresh_due else {
            return Vec::new();
        };
        if remaining > elapsed {
            self.refresh_due = Some(remaining - elapsed);
            return Vec::new();
        }
        self.refresh_due = None;
        vec![Effect::RefreshConversations]
    }

    fn quit(&mut self) -> Vec<Effect> {
        self.state.quit = true;
        self.refresh_due = None;
        let token = self.state.session.take().map(|session| session.token);
        let mut effects = Vec::new();
        if let Some(token) = token {
            effects.push(Effect::Logout { token });
        }
        effects.push(Effect::Quit);
        effects
    }

    /// A push connection opened. The first one after login is the normal start
    /// of the stream; one that follows a drop is a reconnect and owes the user
    /// whatever was said while the link was down.
    fn link_up(&mut self) -> Vec<Effect> {
        if !self.state.is_logged_in() {
            return Vec::new();
        }
        let reconnected = self.state.connection == Connection::Down;
        self.state.connection = Connection::Live;
        if !reconnected {
            return Vec::new();
        }
        // The pane and the unread badges both went stale while the stream was
        // down; ask the server instead of guessing. The list refresh is what
        // makes unread counts accurate again.
        let mut effects = Vec::new();
        if let Some(ConversationId::User(peer)) = self.state.active {
            effects.push(Effect::CatchUpMessages { peer });
        }
        effects.push(Effect::RefreshConversations);
        effects
    }

    /// The push link died. The wiring has already scheduled a redial; the only
    /// visible job here is telling the user why the screen will not update.
    fn link_down(&mut self) -> Vec<Effect> {
        if !self.state.is_logged_in() {
            return Vec::new();
        }
        self.state.connection = Connection::Down;
        Vec::new()
    }

    /// A post-reconnect history snapshot arrived. Unlike opening a conversation,
    /// this cannot replace the pane: the redialed stream is already live, so the
    /// pane may hold messages newer than the snapshot. Merge by `mid`, which is
    /// globally unique, and let the server's copy win a collision.
    fn catch_up_loaded(
        &mut self,
        conversation: ConversationId,
        items: Vec<Message>,
    ) -> Vec<Effect> {
        if self.state.screen != Screen::Conversations || !self.state.is_logged_in() {
            return Vec::new();
        }
        if self.state.active != Some(conversation) {
            // The user opened another conversation while this was in flight.
            return Vec::new();
        }
        // An optimistic row the refetched history already contains is confirmed
        // history now; dropping it is what keeps it from being drawn twice.
        self.state.pending.retain(|row| {
            !(row.conversation == conversation
                && matches!(row.delivery, Delivery::Confirmed { mid, .. }
                    if items.iter().any(|item| item.mid == mid)))
        });
        merge_history(&mut self.state.messages, items);
        Vec::new()
    }

    /// The catch-up request itself failed (the redial worked). Keep the pane and
    /// say so on the status line; the next reconnect retries.
    fn catch_up_failed(&mut self, conversation: ConversationId, message: String) -> Vec<Effect> {
        if self.state.screen != Screen::Conversations || !self.state.is_logged_in() {
            return Vec::new();
        }
        if self.state.active != Some(conversation) {
            return Vec::new();
        }
        self.state.status = Some(format!("补拉聊天记录失败：{message}"));
        Vec::new()
    }
}

/// Keep a list selection inside its list after the data changes.
fn clamp(selected: &mut usize, len: usize) {
    if len == 0 {
        *selected = 0;
    } else if *selected >= len {
        *selected = len - 1;
    }
}

/// One more unread message. "Never read" stays "never read": its badge is a
/// shape, not a count, so the number behind it does not matter.
fn bumped(unread: Unread) -> Unread {
    match unread {
        Unread::All => Unread::All,
        Unread::None => Unread::Count(1),
        Unread::Count(count) => Unread::Count(count.saturating_add(1)),
    }
}

/// A server-supplied reason, or a fallback when it sent none.
fn readable(reason: String, fallback: String) -> String {
    if reason.trim().is_empty() {
        fallback
    } else {
        reason
    }
}

fn sort_conversations(conversations: &mut [Conversation]) {
    // Recency first (the spec's promise), message id as the stable tie-break.
    conversations.sort_by(|a, b| b.time.cmp(&a.time).then_with(|| b.mid.cmp(&a.mid)));
}

fn merge_conversations(existing: &mut Vec<Conversation>, incoming: Vec<Conversation>) {
    for item in incoming {
        match existing.iter().position(|row| row.id == item.id) {
            Some(index) => {
                // Keep the fresher copy of a conversation seen on two pages.
                if item.mid >= existing[index].mid {
                    existing[index] = item;
                }
            }
            None => existing.push(item),
        }
    }
}

/// Fold a freshly fetched history into the messages already on screen.
///
/// Used only for the post-reconnect catch-up: the redialed stream is live while
/// the request is in flight, so the pane can already hold messages newer than
/// the snapshot. Replacing would lose them; appending would duplicate the ones
/// the snapshot also contains. `mid` is a single global sequence, so it is the
/// identity to dedupe on, and the server's copy wins a collision because it
/// carries the server's timestamp and sender name.
fn merge_history(existing: &mut Vec<Message>, fetched: Vec<Message>) {
    let known: std::collections::HashSet<i64> = fetched.iter().map(|row| row.mid).collect();
    let mut merged = fetched;
    merged.extend(
        existing
            .drain(..)
            .filter(|message| !known.contains(&message.mid)),
    );
    merged.sort_by_key(|message| message.mid);
    *existing = merged;
}

/// Everything the render layer needs to draw a row, decided here so it can be
/// asserted without a terminal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowView {
    pub id: ConversationId,
    pub title: String,
    pub preview: String,
    pub time_label: String,
    pub unread_badge: Option<String>,
}

impl Conversation {
    pub fn row_view(&self, now: chrono::NaiveDateTime) -> RowView {
        RowView {
            id: self.id,
            title: self.title.clone(),
            preview: self.preview.clone(),
            time_label: crate::model::time_label(self.time, now),
            unread_badge: self.unread.badge(),
        }
    }
}
