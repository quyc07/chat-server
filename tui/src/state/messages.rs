//! [`Machine`] methods for the messages concern.

use super::*;

impl Machine {
    pub(super) fn messages_loaded(
        &mut self,
        conversation: ConversationId,
        items: Vec<Message>,
    ) -> Vec<Effect> {
        if !self.on_conversations() {
            return Vec::new();
        }
        if self.state.active != Some(conversation) {
            // The user opened another conversation while this was in flight.
            return Vec::new();
        }
        // A sent row the server's history now contains is no longer optimistic;
        // drop it instead of drawing the same message twice.
        self.reconcile_pending(conversation, &items);
        self.state.pending.retain(|row| {
            !(row.conversation == conversation
                && matches!(row.delivery, Delivery::Confirmed { mid, .. }
                    if items.iter().any(|item| item.mid == mid)))
        });
        // The snapshot is authoritative up to its newest id. Anything the pane
        // already holds above that arrived after the server answered this
        // request (a send reply or a stream message) and must not be dropped.
        let newest = items.iter().map(|item| item.mid).max();
        let mut messages = items;
        if let Some(newest) = newest {
            messages.extend(
                self.state
                    .messages
                    .drain(..)
                    .filter(|message| message.mid > newest),
            );
            messages.sort_by_key(|message| message.mid);
        }
        self.state.messages = messages;
        self.state.messages_loading = false;
        self.state.message_scroll_up = 0;
        self.state.status = None;
        // History can be newer than the list row that opened it (the two
        // endpoints are read at different times). Whatever the pane now shows
        // is what the user has read.
        self.report_newest_read(conversation)
    }

