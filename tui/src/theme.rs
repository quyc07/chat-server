//! The one place colour and glyph choices live.
//!
//! A terminal cannot carry a font, a gradient, or a blur, so "modern" here means
//! the three things it *can* carry, decided once:
//!
//! - **Body text is never coloured.** It uses the terminal's own foreground, so
//!   it stays legible whatever theme the user runs.
//! - **Muted means `DIM`, not `DarkGray`.** `DarkGray` is around 2:1 against a
//!   dark background and invisible against a light one; the `DIM` modifier
//!   adapts to both.
//! - **One accent.** Cyan marks focus, the cursor, unread state, and the app
//!   name — nowhere else. Semantic colours appear only in the status dot, so a
//!   colour always means one thing.
//!
//! Selection is `REVERSED`, which is contrast-safe on every theme by
//! construction rather than by picking a background shade that happens to fit
//! the author's terminal.

use ratatui::style::{Color, Modifier, Style};

/// Focus, cursor, unread, the app's own name. Nothing else.
pub const ACCENT: Color = Color::Cyan;

/// Unread marker: filled for a count, hollow for "never read at all", so the
/// two states are distinguishable without a second glyph on the right.
pub const DOT_UNREAD: &str = "●";
/// Never-read marker.
pub const DOT_UNREAD_ALL: &str = "○";
/// A conversation with no unread messages: keeps the gutter aligned.
pub const DOT_READ: &str = "·";

/// Text cursor drawn after the input contents.
pub const CURSOR: &str = "▏";
/// Input prompt.
pub const PROMPT: &str = "❯";

/// Horizontal rule used above the status line.
pub const RULE: char = '─';

/// A section heading: bold, uncoloured.
pub fn title() -> Style {
    Style::default().add_modifier(Modifier::BOLD)
}

/// Secondary information the user still needs to read (timestamps, previews,
/// hints): dimmed, never a dark grey.
pub fn muted() -> Style {
    Style::default().add_modifier(Modifier::DIM)
}

/// The accent, for the few things that deserve attention.
pub fn accent() -> Style {
    Style::default().fg(ACCENT)
}

/// Accent plus weight, for the focused field label.
pub fn accent_bold() -> Style {
    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
}

/// An unfocused panel edge.
pub fn border() -> Style {
    Style::default().add_modifier(Modifier::DIM)
}

/// A focused panel edge.
pub fn focused_border() -> Style {
    Style::default().fg(ACCENT)
}

/// The selected row of any list.
pub fn selected() -> Style {
    Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
}

/// Something worked.
pub fn success() -> Color {
    Color::Green
}

/// Something needs attention but is not a failure (a dropped link).
pub fn warning() -> Color {
    Color::Yellow
}

/// Something failed.
pub fn danger() -> Color {
    Color::Red
}
