//! [`Machine`] methods for the open command concern.

use super::*;

impl Machine {
    /// Keys while several `/open` matches are on screen.
    ///
    /// Esc cancels the chooser instead of quitting: the user asked for a
    /// conversation and changed their mind, which is not a request to exit. The
    /// global Ctrl-C (`Key::Quit`) still quits from here.
    pub(super) fn handle_open_picker_key(&mut self, key: Key) -> Vec<Effect> {
        match key {
            Key::Quit => self.quit(),
            Key::Esc => {
                self.state.open = OpenState::default();
                self.state.status = None;
                Vec::new()
            }
            Key::Up => {
                self.state.open.selected = self.state.open.selected.saturating_sub(1);
                Vec::new()
            }
            Key::Down => {
                if self.state.open.selected + 1 < self.state.open.choices.len() {
                    self.state.open.selected += 1;
                }
                Vec::new()
            }
            Key::Enter => {
                let Some(user) = self
                    .state
                    .open
                    .choices
                    .get(self.state.open.selected)
                    .cloned()
                else {
                    return Vec::new();
                };
                self.open_peer(user)
            }
            // Printable keys are not a second search box; the lookups are
            // explicit commands. Ignore them so a stray keystroke cannot pick
            // somebody by accident.
            _ => Vec::new(),
        }
    }

    /// The `/open` fuzzy lookup answered. The reply is only trusted when it is
    /// the answer to the lookup currently in flight.
    pub(super) fn open_users_found(
        &mut self,
        query: String,
        mut users: Vec<FoundUser>,
    ) -> Vec<Effect> {
        if !self.on_conversations() {
            return Vec::new();
        }
        if !self.state.open.searching || query != self.state.open.query {
            // A newer lookup superseded this one, or the flow was cancelled.
            return Vec::new();
        }
        self.state.open.searching = false;
        // The endpoint is a LIKE over every user; opening a conversation with
        // oneself is never useful and the server would refuse the send.
        if let Some(me) = self.state.user_uid() {
            users.retain(|user| user.id != me);
        }
        match users.len() {
            0 => {
                self.state.status = Some(format!("找不到匹配“{query}”的用户"));
                Vec::new()
            }
            // One unambiguous match opens straight away.
            1 => self.open_peer(users.remove(0)),
            // Several matches: never silently take the first. Put them on screen
            // and let the user choose.
            _ => {
                self.state.open.choices = users;
                self.state.open.selected = 0;
                self.state.status = None;
                Vec::new()
            }
        }
    }

    pub(super) fn open_lookup_failed(&mut self, query: String, message: String) -> Vec<Effect> {
        if !self.on_conversations() {
            return Vec::new();
        }
        if !self.state.open.searching || query != self.state.open.query {
            return Vec::new();
        }
        self.state.open.searching = false;
        self.state.status = Some(format!("搜索用户失败：{message}"));
        Vec::new()
    }

    /// Open the conversation with a user the list may not know. Sending the
    /// first message is what makes the server record it; there is deliberately
    /// no "seed the conversation" request here.
    pub(super) fn open_peer(&mut self, user: FoundUser) -> Vec<Effect> {
        let peer = user.id;
        self.state.open = OpenState::default();
        self.state.input.clear();
        self.state.status = None;
        self.state.active = Some(ConversationId::User(peer));
        self.state.messages.clear();
        self.state.messages_loading = true;
        self.state.message_scroll_up = 0;
        self.state.open_peer_name = Some(user.name);
        // If the peer happened to have a row already, move the highlight to it.
        let mut effects = Vec::new();
        if let Some(index) = self
            .state
            .conversations
            .iter()
            .position(|row| row.id == ConversationId::User(peer))
        {
            self.state.selected = index;
            // The row knows a position even before the history arrives; opening
            // through `/open` reads the conversation exactly like a list
            // switch does. A peer with no row has nothing to report yet.
            let mid = self.state.conversations[index].mid;
            effects = self.report_read(ReadPosition {
                conversation: ConversationId::User(peer),
                mid,
            });
        }
        effects.push(Effect::LoadMessages { peer });
        effects
    }

    /// Start the fuzzy lookup. The query is remembered so a reply that lands
    /// after a newer lookup can be recognised and dropped.
    pub(super) fn begin_open(&mut self, query: String) -> Vec<Effect> {
        if query.is_empty() {
            self.state.status = Some("用法：/open <名字>".to_string());
            return Vec::new();
        }
        self.state.open.choices.clear();
        self.state.open.selected = 0;
        self.state.open.query = query.clone();
        self.state.open.searching = true;
        self.state.input.clear();
        self.state.status = None;
        vec![Effect::LookupOpen { query }]
    }
}