    pub(super) fn messages_failed(
        &mut self,
        conversation: ConversationId,
        message: String,
    ) -> Vec<Effect> {
        if !self.on_conversations() {
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
    pub(super) fn message_received(&mut self, stream_message: StreamMessage) -> Vec<Effect> {
        if !self.state.is_logged_in() {
            return Vec::new();
        }
        let me = self.state.user_uid();
        let conversation = stream_message.conversation(me);
        let mine = stream_message.is_mine(me);
        let from_uid = stream_message.from_uid;
        let mid = stream_message.mid;
        let preview = crate::model::excerpt(&stream_message.body);
        let time = stream_message.time;

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
            // confirm, which the send path already treats as reconciled. The echo
            // must also be newer than the row's baseline, so a redelivered older
            // message with the same text cannot swallow a fresh send.
            if mine {
                let optimistic = self.state.pending.iter().position(|row| {
                    row.conversation == conversation
                        && matches!(row.delivery, Delivery::Sending)
                        && row.body == stream_message.body
                        && mid > row.baseline_mid
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
            let message = stream_message.into_message(from_name);
            self.insert_message(message);
            self.update_row_summary(conversation, mid, preview, time);
            self.realign_selected();
            // The list is the server's to order; refresh it (debounced) instead
            // of trusting the local summary forever.
            self.schedule_refresh();
            // The pane shows it, so the user has read it. My own echo is
            // skipped: the send endpoint already wrote the read index for it.
            return if mine {
                Vec::new()
            } else {
                self.report_read(ReadPosition { conversation, mid })
            };
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
    pub(super) fn peer_loaded(&mut self, uid: i32, name: String) -> Vec<Effect> {
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

    /// Enter in the shared message box: a recognised `/open <name>` command
    /// starts a conversation, everything else is an ordinary message.
    ///
    /// The box is not given a separate command line: `/`, `q`, and CJK text are
    /// all message characters, so the command is only recognised here, on
    /// submit, and only for the exact command word.
    pub(super) fn submit_input(&mut self) -> Vec<Effect> {
        if !self.on_conversations() {
            return Vec::new();
        }
        match parse_command(&self.state.input) {
            Some(Command::Open(query)) => self.begin_open(query),
            None => self.send_message(),
        }
    }

    /// Take the message box's text and send it to the open direct conversation.
    ///
    /// The row is appended before the request leaves, so the flow never waits for
    /// the server. `local_id` is what the HTTP result carries back; without it a
    /// late reply could not find the row it belongs to.
    pub(super) fn send_message(&mut self) -> Vec<Effect> {
        if !self.on_conversations() {
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
        // Everything already on screen was stored before this send, so the
        // server has to assign a higher mid to it. That is what later lets a
        // history snapshot recognise the sent message without knowing its mid.
        let baseline_mid = self
            .state
            .messages
            .iter()
            .map(|message| message.mid)
            .max()
            .unwrap_or(0);
        self.state.pending.push(PendingMessage {
            local_id,
            conversation: ConversationId::User(peer),
            body: text.clone(),
            delivery: Delivery::Sending,
            baseline_mid,
        });
        vec![Effect::SendMessage {
            peer,
            local_id,
            text,
        }]
    }

    /// The server accepted a send: turn the optimistic row into confirmed
    /// history.
    ///
    /// For the open conversation the row is migrated into [`State::messages`]
    /// and inserted by `mid`, so a message that arrived while the reply was in
    /// flight can no longer sit above it. A row confirmed while another
    /// conversation is open has no pane to move into yet, so it stays pending
    /// (with the server id) until that conversation's history is loaded.
    pub(super) fn message_sent(
        &mut self,
        conversation: ConversationId,
        local_id: u64,
        mid: i64,
        time: chrono::NaiveDateTime,
    ) -> Vec<Effect> {
        if !self.on_conversations() {
            return Vec::new();
        }
        let Some(index) = self
            .state
            .pending
            .iter()
            .position(|row| row.local_id == local_id && row.conversation == conversation)
        else {
            // The row was already reconciled by a stream echo or a history load.
            return Vec::new();
        };
        let mut row = self.state.pending.remove(index);
        row.delivery = Delivery::Confirmed { mid, time };
        if self.state.active == Some(conversation) {
            // The stream echo may already have appended the same id; keep one.
            if !self.state.messages.iter().any(|message| message.mid == mid) {
                let from_uid = self.state.user_uid().unwrap_or_default();
                let from_name = self.state.user_name().unwrap_or_default().to_string();
                let body = std::mem::take(&mut row.body);
                self.insert_message(Message {
                    mid,
                    from_uid,
                    from_name,
                    body,
                    time,
                });
            }
            self.state.status = None;
        } else {
            self.state.pending.push(row);
        }
        // A conversation opened from `/open` has no list row until the server
        // records the send. Ask for the authoritative list so it appears on its
        // own instead of the client inventing a row and a preview.
        if !self
            .state
            .conversations
            .iter()
            .any(|row| row.id == conversation)
        {
            self.schedule_refresh();
        }
        Vec::new()
    }

    /// The server rejected a send: mark the row, put the text back in the box, and
    /// explain why on the status line. Nothing is lost, so a retry is one Enter.
    pub(super) fn message_send_failed(
        &mut self,
        conversation: ConversationId,
        local_id: u64,
        message: String,
    ) -> Vec<Effect> {
        if !self.on_conversations() {
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
    pub(super) fn clear_open_conversation(&mut self) {
        self.state.active = None;
        self.state.messages.clear();
        self.state.messages_loading = false;
        self.state.message_scroll_up = 0;
        self.state.input.clear();
        self.state.open = OpenState::default();
        self.state.open_peer_name = None;
    }

    /// Insert one confirmed message into the pane in `mid` order. Message ids are
    /// a single global sequence, so this is also the render order.
    pub(super) fn insert_message(&mut self, message: Message) {
        let at = self
            .state
            .messages
            .partition_point(|existing| existing.mid < message.mid);
        self.state.messages.insert(at, message);
    }

    /// Consume optimistic rows that a history snapshot has already stored.
    ///
    /// A snapshot can arrive before the send's HTTP reply, in which case the
    /// pending row still has no `mid` and cannot be matched by id. Match on the
    /// text instead, but only against rows newer than the row's baseline: mids
    /// are globally monotonic, so an older message with the same text cannot be
    /// this send. Candidates are claimed one-by-one, so two identical messages
    /// sent back-to-back still map to two rows.
    pub(super) fn reconcile_pending(&mut self, conversation: ConversationId, items: &[Message]) {
        let me = self.state.user_uid();
        let mut claimed: Vec<i64> = Vec::new();
        let mut consumed: Vec<u64> = Vec::new();
        for row in &self.state.pending {
            if row.conversation != conversation || !matches!(row.delivery, Delivery::Sending) {
                continue;
            }
            let candidate = items.iter().find(|item| {
                Some(item.from_uid) == me
                    && item.mid > row.baseline_mid
                    && item.body == row.body
                    && !claimed.contains(&item.mid)
            });
            if let Some(item) = candidate {
                claimed.push(item.mid);
                consumed.push(row.local_id);
            }
        }
        self.state
            .pending
            .retain(|row| !consumed.contains(&row.local_id));
    }

    /// Refresh the last-message summary of a row that is already in the list.
    pub(super) fn update_row_summary(
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
}
