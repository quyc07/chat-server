//! [`Machine`] methods for the read index concern.

use super::*;

impl Machine {
    /// The server accepted a read position.
    ///
    /// Nothing is recomputed locally — the unread count is the server's logic —
    /// so the only visible consequence is the authoritative list refresh that
    /// carries the count the report just changed.
    pub(super) fn read_reported(&mut self) -> Vec<Effect> {
        if !self.state.is_logged_in() {
            return Vec::new();
        }
        self.schedule_refresh();
        Vec::new()
    }

    pub(super) fn read_report_failed(
        &mut self,
        position: ReadPosition,
        message: String,
    ) -> Vec<Effect> {
        if !self.state.is_logged_in() {
            return Vec::new();
        }
        // Nothing was accepted, so the position does not count as reported: the
        // next open, history load, or stream message retries it.
        if self.reported.get(&position.conversation) == Some(&position.mid) {
            self.reported.remove(&position.conversation);
        }
        // Never a dialog and never a blocked input: only the status line, and
        // only the server's own text — the ids stay inside the client.
        self.state.status = Some(format!("已读状态同步失败：{message}"));
        Vec::new()
    }

    /// Report the newest message currently in the pane, if it advances.
    ///
    /// Both ways the pane gains history (the initial load and the reconnect
    /// catch-up) end here, so a message the user can see is never left unread.
    pub(super) fn report_newest_read(&mut self, conversation: ConversationId) -> Vec<Effect> {
        match self.state.messages.iter().map(|message| message.mid).max() {
            Some(newest) => self.report_read(ReadPosition {
                conversation,
                mid: newest,
            }),
            None => Vec::new(),
        }
    }

    /// Ask the server to mark `conversation` read up to `mid`.
    ///
    /// Reporting is advance-only: the same or an older position is never sent
    /// again, so opening, loading history, and every stream message can each ask
    /// without turning into duplicate requests. The tracker records the position
    /// as reported *before* the request leaves; a rejection rolls it back so the
    /// next event retries instead of silently staying unread.
    pub(super) fn report_read(&mut self, position: ReadPosition) -> Vec<Effect> {
        // Message ids start at 1 (the server's sled sequence), so there is no
        // position to report before the first message. Reporting one anyway
        // would make the server upsert a row for the peer.
        if position.mid < 1 {
            return Vec::new();
        }
        if self
            .reported
            .get(&position.conversation)
            .copied()
            .unwrap_or(i64::MIN)
            >= position.mid
        {
            return Vec::new();
        }
        self.reported.insert(position.conversation, position.mid);
        vec![Effect::ReportRead(position)]
    }
}
