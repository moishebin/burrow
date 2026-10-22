//! Burrow's browser-independent core: the sidebar model, the commands that change it, and
//! persistence. See `docs/DECISIONS.md` D-005.

pub mod agent;
pub mod commands;
pub mod extensions;
pub mod favicon;
pub mod history;
pub mod import_arc;
pub mod links;
pub mod model;
pub mod omnibox;
pub mod store;
pub mod update;

pub use commands::{Command, CommandError, Effect, Location};
pub use model::{Folder, Id, Node, Profile, Route, Section, Settings, Space, Split, State, Tab};

#[cfg(test)]
mod tests;
