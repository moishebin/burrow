//! Minimize, maximize and close for the frameless window (D-035). They sit in the window's
//! top-right corner, as in Arc on Windows, and stay out of sight until the mouse goes there: at
//! rest the overlay is only the strip above the content card, painted the window's color, so it
//! never covers the page. Hovering the strip slides the buttons down over the page's corner.

use cef::*;

use super::{argb, cef, mix_with_white, Burrow};
use crate::{consts, task, views};

pub const BUTTON_MINIMIZE: i32 = 101;
pub const BUTTON_MAXIMIZE: i32 = 102;
pub const BUTTON_CLOSE: i32 = 103;
const IDS: [i32; 3] = [BUTTON_MINIMIZE, BUTTON_MAXIMIZE, BUTTON_CLOSE];

const BUTTON_WIDTH: i32 = 46;
const OPEN_HEIGHT: i32 = 32;
/// How long the buttons stay after the mouse leaves them.
const HIDE_DELAY_MS: i64 = 500;
const CLOSE_RED: u32 = 0xFFC42B1C;

pub(super) struct WindowButtons {
    overlay: OverlayController,
    buttons: Vec<LabelButton>,
    hovered: [bool; 3],
    open: bool,
    /// Bumped on every hover change; a pending hide only runs if nothing happened since.
    generation: u64,
}

/// Windows draws its own caption glyphs with Segoe MDL2 Assets; elsewhere plain characters.
fn glyph(id: i32, maximized: bool) -> &'static str {
    let windows = cfg!(target_os = "windows");
    match (id, maximized, windows) {
        (BUTTON_MINIMIZE, _, true) => "\u{E921}",
        (BUTTON_MAXIMIZE, false, true) => "\u{E922}",
        (BUTTON_MAXIMIZE, true, true) => "\u{E923}",
        (_, _, true) => "\u{E8BB}",
        (BUTTON_MINIMIZE, _, false) => "—",
        (BUTTON_MAXIMIZE, false, false) => "□",
        (BUTTON_MAXIMIZE, true, false) => "❐",
        _ => "✕",
    }
}

/// Blends an ARGB color toward black by `amount`.
fn darken(c: u32, amount: f32) -> u32 {
    let ch = |shift: u32| (((((c >> shift) & 0xFF) as f32) * (1.0 - amount)).round() as u32) << shift;
    0xFF00_0000 | ch(16) | ch(8) | ch(0)
}

impl Burrow {
    /// Creates the corner buttons. Only for the frameless window; the system title bar has its own.
    pub(super) fn create_window_buttons(&mut self) -> Option<()> {
        if !self.frameless() || self.window_buttons.is_some() {
            return None;
        }
        let window = self.window.clone()?;
        let panel = panel_create(None)?;
        let layout = panel.set_to_box_layout(Some(&BoxLayoutSettings {
            horizontal: 1,
            cross_axis_alignment: AxisAlignment::STRETCH,
            ..Default::default()
        }));
        let mut buttons = Vec::new();
        for id in IDS {
            let mut delegate = views::WindowButtonDelegate::new();
            let b = label_button_create(Some(&mut delegate), Some(&cef(glyph(id, false))))?;
            View::from(&b).set_id(id);
            View::from(&b).set_focusable(0);
            b.set_horizontal_alignment(HorizontalAlignment::CENTER);
            b.set_minimum_size(Some(&Size { width: BUTTON_WIDTH, height: OPEN_HEIGHT }));
            if cfg!(target_os = "windows") {
                b.set_font_list(Some(&cef("Segoe MDL2 Assets, 10px")));
            }
            b.set_tooltip_text(Some(&cef(match id {
                BUTTON_MINIMIZE => "Minimize",
                BUTTON_MAXIMIZE => "Maximize",
                _ => "Close",
            })));
            panel.add_child_view(Some(&mut View::from(&b)));
            if let Some(l) = &layout {
                l.set_flex_for_view(Some(&mut View::from(&b)), 1);
            }
            buttons.push(b);
        }
        let overlay = window.add_overlay_view(Some(&mut View::from(&panel)), DockingMode::CUSTOM, 0)?;
        self.window_buttons = Some(WindowButtons { overlay, buttons, hovered: [false; 3], open: false, generation: 0 });
        self.layout_window_buttons();
        Some(())
    }

