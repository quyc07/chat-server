//! [`Machine`] methods for the friends concern.

use super::*;

impl Machine {
    pub(super) fn open_friends(&mut self) -> Vec<Effect> {
        if !self.on_conversations() {
            return Vec::new();
        }
        self.state.screen = Screen::Friends;
        self.state.friends.focus = FriendsFocus::Search;
        self.state.friends.notice = None;
        self.state.friends.loading_requests = true;
        self.state.friends.loading_friends = true;
        vec![Effect::LoadFriendRequests, Effect::LoadFriends]
    }

    pub(super) fn handle_friends_key(&mut self, key: Key) -> Vec<Effect> {
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

    pub(super) fn friends_char(&mut self, c: char) -> Vec<Effect> {
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

    pub(super) fn friends_enter(&mut self) -> Vec<Effect> {
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

    pub(super) fn review_selected(&mut self, decision: FriendDecision) -> Vec<Effect> {
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

    pub(super) fn move_friend_selection(&mut self, down: bool) {
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

    pub(super) fn users_found(&mut self, query: String, users: Vec<FoundUser>) -> Vec<Effect> {
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

    pub(super) fn user_search_failed(&mut self, message: String) -> Vec<Effect> {
        if !self.friends_active() {
            return Vec::new();
        }
        self.state.friends.searching = false;
        self.state.friends.notice = Some(Notice::Error(message));
        Vec::new()
    }

    pub(super) fn friend_request_sent(
        &mut self,
        target: String,
        outcome: SendOutcome,
    ) -> Vec<Effect> {
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

    pub(super) fn friend_request_send_failed(
        &mut self,
        target: String,
        message: String,
    ) -> Vec<Effect> {
        if !self.friends_active() {
            return Vec::new();
        }
        self.state.friends.notice = Some(Notice::Error(format!(
            "向 {target} 发送好友请求失败：{message}"
        )));
        Vec::new()
    }

    pub(super) fn friend_requests_loaded(&mut self, items: Vec<FriendRequest>) -> Vec<Effect> {
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

    pub(super) fn friend_requests_failed(&mut self, message: String) -> Vec<Effect> {
        if !self.friends_active() {
            return Vec::new();
        }
        self.state.friends.loading_requests = false;
        self.state.friends.notice = Some(Notice::Error(message));
        Vec::new()
    }

    pub(super) fn friends_loaded(&mut self, items: Vec<Friend>) -> Vec<Effect> {
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

    pub(super) fn friends_failed(&mut self, message: String) -> Vec<Effect> {
        if !self.friends_active() {
            return Vec::new();
        }
        self.state.friends.loading_friends = false;
        self.state.friends.notice = Some(Notice::Error(message));
        Vec::new()
    }

    pub(super) fn friend_request_reviewed(
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

    pub(super) fn friend_request_review_failed(&mut self, message: String) -> Vec<Effect> {
        if !self.friends_active() {
            return Vec::new();
        }
        self.state.friends.notice = Some(Notice::Error(format!("处理好友请求失败：{message}")));
        Vec::new()
    }
}
