//! The top bar of the frameless window (D-035, D-045, D-047, D-051): the page's site in the middle,
//! between a copy-link and a site-controls button, and minimize, maximize and close on the right,
//! as in Arc on Windows. It stays out of sight until
//! the mouse goes to the top: at rest it is only the strip above the content card, painted the
//! window's color. With the mouse anywhere along that strip, the bar opens across the whole card
//! and pushes the page down by its height, so it never covers the page.
//!
//! The site and the window buttons are two window overlays. Around the site is the window itself,
//! which is a drag area for moving the window: an overlay is its own native child window on Windows, so
//! the window's drag areas don't reach under one. Over a drag area the views don't see the mouse,
//! so on Windows Burrow follows the cursor itself to open and close the bar; elsewhere it uses the
//! overlays' hover and the bar has no drag area.

use cef::*;

use super::{argb, cef, mix_with_white, now_ms, Burrow};
use crate::{consts, task, views};

pub const BUTTON_MINIMIZE: i32 = 101;
pub const BUTTON_MAXIMIZE: i32 = 102;
pub const BUTTON_CLOSE: i32 = 103;
/// The site in the middle of the bar; clicking it opens the address box.
pub const BAR_URL: i32 = 104;
/// Left of the site: copies the link.
pub const BAR_LINK: i32 = 105;
/// Right of the site: the site's controls (zoom, Boost, …).
pub const BAR_SITE: i32 = 106;
/// The site's part of the bar, in the order shown, then the window buttons.
const IDS: [i32; 6] = [BAR_LINK, BAR_URL, BAR_SITE, BUTTON_MINIMIZE, BUTTON_MAXIMIZE, BUTTON_CLOSE];
const SITE_PARTS: usize = 3;

const BUTTON_WIDTH: i32 = 46;
const BUTTONS_WIDTH: i32 = BUTTON_WIDTH * 3;
/// The open bar's height, from the window's top edge.
const BAR_HEIGHT: i32 = 32;
/// How long the bar stays after the mouse leaves it.
const HIDE_DELAY_MS: i64 = 500;
/// How often Burrow looks at the cursor where it follows it itself.
const TRACK_MS: i64 = 100;
const CLOSE_RED: u32 = 0xFFC42B1C;
const INK: u32 = 0xFF1F1D2B;
/// The address and the glyphs at rest: quieter than the page, dark again on hover.
const INK_SOFT: u32 = 0xFF5F5B6E;
const WHITE: u32 = 0xFFFFFFFF;
/// Width of the copy-link and site-controls buttons.
const ICON_WIDTH: i32 = 30;
#[cfg(target_os = "windows")]
const FONT: &str = "Segoe UI, 12px";
#[cfg(not(target_os = "windows"))]
const FONT: &str = "Sans, 12px";
/// Roughly how wide one character of the site is, to shorten it before it reaches the buttons.
const CHAR_WIDTH: i32 = 7;

pub(super) struct WindowButtons {
    /// The site; at rest, the whole strip above the card.
    left: OverlayController,
    left_layout: Option<BoxLayout>,
    /// Minimize, maximize and close.
    right: OverlayController,
    /// In `IDS` order: the site's three parts, then the window buttons.
    buttons: Vec<LabelButton>,
    hovered: [bool; 6],
    open: bool,
    /// Bumped on every hover change; a pending hide only runs if nothing happened since.
    generation: u64,
    /// How long the cursor has been away from the open bar, where Burrow follows it.
    away_ms: i64,
    /// An agent showed the bar; it stays until then even without the mouse.
    reveal_until: u64,
}

#[derive(Clone, Copy)]
enum Glyph {
    Minimize,
    Maximize,
    Restore,
    Close,
    /// A chain link (copy link), 14×14.
    Link,
    /// Two sliders (site controls), 14×14.
    Sliders,
}

impl Glyph {
    /// Logical size: caption glyphs are 10 px, the site's icons 14.
    fn size(self) -> i32 {
        match self {
            Glyph::Link | Glyph::Sliders => 14,
            _ => 10,
        }
    }
}