    /// Places the overlay in the corner (a thin strip, or the full buttons while hovered) and
    /// paints it in the space's color. Hidden in fullscreen.
    pub(super) fn layout_window_buttons(&self) {
        let Some(wb) = &self.window_buttons else { return };
        if self.fullscreen {
            return wb.overlay.set_visible(0);
        }
        let card = View::from(&self.content).bounds();
        let right = card.x + card.width + consts::CARD_MARGIN;
        let width = BUTTON_WIDTH * IDS.len() as i32;
        let height = if wb.open { OPEN_HEIGHT } else { consts::CARD_MARGIN };
        wb.overlay.set_bounds(Some(&Rect { x: right - width, y: 0, width, height }));

        let space = argb(&self.state.active_space().color);
        let base = if wb.open { mix_with_white(space, 0.35) } else { space };
        let maximized = self.window.as_ref().is_some_and(|w| w.is_maximized() != 0);
        if let Some(panel) = wb.overlay.contents_view() {
            panel.set_background_color(base);
        }
        for (i, b) in wb.buttons.iter().enumerate() {
            let id = IDS[i];
            let hovered = wb.hovered[i];
            let (bg, ink) = match (id, hovered) {
                (BUTTON_CLOSE, true) => (CLOSE_RED, 0xFFFFFFFF),
                (_, true) => (darken(base, 0.12), 0xFF1F1D2B),
                _ => (base, 0xFF1F1D2B),
            };
            View::from(b).set_background_color(bg);
            b.set_enabled_text_colors(ink);
            // The resting strip is too thin for the glyphs; they'd show as cut-off marks.
            b.set_text(Some(&cef(if wb.open { glyph(id, maximized) } else { "" })));
            if id == BUTTON_MAXIMIZE {
                b.set_tooltip_text(Some(&cef(if maximized { "Restore" } else { "Maximize" })));
            }
        }
        wb.overlay.set_visible(1);
    }

    pub fn window_button(&mut self, id: i32) {
        match id {
            BUTTON_MINIMIZE => self.act(super::Act::WindowMinimize),
            BUTTON_MAXIMIZE => self.act(super::Act::WindowMaximize),
            BUTTON_CLOSE => self.act(super::Act::WindowClose),
            super::modal::BACKDROP_BUTTON => self.close_modal(),
            _ => {}
        }
    }

    /// The mouse entered or left a button. Entering opens them at once; leaving all of them
    /// hides them after a short delay, so moving between buttons doesn't flicker.
    pub fn window_button_hover(&mut self, id: i32, hovered: bool) {
        let Some(wb) = &mut self.window_buttons else { return };
        let Some(i) = IDS.iter().position(|x| *x == id) else { return };
        wb.hovered[i] = hovered;
        wb.generation += 1;
        if wb.hovered.iter().any(|h| *h) {
            wb.open = true;
        } else {
            let generation = wb.generation;
            task::post_ui_delayed(HIDE_DELAY_MS, move || {
                super::with(|b| {
                    if let Some(wb) = &mut b.window_buttons
                        && wb.generation == generation
                    {
                        wb.open = false;
                        b.layout_window_buttons();
                    }
                });
            });
        }
        self.layout_window_buttons();
    }

    /// Shows the buttons for a moment without the mouse (agents and tests can't hover).
    pub fn reveal_window_buttons(&mut self) -> bool {
        let Some(wb) = &mut self.window_buttons else { return false };
        wb.open = true;
        wb.generation += 1;
        let generation = wb.generation;
        self.layout_window_buttons();
        task::post_ui_delayed(2000, move || {
            super::with(|b| {
                if let Some(wb) = &mut b.window_buttons
                    && wb.generation == generation
                {
                    wb.open = false;
                    b.layout_window_buttons();
                }
            });
        });
        true
    }
}
