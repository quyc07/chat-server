//! [`Machine`] methods for the conversation list concern.

use super::*;

impl Machine {
    pub(super) fn handle_conversations_key(&mut self, key: Key) -> Vec<Effect> {
        // While several `/open` matches are on screen the chooser owns the
        // keyboard: an arrow must not move the conversation list, and Enter must
        // not send a half-typed message to whoever was previously open.
        if self.state.open.choosing() {
            return self.handle_open_picker_key(key);
        }
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
            // Enter is for sending, or for a `/open` command typed in the same
            // box. Parsing happens on submit so `/`, `q`, and CJK text remain
            // ordinary characters while typing.
            Key::Enter => self.submit_input(),
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

    pub(super) fn conversations_loaded(
        &mut self,
        page: u64,
        items: Vec<Conversation>,
    ) -> Vec<Effect> {
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
    pub(super) fn open_selected(&mut self) -> Vec<Effect> {
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
                let mut effects = self.report_read(ReadPosition {
                    conversation: conversation.id,
                    mid: conversation.mid,
                });
                effects.push(Effect::LoadMessages { peer });
                effects
            }
            ConversationId::Group(_) => {
                // Group conversations are phase two in the spec. A direct-message history
                // request for a group id cannot succeed, so explain instead.
                self.state.status = Some("群聊功能暂未实现".to_string());
                Vec::new()
            }
        }
    }

    /// Re-fetch the highlighted conversation even when it is already open
    /// (Enter retries a failed load).
    pub(super) fn reload_selected(&mut self) -> Vec<Effect> {
        self.state.active = None;
        self.open_selected()
    }

    /// The debounced refresh answered. Rows the server returned replace the
    /// local copies outright: order, preview, and unread are its logic. Rows
    /// outside the first page, or seen only on the stream, are kept.
    pub(super) fn conversations_refreshed(&mut self, items: Vec<Conversation>) -> Vec<Effect> {
        if !self.on_conversations() {
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
    pub(super) fn realign_selected(&mut self) {
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
    pub(super) fn schedule_refresh(&mut self) {
        self.refresh_due = Some(REFRESH_DEBOUNCE);
    }

    pub(super) fn load_next_page(&mut self) -> Vec<Effect> {
        self.state.loading = true;
        vec![Effect::LoadHistory {
            page: self.state.next_page,
        }]
    }
}
