//! Terminal UI client for the chat server.
//!
//! The client speaks HTTP and JSON only: it deliberately does not depend on the
//! server's `entity` or `msg` crates, so no database driver enters its build graph.
//!
//! Three seams carry the test coverage:
//!
//! - [`state`] is a pure state machine. Events in, state and effects out; no I/O
//!   and no rendering.
//! - [`api`] is the HTTP wire client. It is tested against a stub server to pin
//!   request shape, and against JSON captured from the server's own types.
//! - [`link`] is the push-link supervisor. Its liveness watchdog takes the clock
//!   as a parameter, so the 15-second silence threshold is tested with made-up
//!   instants, and the redial loop is tested against a stub server with the
//!   retry delay injected instead of slept through.

pub mod api;
pub mod app;
pub mod link;
pub mod model;
pub mod render;
pub mod sse;
pub mod state;

pub mod theme;
