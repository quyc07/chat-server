//! Every screen must draw at every size without panicking.
//!
//! This asserts nothing about appearance — the project deliberately has no
//! render seam for pixels. What it does pin is the collapse behaviour: layout
//! arithmetic over a tiny or zero-sized area is where TUI renderers panic, and
//! that is a crash rather than a cosmetic bug.

use chat_tui::render;
use chat_tui::state::{Event, Key, Machine};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

fn machine_on(screen: Screen) -> Machine {
    let mut machine = Machine::new();
    if screen == Screen::Login {
        return machine;
    }
    // `uid_from_token` only reads the payload segment.
    machine.handle(Event::LoginSucceeded {
        token: "x.eyJpZCI6MX0.y".to_string(),
    });
    if screen == Screen::Friends {
        machine.handle(Event::Key(Key::Friends));
    }
    machine
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Screen {
    Login,
    Conversations,
    Friends,
}

#[test]
fn every_screen_draws_at_every_size_without_panicking() {
    let sizes = [
        (0, 0),
        (1, 1),
        (1, 40),
        (40, 1),
        (2, 2),
        (20, 8),
        (39, 7),
        (40, 8),
        (69, 15),
        (70, 16),
        (80, 24),
        (120, 40),
        (200, 60),
    ];
    for screen in [Screen::Login, Screen::Conversations, Screen::Friends] {
        let mut machine = machine_on(screen);
        for (width, height) in sizes {
            machine.handle(Event::Resize { width, height });
            let backend = TestBackend::new(width.max(1), height.max(1));
            let mut terminal = Terminal::new(backend).unwrap();
            terminal
                .draw(|frame| render::draw(frame, machine.state()))
                .unwrap_or_else(|error| {
                    panic!("{screen:?} failed to draw at {width}x{height}: {error}")
                });
        }
    }
}
