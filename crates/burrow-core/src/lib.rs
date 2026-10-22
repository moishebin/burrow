//! Burrow's browser-independent core: the sidebar model, the commands that change it, and
//! persistence. See `docs/DECISIONS.md` D-005.

pub mod agent;
pub mod commands;
pub mod import_arc;
pub mod model;
pub mod omnibox;
pub mod store;

pub use commands::{Command, CommandError, Effect, Location};
pub use model::{Folder, Id, Node, Profile, Section, Settings, Space, State, Tab};

#[cfg(test)]
mod tests;
