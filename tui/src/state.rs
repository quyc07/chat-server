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
    FriendRequestStatus, ReviewOutcome, SendOutcome,
};

/// The server caps `PageReq.limit` at 50; the client asks for as much as allowed.
pub const PAGE_LIMIT: u64 = 50;

/// The JWT lives for one hour on the server. Replace it well before expiry,
/// because `/token/renew` cannot rescue an expired token.
pub const RELOGIN_AFTER: Duration = Duration::from_secs(50 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Login,
    Conversations,
    Friends,
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
    Quit,
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
    pub friends: FriendsState,
    pub viewport: Viewport,
    pub quit: bool,
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
            friends: FriendsState::default(),
            viewport: Viewport::default(),
            quit: false,
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
            Key::Quit | Key::Esc => self.quit(),
            Key::Char('q') => self.quit(),
            Key::Friends => self.open_friends(),
            Key::Up => {
                self.state.selected = self.state.selected.saturating_sub(1);
                Vec::new()
            }
            Key::Down => {
                if self.state.selected + 1 < self.state.conversations.len() {
                    self.state.selected += 1;
                    Vec::new()
                } else if self.state.has_more && !self.state.loading {
                    self.load_next_page()
                } else {
                    Vec::new()
                }
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
        self.state.friends = FriendsState::default();
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
        self.state.has_more = received as u64 >= PAGE_LIMIT;
        self.state.next_page = page + 1;
        self.state.status = None;
        Vec::new()
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
        self.state.loading = false;
        self.state.has_more = false;
        self.state.friends = FriendsState::default();
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
        self.state.loading = false;
        self.state.has_more = false;
        self.state.friends = FriendsState::default();
        self.since_login = Duration::ZERO;
        self.relogin_inflight = false;
        Vec::new()
    }

    fn tick(&mut self, elapsed: Duration) -> Vec<Effect> {
        if !matches!(self.state.screen, Screen::Conversations | Screen::Friends)
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

/// Keep a list selection inside its list after the data changes.
fn clamp(selected: &mut usize, len: usize) {
    if len == 0 {
        *selected = 0;
    } else if *selected >= len {
        *selected = len - 1;
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
