//! Terminal UI client for the chat server.
//!
//! The client speaks HTTP and JSON only: it deliberately does not depend on the
//! server's `entity` or `msg` crates, so no database driver enters its build graph.
//!
//! Two seams carry the test coverage:
//!
//! - [`state`] is a pure state machine. Events in, state and effects out; no I/O
//!   and no rendering.
//! - [`api`] is the HTTP wire client. It is tested against a stub server to pin
//!   request shape, and against JSON captured from the server's own types.

pub mod api;
pub mod app;
pub mod model;
pub mod render;
pub mod state;