/// Distance from (x, y) to the segment (ax, ay)–(bx, by).
fn to_segment(x: f32, y: f32, (ax, ay): (f32, f32), (bx, by): (f32, f32)) -> f32 {
    let (dx, dy) = (bx - ax, by - ay);
    let t = (((x - ax) * dx + (y - ay) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
    ((x - ax - t * dx).powi(2) + (y - ay - t * dy).powi(2)).sqrt()
}

/// Whether a point (in the glyph's logical pixels) is inked. Caption glyphs are Windows-style
/// with a one-pixel stroke; the site's icons are line icons with a 1.3 px stroke.
fn inked(g: Glyph, x: f32, y: f32) -> bool {
    const HALF: f32 = 0.65;
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
        // A rounded link, open at the top and bottom middle, with a bar across.
        Glyph::Link => {
            let ring = (to_segment(x, y, (4.0, 7.0), (10.0, 7.0)) - 3.2).abs() <= HALF;
            let gap = (x - 7.0).abs() < 1.6 && (y - 7.0).abs() > 1.5;
            (ring && !gap) || to_segment(x, y, (4.6, 7.0), (9.4, 7.0)) <= HALF
        }
        // Two rails, each with a ring for its knob.
        Glyph::Sliders => [(4.0, 9.0), (10.0, 5.0)].iter().any(|&(y0, knob): &(f32, f32)| {
            let r = ((x - knob).powi(2) + (y - y0).powi(2)).sqrt();
            (r - 2.0).abs() <= HALF || (to_segment(x, y, (1.0, y0), (13.0, y0)) <= HALF && r > 2.0)
        }),
    }
}

/// Draws a glyph in `color` at 1x and 2x. Drawn rather than taken from an icon font: Segoe MDL2
/// Assets showed as missing-character boxes on Windows.
fn glyph_image(g: Glyph, color: u32) -> Option<Image> {
    let image = image_create()?;
    let [_, r, gr, b] = color.to_be_bytes();
    for scale in [1, 2] {
        let size = g.size() * scale;
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
        BAR_LINK => Glyph::Link,
        BAR_SITE => Glyph::Sliders,
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

/// The site as the bar shows it: the host without `www.`, or the address for pages without one.
fn display_site(url: &str) -> &str {
    let Some((_, rest)) = url.split_once("://") else { return url };
    let host = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let host = host.strip_prefix("www.").unwrap_or(host);
    if host.is_empty() { rest } else { host }
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

/// A horizontal panel for an overlay, its children centered along it.
fn row() -> Option<(Panel, Option<BoxLayout>)> {
    let panel = panel_create(None)?;
    let layout = panel.set_to_box_layout(Some(&BoxLayoutSettings {
        horizontal: 1,
        main_axis_alignment: AxisAlignment::CENTER,
        cross_axis_alignment: AxisAlignment::STRETCH,
        ..Default::default()
    }));
    Some((panel, layout))
}

impl Burrow {
    /// Creates the top bar. Only for the frameless window; the system title bar has its own.
    pub(super) fn create_window_buttons(&mut self) -> Option<()> {
        if !self.frameless() || self.window_buttons.is_some() {
            return None;
        }
        let window = self.window.clone()?;
        let (left_panel, left_layout) = row()?;
        let (right_panel, _) = row()?;
        let mut buttons = Vec::new();
        for (i, id) in IDS.into_iter().enumerate() {
            let mut delegate = views::WindowButtonDelegate::new();
            let b = label_button_create(Some(&mut delegate), Some(&cef("")))?;
            View::from(&b).set_id(id);
            View::from(&b).set_focusable(0);
            b.set_tooltip_text(Some(&cef(match id {
                BAR_LINK => "Copy link",
                BAR_URL => "Edit address",
                BAR_SITE => "Site controls",
                BUTTON_MINIMIZE => "Minimize",
                BUTTON_MAXIMIZE => "Maximize",
                _ => "Close",
            })));
            b.set_horizontal_alignment(HorizontalAlignment::CENTER);
            if id == BAR_URL {
                b.set_font_list(Some(&cef(FONT)));
            } else {
                let width = if i < SITE_PARTS { ICON_WIDTH } else { BUTTON_WIDTH };
                b.set_minimum_size(Some(&Size { width, height: BAR_HEIGHT }));
            }
            let panel = if i < SITE_PARTS { &left_panel } else { &right_panel };
            panel.add_child_view(Some(&mut View::from(&b)));
            buttons.push(b);
        }
        let left = window.add_overlay_view(Some(&mut View::from(&left_panel)), DockingMode::CUSTOM, 0)?;
        let right = window.add_overlay_view(Some(&mut View::from(&right_panel)), DockingMode::CUSTOM, 0)?;
        self.window_buttons = Some(WindowButtons {
            left,
            left_layout,
            right,
            buttons,
            hovered: [false; 6],
            open: false,
            generation: 0,
            away_ms: 0,
            reveal_until: 0,
        });
        self.layout_window_buttons();
        if cursor_screen_point().is_some() {
            track_cursor();
        }
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
        let base = argb(&self.state.active_space().color);
        // Under the bar's drag area the card itself shows: the space color while open.
        self.content.set_background_color(if open { base } else { mix_with_white(base, 0.75) });
        if self.fullscreen {
            wb.left.set_visible(0);
            wb.right.set_visible(0);
            return self.apply_drag_regions();
        }
        let card = content.bounds();
        let width = card.width + consts::CARD_MARGIN;
        let url_room = width - BUTTONS_WIDTH;
        let tab = self.state.active_tab().filter(|_| open);
        let url = &wb.buttons[1];
        let text = display_site(tab.map_or("", |t| t.url.as_str()));
        let room = (url_room - 2 * ICON_WIDTH - 24) / CHAR_WIDTH;
        url.set_text(Some(&cef(&shorten(text, room.max(1) as usize))));
        url.set_enabled_text_colors(if wb.hovered[1] { INK } else { INK_SOFT });
        // At rest the empty site fills the strip, so hovering it opens the bar (where Burrow
        // doesn't follow the cursor). The icons only show for a page.
        for (i, b) in wb.buttons[..SITE_PARTS].iter().enumerate() {
            View::from(b).set_visible((IDS[i] == BAR_URL || tab.is_some()) as i32);
        }
        if let Some(l) = &wb.left_layout {
            l.set_flex_for_view(Some(&mut View::from(url)), (!open) as i32);
        }
        if let Some(panel) = wb.left.contents_view() {
            panel.set_background_color(base);
        }
        if open {
            wb.left.set_bounds(Some(&self.top_bar_site_rect(url_room)));
            wb.right.set_bounds(Some(&Rect { x: card.x + url_room, y: 0, width: BUTTONS_WIDTH, height: BAR_HEIGHT }));
        } else {
            wb.left.set_bounds(Some(&Rect { x: card.x, y: 0, width, height: consts::CARD_MARGIN }));
        }

        let maximized = self.window.as_ref().is_some_and(|w| w.is_maximized() != 0);
        for (i, b) in wb.buttons.iter().enumerate() {
            let id = IDS[i];
            let hovered = wb.hovered[i];
            let (bg, ink) = match (id, hovered) {
                (BUTTON_CLOSE, true) => (CLOSE_RED, WHITE),
                (_, true) => (darken(base, 0.08), INK),
                _ => (base, INK_SOFT),
            };
            if id == BAR_URL {
                // Only its text darkens on hover, like a link.
                View::from(b).set_background_color(base);
                continue;
            }
            View::from(b).set_background_color(bg);
            let mut image = glyph_image(glyph(id, maximized), ink);
            for state in [ButtonState::NORMAL, ButtonState::HOVERED, ButtonState::PRESSED] {
                b.set_image(state, image.as_mut());
            }
            if id == BUTTON_MAXIMIZE {
                b.set_tooltip_text(Some(&cef(if maximized { "Restore" } else { "Maximize" })));
            }
        }
        wb.left.set_visible(1);
        wb.right.set_visible(open as i32);
        self.apply_drag_regions();
    }

    pub fn window_button(&mut self, id: i32) {
        match id {
            BAR_LINK => self.shortcut(super::Shortcut::CopyUrl),
            BAR_URL => self.shortcut(super::Shortcut::FocusAddress),
            BAR_SITE => self.open_site_controls(),
            BUTTON_MINIMIZE => self.act(super::Act::WindowMinimize),
            BUTTON_MAXIMIZE => self.act(super::Act::WindowMaximize),
            BUTTON_CLOSE => self.act(super::Act::WindowClose),
            super::modal::BACKDROP_BUTTON => self.close_modal(),
            _ => {}
        }
    }

    /// The mouse entered or left part of the bar. Where Burrow follows the cursor itself this
    /// only colors the buttons. Elsewhere entering opens the bar at once; leaving all of it hides
    /// it after a short delay, so moving across it doesn't flicker.
    pub fn window_button_hover(&mut self, id: i32, hovered: bool) {
        let Some(wb) = &mut self.window_buttons else { return };
        let Some(i) = IDS.iter().position(|x| *x == id) else { return };
        wb.hovered[i] = hovered;
        if cursor_screen_point().is_none() {
            wb.generation += 1;
            if wb.hovered.iter().any(|h| *h) {
                wb.open = true;
            } else {
                schedule_hide(wb.generation, HIDE_DELAY_MS);
            }
        }
        self.layout_window_buttons();
    }

    /// The site controls (D-051): a dialog for the shown page's site, from the bar's sliders.
    pub(super) fn open_site_controls(&mut self) {
        if let Some(url) = self.state.active_tab().map(|t| t.url.clone()) {
            self.open_dialog("site", &serde_json::json!([url]));
        }
    }

    /// Opens or closes the bar by where the cursor is: over the strip above the card, or over the
    /// open bar, and not over another window in front of Burrow's.
    fn track_cursor_tick(&mut self) {
        let Some(window) = self.window.clone() else { return };
        let Some(wb) = &self.window_buttons else { return };
        let Some(mut point) = cursor_screen_point() else { return };
        View::from(&window).convert_point_from_screen(Some(&mut point));
        let card = View::from(&self.content).bounds();
        let height = if wb.open { BAR_HEIGHT } else { consts::CARD_MARGIN };
        let inside = !self.fullscreen
            && window.is_minimized() == 0
            && (card.x..card.x + card.width + consts::CARD_MARGIN).contains(&point.x)
            && (0..height).contains(&point.y)
            && cursor_over_window(&window);
        let Some(wb) = &mut self.window_buttons else { return };
        if inside {
            wb.away_ms = 0;
            if !wb.open {
                wb.open = true;
                self.layout_window_buttons();
            }
        } else if wb.open && now_ms() >= wb.reveal_until {
            wb.away_ms += TRACK_MS;
            if wb.away_ms >= HIDE_DELAY_MS {
                wb.open = false;
                self.layout_window_buttons();
            }
        }
    }

    /// The site's overlay while the bar is open: centered over the card, as wide as the site and
    /// its two buttons, where Burrow follows the cursor (the rest of the bar moves the window).
    /// Elsewhere it spans the bar up to the window buttons, so hovering anywhere keeps it open,
    /// and its row centers the site.
    fn top_bar_site_rect(&self, url_room: i32) -> Rect {
        let card = View::from(&self.content).bounds();
        let full = Rect { x: card.x, y: 0, width: url_room, height: BAR_HEIGHT };
        let Some(wb) = &self.window_buttons else { return full };
        if cursor_screen_point().is_none() {
            return full;
        }
        let width = wb.buttons[..SITE_PARTS]
            .iter()
            .filter(|b| View::from(*b).is_visible() != 0)
            .map(|b| View::from(b).preferred_size().width)
            .sum::<i32>()
            .clamp(1, url_room);
        // Centered on the card, but never over the window buttons.
        let x = (card.x + (card.width - width) / 2).min(card.x + url_room - width).max(card.x);
        Rect { x, y: 0, width, height: BAR_HEIGHT }
    }

    /// The open bar's drag areas in window coordinates: on both sides of the site.
    pub(super) fn top_bar_drag_rects(&self) -> Vec<Rect> {
        if !self.top_bar_open() || cursor_screen_point().is_none() {
            return vec![];
        }
        let card = View::from(&self.content).bounds();
        let url_room = card.width + consts::CARD_MARGIN - BUTTONS_WIDTH;
        let site = self.top_bar_site_rect(url_room);
        let end = card.x + url_room;
        [(card.x, site.x), (site.x + site.width, end)]
            .into_iter()
            .filter(|(a, b)| b > a)
            .map(|(a, b)| Rect { x: a, y: 0, width: b - a, height: BAR_HEIGHT })
            .collect()
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
        wb.reveal_until = now_ms() + 2000;
        let generation = wb.generation;
        self.layout_window_buttons();
        if cursor_screen_point().is_none() {
            schedule_hide(generation, 2000);
        }
        true
    }
}

/// Closes the bar after `delay_ms` unless it was hovered since (`generation` changed).
fn schedule_hide(generation: u64, delay_ms: i64) {
    task::post_ui_delayed(delay_ms, move || {
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

/// Looks at the cursor every `TRACK_MS` while the window is open.
fn track_cursor() {
    task::post_ui_delayed(TRACK_MS, || {
        let alive = super::with(|b| {
            b.track_cursor_tick();
            b.window.is_some()
        });
        if alive == Some(true) {
            track_cursor();
        }
    });
}

/// The mouse's position on screen in DIPs, where Burrow can ask for it.
#[cfg(target_os = "windows")]
fn cursor_screen_point() -> Option<Point> {
    let mut p = windows::Win32::Foundation::POINT::default();
    unsafe { windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut p) }.ok()?;
    Some(display_convert_screen_point_from_pixels(Some(&Point { x: p.x, y: p.y })))
}

#[cfg(not(target_os = "windows"))]
fn cursor_screen_point() -> Option<Point> {
    None
}

/// Whether the window under the cursor is Burrow's (or one of its overlays), not one in front.
#[cfg(target_os = "windows")]
fn cursor_over_window(window: &Window) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{GetAncestor, GetCursorPos, WindowFromPoint, GA_ROOTOWNER};
    let mut p = windows::Win32::Foundation::POINT::default();
    if unsafe { GetCursorPos(&mut p) }.is_err() {
        return false;
    }
    let root = unsafe { GetAncestor(WindowFromPoint(p), GA_ROOTOWNER) };
    root.0 as usize == window.window_handle().0 as usize
}

#[cfg(not(target_os = "windows"))]
fn cursor_over_window(_window: &Window) -> bool {
    true
}
