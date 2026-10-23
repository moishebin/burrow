//! The top bar of the frameless window (D-035, D-045): the page's address on the left and
//! minimize, maximize and close on the right, as in Arc on Windows. It stays out of sight until
//! the mouse goes to the top: at rest the overlay is only the strip above the content card,
//! painted the window's color. Hovering anywhere along that strip opens the bar across the whole
//! card and pushes the page down by its height, so it never covers the page.

use cef::*;

use super::{argb, cef, Burrow};
use crate::{consts, task, views};

pub const BUTTON_MINIMIZE: i32 = 101;
pub const BUTTON_MAXIMIZE: i32 = 102;
pub const BUTTON_CLOSE: i32 = 103;
/// The address on the left of the bar; clicking it copies the link.
pub const BAR_URL: i32 = 104;
const IDS: [i32; 4] = [BAR_URL, BUTTON_MINIMIZE, BUTTON_MAXIMIZE, BUTTON_CLOSE];

const BUTTON_WIDTH: i32 = 46;
/// The open bar's height, from the window's top edge.
const BAR_HEIGHT: i32 = 32;
/// How long the bar stays after the mouse leaves it.
const HIDE_DELAY_MS: i64 = 500;
const CLOSE_RED: u32 = 0xFFC42B1C;
const INK: u32 = 0xFF1F1D2B;
const WHITE: u32 = 0xFFFFFFFF;
/// Roughly how wide one character of the address is, to shorten it before it reaches the buttons.
const CHAR_WIDTH: i32 = 7;

pub(super) struct WindowButtons {
    overlay: OverlayController,
    /// In `IDS` order.
    buttons: Vec<LabelButton>,
    hovered: [bool; 4],
    open: bool,
    /// Bumped on every hover change; a pending hide only runs if nothing happened since.
    generation: u64,
}

#[derive(Clone, Copy)]
enum Glyph {
    Minimize,
    Maximize,
    Restore,
    Close,
}

/// Whether a point (in 10×10 logical pixels) is inked in a caption glyph, Windows-style with a
/// one-pixel stroke.
fn inked(g: Glyph, x: f32, y: f32) -> bool {
    let outline = |x0: f32, y0: f32, x1: f32, y1: f32| {
        (x0..=x1).contains(&x) && (y0..=y1).contains(&y) && !(x > x0 + 1.0 && x < x1 - 1.0 && y > y0 + 1.0 && y < y1 - 1.0)
    };
    match g {
        Glyph::Minimize => (4.5..=5.5).contains(&y),
        Glyph::Maximize => outline(0.0, 0.0, 10.0, 10.0),
        Glyph::Restore => {
            let front = outline(0.0, 2.0, 8.0, 10.0);
            let behind = (0.0..8.0).contains(&x) && (2.0..10.0).contains(&y);
            front || (!behind && outline(2.0, 0.0, 10.0, 8.0))
        }
        // Within half a stroke of either diagonal.
        Glyph::Close => (x - y).abs() <= 0.75 || (x + y - 10.0).abs() <= 0.75,
    }
}

/// Draws a glyph in `color` at 1x and 2x. Drawn rather than taken from an icon font: Segoe MDL2
/// Assets showed as missing-character boxes on Windows.
fn glyph_image(g: Glyph, color: u32) -> Option<Image> {
    let image = image_create()?;
    let [_, r, gr, b] = color.to_be_bytes();
    for scale in [1, 2] {
        let size = 10 * scale;
        let mut pixels = Vec::with_capacity((size * size * 4) as usize);
        for py in 0..size {
            for px in 0..size {
                // 4×4 samples per pixel, for smooth diagonals.
                let mut hits = 0;
                for sy in 0..4 {
                    for sx in 0..4 {
                        let x = (px as f32 + (sx as f32 + 0.5) / 4.0) / scale as f32;
                        let y = (py as f32 + (sy as f32 + 0.5) / 4.0) / scale as f32;
                        hits += inked(g, x, y) as u32;
                    }
                }
                pixels.extend_from_slice(&[r, gr, b, (hits * 255 / 16) as u8]);
            }
        }
        image.add_bitmap(scale as f32, size, size, ColorType::RGBA_8888, AlphaType::POSTMULTIPLIED, Some(&pixels));
    }
    Some(image)
}

fn glyph(id: i32, maximized: bool) -> Glyph {
    match id {
        BUTTON_MINIMIZE => Glyph::Minimize,
        BUTTON_MAXIMIZE if maximized => Glyph::Restore,
        BUTTON_MAXIMIZE => Glyph::Maximize,
        _ => Glyph::Close,
    }
}

/// Blends an ARGB color toward black by `amount`.
fn darken(c: u32, amount: f32) -> u32 {
    let ch = |shift: u32| (((((c >> shift) & 0xFF) as f32) * (1.0 - amount)).round() as u32) << shift;
    0xFF00_0000 | ch(16) | ch(8) | ch(0)
}

/// Shortens `url` to at most `max` characters, ending in an ellipsis.
fn shorten(url: &str, max: usize) -> String {
    if url.chars().count() <= max {
        return url.to_string();
    }
    let mut s: String = url.chars().take(max.saturating_sub(1)).collect();
    s.push('…');
    s
}

