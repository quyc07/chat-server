//! Pure session state machine.
//!
//! This module never performs I/O and never draws. Feed it [`Event`]s derived
//! from keyboard input, HTTP results, ticks, and resizes; it returns the state to
//! render plus the [`Effect`]s the wiring layer must execute. That split is what
//! makes login transitions, list ordering, unread markers, and re-login timing
//! testable without a terminal or a server.

use std::collections::HashMap;
use std::fmt;
use std::time::Duration;

use crate::model::{
    Conversation, ConversationId, FoundUser, Friend, FriendDecision, FriendRequest,
    FriendRequestStatus, Message, MessageView, ReadPosition, ReviewOutcome, SendOutcome,
    StreamMessage, Unread,
};

mod conversation_list;
mod friends;
mod login;
mod messages;
mod open_command;
mod read_index;
mod reconnect;

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

/// The `/open <name>` cold-start flow: a fuzzy lookup that may need the user to
/// choose among several matches.
///
/// It lives beside the conversation list rather than on its own screen because
/// the command is typed in the same message box: while the chooser is up it owns
/// the keyboard, and cancelling returns to ordinary typing without losing the
/// list, history, or draft.
#[derive(Debug, Clone, Default)]
pub struct OpenState {
    /// The submitted lookup text. A reply for a different query is stale and is
    /// dropped, so a fast retry can never be answered by the previous lookup.
    pub query: String,
    /// A lookup is on the wire.
    pub searching: bool,
    /// Multiple matches awaiting a choice. Non-empty means the chooser owns the
    /// keyboard; an empty list means ordinary typing.
    pub choices: Vec<FoundUser>,
    pub selected: usize,
}

impl OpenState {
    /// True while several users matched and the user must pick one.
    pub fn choosing(&self) -> bool {
        !self.choices.is_empty()
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
    /// The `/open <name>` lookup answered. `query` is the text that produced it;
    /// a reply whose query no longer matches is stale.
    OpenUsersFound {
        query: String,
        users: Vec<FoundUser>,
    },
    /// The `/open <name>` lookup failed (network, decode, server refusal).
    OpenLookupFailed {
        query: String,
        message: String,
    },
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
    /// One message arrived on the event stream. The server has already filtered
    /// it to messages addressed to or sent by this user.
    MessageReceived(StreamMessage),
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
    /// The server accepted a read position. Nothing is recomputed locally: the
    /// unread count is the server's, so only a list refresh follows.
    ReadReported,
    /// The server rejected a read position. `position` is the one that has to be
    /// retried later; the message is already human-readable.
    ReadReportFailed {
        position: ReadPosition,
        message: String,
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
    /// `PUT /ri`: tell the server this conversation is read up to the position's
    /// `mid`. The server derives the unread count from it, so this is the only
    /// way an open conversation becomes read.
    ReportRead(ReadPosition),
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
    /// `GET /user/find/{name}` for `/open`. A separate effect from [`Effect::FindUsers`]
    /// because the friends screen has its own query and state; sharing one result
    /// event would let the two flows answer each other's lookups.
    LookupOpen {
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
/// Sending and Failed rows have no server id yet (or never will). Confirmed rows
/// carry the id the server assigned; a confirmed row for the open conversation
/// is migrated into [`State::messages`] immediately, so a `Confirmed` row in
/// [`State::pending`] only survives for a conversation that was not open when
/// the reply arrived.
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

/// A message the user sent that the pane has not reconciled yet.
///
/// Pending rows live next to [`State::messages`] instead of inside it so the
/// server's history stays exactly what the server said, while an optimistic row
/// can still be reconciled (or marked failed) without being duplicated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingMessage {
    /// Correlates the row with its HTTP result; unique within the process.
    pub local_id: u64,
    pub conversation: ConversationId,
    pub body: String,
    pub delivery: Delivery,
    /// The newest message id in the pane when the send was typed. Message ids
    /// are a global monotonic sequence, so a history snapshot row at or below
    /// this id cannot be this send; only a row above it can.
    pub baseline_mid: i64,
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
    /// The `/open <name>` lookup and chooser.
    pub open: OpenState,
    /// Name of the active peer when the list has no row for them yet. `/open`
    /// can open a conversation the server has not recorded, so the pane needs a
    /// title the list cannot supply.
    pub open_peer_name: Option<String>,
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
            open: OpenState::default(),
            open_peer_name: None,
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

    /// Title for the message pane. A conversation the list knows uses the
    /// server's title; a peer opened with `/open` but never recorded uses the
    /// name the lookup returned.
    pub fn active_title(&self) -> Option<String> {
        match self.active_conversation() {
            Some(conversation) => Some(conversation.title.clone()),
            None => self.open_peer_name.clone(),
        }
    }

    /// Which conversation-list row wears the highlight. The open conversation
    /// wins when it has a row; an open peer the list does not know yet
    /// highlights nothing rather than a stranger's row.
    pub fn highlighted_index(&self) -> Option<usize> {
        if let Some(active) = self.active {
            return self.conversations.iter().position(|row| row.id == active);
        }
        if self.conversations.is_empty() {
            None
        } else {
            Some(self.selected.min(self.conversations.len() - 1))
        }
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
    /// The highest message id already reported read, per conversation. Reporting
    /// is advance-only, so opening, loading history, and every stream message
    /// cannot turn into the same report twice.
    reported: HashMap<ConversationId, i64>,
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
            reported: HashMap::new(),
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
                if !self.on_conversations() {
                    return Vec::new();
                }
                self.conversations_loaded(page, items)
            }
            Event::ConversationsFailed(message) => {
                if !self.on_conversations() {
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
            Event::OpenUsersFound { query, users } => self.open_users_found(query, users),
            Event::OpenLookupFailed { query, message } => self.open_lookup_failed(query, message),
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
            Event::MessageReceived(message) => self.message_received(message),
            Event::PeerLoaded { uid, name } => self.peer_loaded(uid, name),
            Event::ConversationsRefreshed { items } => self.conversations_refreshed(items),
            Event::ReadReported => self.read_reported(),
            Event::ReadReportFailed { position, message } => {
                self.read_report_failed(position, message)
            }
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

    /// True only while the signed-in user is on the conversation-list screen.
    /// Late HTTP replies from a previous session must never repaint it.
    fn on_conversations(&self) -> bool {
        self.state.screen == Screen::Conversations && self.state.is_logged_in()
    }

    fn friends_active(&self) -> bool {
        self.state.screen == Screen::Friends && self.state.is_logged_in()
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
}

/// A command typed into the shared message box.
///
/// Only exact command words are recognised, and only on submit: a message that
/// happens to start with `/` is still a message.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Command {
    /// `/open <name>`; the query may be empty, which the caller reports as a
    /// usage hint rather than sending a lookup.
    Open(String),
}

fn parse_command(input: &str) -> Option<Command> {
    let text = input.trim();
    let rest = text.strip_prefix("/open")?;
    if rest.is_empty() {
        return Some(Command::Open(String::new()));
    }
    // `/openfoo` is an ordinary message; the command word must be followed by
    // whitespace before its argument.
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    Some(Command::Open(rest.trim().to_string()))
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
