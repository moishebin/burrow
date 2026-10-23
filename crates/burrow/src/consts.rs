//! Names and fixed numbers. The product name lives here so a rename is one edit (D-008).

pub const APP_NAME: &str = "Burrow";
/// Sidebar width in DIPs.
pub const SIDEBAR_WIDTH: i32 = 220;
/// Width of the hidden sidebar's edge strip; hovering it slides the sidebar out (Arc-style).
pub const SIDEBAR_COLLAPSED_WIDTH: i32 = 10;
/// Gap around the content card, in DIPs.
pub const CARD_MARGIN: i32 = 8;
/// ARGB. Shown before the first space color is applied.
pub const BACKGROUND: u32 = 0xFFC9C2F0;
/// Prefix the sidebar uses for messages to Rust via `console.log` (D-009).
pub const IPC_PREFIX: &str = "__burrow__";
