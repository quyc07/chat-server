//! [`Machine`] methods for the login concern.

use super::*;

impl Machine {
    pub(super) fn handle_login_key(&mut self, key: Key) -> Vec<Effect> {
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

    pub(super) fn submit_login(&mut self) -> Vec<Effect> {
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

    pub(super) fn login_succeeded(&mut self, token: String) -> Vec<Effect> {
        if self.state.screen != Screen::Login {
            return Vec::new();
        }
        let name = self.state.form.name.clone();
        let password = std::mem::take(&mut self.state.form.password);
        // The login response has no uid; the JWT payload does. Read it once per
        // token, here, so the rest of the state machine never parses a JWT.
        // Without our own id every message's authorship would silently invert,
        // so a token we cannot attribute is a failed login, not a usable session.
        let uid = match crate::model::uid_from_token(&token) {
            Ok(uid) => uid,
            Err(error) => {
                self.state.form.busy = false;
                self.state.status = Some(format!("登录令牌无效，无法确定当前用户：{error}"));
                return Vec::new();
            }
        };
        self.state.session = Some(Session {
            token,
            uid: Some(uid),
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
        // A new session knows nothing about what the previous one reported.
        self.reported.clear();
        vec![Effect::LoadHistory { page: 1 }]
    }

    pub(super) fn register_succeeded(&mut self) -> Vec<Effect> {
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

    pub(super) fn relogin_succeeded(&mut self, token: String) -> Vec<Effect> {
        // A new token carries its own claims; re-read the uid once, as at login.
        // A token we cannot attribute is as unusable as a rejected one: do not
        // keep a session whose authorship would be unknown.
        let uid = match crate::model::uid_from_token(&token) {
            Ok(uid) => uid,
            Err(error) => {
                return self.relogin_failed(format!("无法从新令牌确定当前用户：{error}"));
            }
        };
        let Some(session) = self.state.session.as_mut() else {
            return Vec::new();
        };
        session.uid = Some(uid);
        session.token = token;
        self.relogin_inflight = false;
        self.since_login = Duration::ZERO;
        self.state.status = None;
        Vec::new()
    }

    pub(super) fn relogin_failed(&mut self, message: String) -> Vec<Effect> {
        let Some(session) = self.state.session.take() else {
            return Vec::new();
        };
        self.end_session(session, format!("自动续登失败：{message}，请重新登录"));
        Vec::new()
    }

    pub(super) fn token_expired(&mut self) -> Vec<Effect> {
        let Some(session) = self.state.session.take() else {
            return Vec::new();
        };
        self.end_session(session, "登录已过期，请重新登录".to_string());
        Vec::new()
    }

    /// Return to the login screen after the session ended.
    ///
    /// The name is kept for convenience but the password must be re-entered, and
    /// every piece of state private to the old session is dropped.
    pub(super) fn end_session(&mut self, session: Session, status: String) {
        self.state.form.name = session.name;
        self.state.form.password.clear();
        self.state.form.busy = false;
        self.state.form.mode = LoginMode::Login;
        self.state.form.focus = Focus::Password;
        self.state.screen = Screen::Login;
        self.state.status = Some(status);
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
    }

    pub(super) fn tick(&mut self, elapsed: Duration) -> Vec<Effect> {
        // The debounced list refresh is independent of the re-login timer.
        let mut effects = self.tick_refresh(elapsed);
        if (!self.on_conversations() && !self.friends_active()) || self.relogin_inflight {
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

    pub(super) fn quit(&mut self) -> Vec<Effect> {
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
}