impl Burrow {
    /// Creates the top bar. Only for the frameless window; the system title bar has its own.
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
            let b = label_button_create(Some(&mut delegate), Some(&cef("")))?;
            View::from(&b).set_id(id);
            View::from(&b).set_focusable(0);
            if id == BAR_URL {
                b.set_horizontal_alignment(HorizontalAlignment::LEFT);
                // Small, so a long address never pushes the buttons out of the bar.
                b.set_minimum_size(Some(&Size { width: 1, height: BAR_HEIGHT }));
            } else {
                b.set_horizontal_alignment(HorizontalAlignment::CENTER);
                b.set_minimum_size(Some(&Size { width: BUTTON_WIDTH, height: BAR_HEIGHT }));
            }
            b.set_tooltip_text(Some(&cef(match id {
                BAR_URL => "Copy link",
                BUTTON_MINIMIZE => "Minimize",
                BUTTON_MAXIMIZE => "Maximize",
                _ => "Close",
            })));
            panel.add_child_view(Some(&mut View::from(&b)));
            if let Some(l) = &layout {
                l.set_flex_for_view(Some(&mut View::from(&b)), (id == BAR_URL) as i32);
            }
            buttons.push(b);
        }
        let overlay = window.add_overlay_view(Some(&mut View::from(&panel)), DockingMode::CUSTOM, 0)?;
        self.window_buttons = Some(WindowButtons { overlay, buttons, hovered: [false; 4], open: false, generation: 0 });
        self.layout_window_buttons();
        Some(())
    }

    /// Places the bar over the top of the card (a thin strip, or the full bar while open),
    /// pushes the page down while it is open, and paints it in the space's color. Hidden in
    /// fullscreen.
    pub(super) fn layout_window_buttons(&self) {
        let Some(wb) = &self.window_buttons else { return };
        let open = wb.open && !self.fullscreen;
        let push = if open { BAR_HEIGHT - consts::CARD_MARGIN } else { 0 };
        let content = View::from(&self.content);
        if content.insets().top != push {
            content.set_insets(Some(&Insets { top: push, left: 0, bottom: 0, right: 0 }));
            self.content.layout();
        }
        if self.fullscreen {
            return wb.overlay.set_visible(0);
        }
        let card = content.bounds();
        let width = card.width + consts::CARD_MARGIN;
        let height = if open { BAR_HEIGHT } else { consts::CARD_MARGIN };
        wb.overlay.set_bounds(Some(&Rect { x: card.x, y: 0, width, height }));

        let base = argb(&self.state.active_space().color);
        let maximized = self.window.as_ref().is_some_and(|w| w.is_maximized() != 0);
        if let Some(panel) = wb.overlay.contents_view() {
            panel.set_background_color(base);
        }
        for (i, b) in wb.buttons.iter().enumerate() {
            let id = IDS[i];
            let hovered = wb.hovered[i];
            let bg = match (id, hovered) {
                (BAR_URL, _) => base,
                (BUTTON_CLOSE, true) => CLOSE_RED,
                (_, true) => darken(base, 0.12),
                _ => base,
            };
            View::from(b).set_background_color(bg);
            // The resting strip is too thin for text and glyphs; they'd show as cut-off marks.
            if id == BAR_URL {
                let url = self.state.active_tab().map_or("", |t| t.url.as_str());
                let room = ((width - BUTTON_WIDTH * 3 - 24) / CHAR_WIDTH).max(1) as usize;
                b.set_enabled_text_colors(INK);
                b.set_text(Some(&cef(&if open { shorten(url, room) } else { String::new() })));
                continue;
            }
            let ink = if id == BUTTON_CLOSE && hovered { WHITE } else { INK };
            let mut image = if open { glyph_image(glyph(id, maximized), ink) } else { None };
            for state in [ButtonState::NORMAL, ButtonState::HOVERED, ButtonState::PRESSED] {
                b.set_image(state, image.as_mut());
            }
            if id == BUTTON_MAXIMIZE {
                b.set_tooltip_text(Some(&cef(if maximized { "Restore" } else { "Maximize" })));
            }
        }
        wb.overlay.set_visible(1);
    }

    pub fn window_button(&mut self, id: i32) {
        match id {
            BAR_URL => self.shortcut(super::Shortcut::CopyUrl),
            BUTTON_MINIMIZE => self.act(super::Act::WindowMinimize),
            BUTTON_MAXIMIZE => self.act(super::Act::WindowMaximize),
            BUTTON_CLOSE => self.act(super::Act::WindowClose),
            super::modal::BACKDROP_BUTTON => self.close_modal(),
            _ => {}
        }
    }

    /// The mouse entered or left part of the bar. Entering opens it at once; leaving all of it
    /// hides it after a short delay, so moving across it doesn't flicker.
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

    /// Whether the top bar is open (hovered, or shown by an agent).
    pub fn top_bar_open(&self) -> bool {
        self.window_buttons.as_ref().is_some_and(|wb| wb.open) && !self.fullscreen
    }

    /// Shows the bar for a moment without the mouse (agents and tests can't hover).
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
