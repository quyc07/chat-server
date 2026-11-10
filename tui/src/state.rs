//! Pure session state machine.
//!
//! This module never performs I/O and never draws. Feed it [`Event`]s derived
//! from keyboard input, HTTP results, ticks, and resizes; it returns the state to
//! render plus the [`Effect`]s the wiring layer must execute. That split is what
//! makes login transitions, list ordering, unread markers, and re-login timing
//! testable without a terminal or a server.

use std::fmt;
use std::time::Duration;

use crate::model::{Conversation, ConversationId, Message, MessageView};

/// The server caps `PageReq.limit` at 50; the client asks for as much as allowed.
pub const PAGE_LIMIT: u64 = 50;

/// How many wrapped lines one PageUp/PageDown moves the message pane.
pub const MESSAGE_SCROLL_LINES: u16 = 10;

/// The JWT lives for one hour on the server. Replace it well before expiry,
/// because `/token/renew` cannot rescue an expired token.
pub const RELOGIN_AFTER: Duration = Duration::from_secs(50 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Login,
    Conversations,
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
    session: Option<Session>,
    next_page: u64,
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
            viewport: Viewport::default(),
            quit: false,
            active: None,
            messages: Vec::new(),
            messages_loading: false,
            message_scroll_up: 0,
            session: None,
            next_page: 1,
        }
    }

    pub fn token(&self) -> Option<&str> {
        self.session.as_ref().map(Session::token)
    }

    pub fn user_name(&self) -> Option<&str> {
        self.session.as_ref().map(Session::name)
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
    pub fn message_views(&self, now: chrono::NaiveDateTime) -> Vec<MessageView> {
        let me = self.user_name().unwrap_or_default();
        self.messages
            .iter()
            .map(|message| message.view(me, now))
            .collect()
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
}

#[derive(Debug)]
pub struct Machine {
    state: State,
    /// Time since the current token was installed.
    since_login: Duration,
    /// A re-login request is on the wire; never queue a second one.
    relogin_inflight: bool,
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
        }
    }

    fn handle_key(&mut self, key: Key) -> Vec<Effect> {
        match self.state.screen {
            Screen::Login => self.handle_login_key(key),
            Screen::Conversations => self.handle_conversations_key(key),
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
            Key::Quit | Key::Esc => self.quit(),
            Key::Char('q') => self.quit(),
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
            Key::Enter => self.reload_selected(),
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
        self.state.session = Some(Session {
            token,
            name,
            password,
        });
        self.state.screen = Screen::Conversations;
        self.state.form.busy = false;
        self.state.status = None;
        self.state.conversations.clear();
        self.state.selected = 0;
        self.clear_open_conversation();
        self.state.next_page = 2;
        self.state.loading = true;
        self.since_login = Duration::ZERO;
        self.relogin_inflight = false;
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

    /// Drop everything private to the open conversation. Used whenever the
    /// session ends so another account never sees the previous history.
    fn clear_open_conversation(&mut self) {
        self.state.active = None;
        self.state.messages.clear();
        self.state.messages_loading = false;
        self.state.message_scroll_up = 0;
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
        self.state.loading = false;
        self.state.has_more = false;
        self.since_login = Duration::ZERO;
        self.relogin_inflight = false;
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
        self.state.loading = false;
        self.state.has_more = false;
        self.since_login = Duration::ZERO;
        self.relogin_inflight = false;
        Vec::new()
    }

    fn tick(&mut self, elapsed: Duration) -> Vec<Effect> {
        if self.state.screen != Screen::Conversations
            || !self.state.is_logged_in()
            || self.relogin_inflight
        {
            return Vec::new();
        }
        self.since_login += elapsed;
        if self.since_login < RELOGIN_AFTER {
            return Vec::new();
        }
        let Some(session) = self.state.session.as_ref() else {
            return Vec::new();
        };
        let name = session.name.clone();
        let password = session.password().to_string();
        self.relogin_inflight = true;
        vec![Effect::Relogin { name, password }]
    }

    fn quit(&mut self) -> Vec<Effect> {
        self.state.quit = true;
        let token = self.state.session.take().map(|session| session.token);
        let mut effects = Vec::new();
        if let Some(token) = token {
            effects.push(Effect::Logout { token });
        }
        effects.push(Effect::Quit);
        effects
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
