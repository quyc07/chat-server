//! [`Machine`] methods for the reconnect concern.

use super::*;

impl Machine {
    /// A push connection opened. The first one after login is the normal start
    /// of the stream; one that follows a drop is a reconnect and owes the user
    /// whatever was said while the link was down.
    pub(super) fn link_up(&mut self) -> Vec<Effect> {
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
    pub(super) fn link_down(&mut self) -> Vec<Effect> {
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
    pub(super) fn catch_up_loaded(
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
        // A row the refetched history already contains is confirmed history now;
        // dropping it is what keeps it from being drawn twice. This runs before
        // the merge because the snapshot can beat the send reply.
        self.reconcile_pending(conversation, &items);
        self.state.pending.retain(|row| {
            !(row.conversation == conversation
                && matches!(row.delivery, Delivery::Confirmed { mid, .. }
                    if items.iter().any(|item| item.mid == mid)))
        });
        merge_history(&mut self.state.messages, items);
        // The messages missed during the outage are on screen now, so the open
        // conversation is read up to them as well.
        self.report_newest_read(conversation)
    }

    /// The catch-up request itself failed (the redial worked). Keep the pane and
    /// say so on the status line; the next reconnect retries.
    pub(super) fn catch_up_failed(
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
        self.state.status = Some(format!("补拉聊天记录失败：{message}"));
        Vec::new()
    }
}
