//! Incremental `text/event-stream` framing.
//!
//! This module knows the SSE transport grammar and nothing about chat: bytes in,
//! [`Frame`]s out. The server names its events (`Chat`, `Heartbeat`) and also
//! emits `: keep-alive-text` comment lines every few seconds; comments are
//! consumed here and never become frames, which is what keeps them out of the
//! message pane.
//!
//! The parser is incremental because a network chunk is not a frame: a chunk can
//! end in the middle of a line, in the middle of a multi-byte UTF-8 character,
//! or contain several frames at once. Everything incomplete stays buffered until
//! the rest arrives, which is also why it can be tested by feeding bytes.

/// A server that never sends a newline must not grow the buffer without bound.
pub const MAX_LINE_BYTES: usize = 1 << 20;

/// One dispatched event. The transport keeps the server's event name verbatim;
/// deciding what a name means is [`crate::model`]'s job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub event: String,
    pub data: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SseError {
    #[error("event-stream line is not valid UTF-8")]
    Utf8,
    #[error("event-stream line exceeded {MAX_LINE_BYTES} bytes without a newline")]
    TooLong,
}

/// Bytes-to-frames state machine. One instance per connection.
#[derive(Debug, Default)]
pub struct Parser {
    /// Bytes of the current, still incomplete line.
    buffer: Vec<u8>,
    /// The `event:` field of the frame being assembled.
    event: Option<String>,
    /// The `data:` field(s) of the frame being assembled, newline-terminated.
    data: String,
}

impl Parser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one network chunk, get back every frame it completed.
    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<Frame>, SseError> {
        self.buffer.extend_from_slice(chunk);

        let mut frames = Vec::new();
        while let Some(newline) = self.buffer.iter().position(|byte| *byte == b'\n') {
            let mut line: Vec<u8> = self.buffer.drain(..=newline).collect();
            line.pop(); // the '\n'
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            let line = std::str::from_utf8(&line).map_err(|_| SseError::Utf8)?;
            self.line(line, &mut frames);
        }

        if self.buffer.len() > MAX_LINE_BYTES {
            return Err(SseError::TooLong);
        }
        Ok(frames)
    }

    fn line(&mut self, line: &str, frames: &mut Vec<Frame>) {
        // A blank line dispatches the frame being assembled.
        if line.is_empty() {
            if self.data.is_empty() {
                // An event with no data carries nothing; drop it and its name.
                self.event = None;
                return;
            }
            let data = std::mem::take(&mut self.data);
            let data = data.strip_suffix('\n').unwrap_or(&data).to_string();
            // Per the SSE spec an unnamed event has the type "message".
            let event = self.event.take().unwrap_or_else(|| "message".to_string());
            frames.push(Frame { event, data });
            return;
        }

        // `: keep-alive-text` and friends: liveness only, never a message.
        if line.starts_with(':') {
            return;
        }

        let (field, value) = match line.split_once(':') {
            Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
            None => (line, ""),
        };
        match field {
            "event" => self.event = Some(value.to_string()),
            "data" => {
                self.data.push_str(value);
                self.data.push('\n');
            }
            // `id`, `retry`, and unknown fields have no meaning for this client.
            _ => {}
        }
    }
}
