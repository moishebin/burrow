//! The hidden sidebar floats over the page (D-046). While the sidebar is hidden, its view moves
//! out of the window's layout into an overlay of the window (never a window of its own). At rest
//! the overlay is the thin edge strip; hovering it slides the sidebar out over the page, as in
//! Arc, so the page doesn't move. The overlay (and the view filling it) grows from the edge, while
//! the sidebar's page keeps its contents at full width and slides them along with CSS (same
//! duration and easing). Moving the view instead would move it under a mouse that stands still,
//! and Chromium reports that as the mouse leaving, which ends the peek. Showing the sidebar again
//! puts the view back into the layout.

use std::time::Instant;

use cef::*;

use super::Burrow;
use crate::{consts, task};

/// How long a slide takes; `SLIDE` in sidebar.js matches it.
const SLIDE_MS: f64 = 180.0;
/// Frame interval of the slide.
const FRAME_MS: i64 = 12;

pub(super) struct Float {
    overlay: OverlayController,
    holder: Panel,
    /// How wide the overlay is now, in DIPs, from the window's left edge.
    shown: i32,
    slide: Option<Slide>,
    /// Bumped by every new slide; a frame from an older slide stops.
    generation: u64,
}

struct Slide {
    from: i32,
    to: i32,
    start: Instant,
}

impl Burrow {
    /// The sidebar view sits in the window's layout while it is shown (and in fullscreen, where
    /// the layout hides it), and in the overlay while it is hidden.
    pub(super) fn floats(&self) -> bool {
        !self.sidebar_visible && !self.fullscreen && self.window.is_some()
    }

    /// Moves the sidebar view to where it belongs now: the overlay or the layout. `shown` is how
    /// much of it a new overlay shows at first.
    pub(super) fn place_sidebar(&mut self, shown: i32) {
        match (self.floats(), self.float.is_some()) {
            (true, false) => {
                self.float_sidebar(shown);
            }
            (false, true) => self.dock_sidebar(),
            _ => {}
        }
    }

    fn float_sidebar(&mut self, shown: i32) -> Option<()> {
        let window = self.window.clone()?;
        let mut view = View::from(&self.sidebar);
        window.remove_child_view(Some(&mut view));
        let holder = panel_create(None)?;
        holder.add_child_view(Some(&mut view));
        let overlay = window.add_overlay_view(Some(&mut View::from(&holder)), DockingMode::CUSTOM, 1)?;
        self.float = Some(Float { overlay, holder, shown, slide: None, generation: 0 });
        self.repaint_window();
        self.layout_window();
        self.layout_float();
        if let Some(f) = &self.float {
            f.overlay.set_visible(1);
        }
        // Moving the live sidebar into the overlay takes the keyboard, while the window still
        // thinks the page has it, so asking for the page does nothing. Once the overlay is up,
        // focus the content panel (in the window, so shortcuts work again), then the page's
        // browser directly. The panel stays focusable: making it unfocusable while focused, or
        // focusing the page's view from it, moves focus into the page as if by Tab, which
        // focuses its first link.
        task::post_ui_delayed(50, || {
            super::with(|b| {
                let content = View::from(&b.content);
                content.set_focusable(1);
                content.request_focus();
                b.with_active_browser(|br| {
                    if let Some(h) = br.host() {
                        h.set_focus(1);
                    }
                });
            });
        });
        Some(())
    }

    fn dock_sidebar(&mut self) {
        let Some(f) = self.float.take() else { return };
        let mut view = View::from(&self.sidebar);
        f.holder.remove_child_view(Some(&mut view));
        f.overlay.destroy();
        if let Some(w) = &self.window {
            w.add_child_view_at(Some(&mut view), 0);
        }
        self.repaint_window();
        self.layout_window();
    }

    /// Moving the sidebar resets the window's color (the margins around the card) to the default,
    /// a moment later; paint it again after that.
    fn repaint_window(&self) {
        task::post_ui_delayed(0, || {
            super::with(|b| b.apply_theme());
        });
    }

    /// The overlay covers the left edge, full height, as wide as the part that shows.
    pub(super) fn layout_float(&self) {
        let (Some(f), Some(w)) = (&self.float, &self.window) else { return };
        let height = View::from(w).bounds().height;
        f.overlay.set_bounds(Some(&Rect { x: 0, y: 0, width: f.shown.max(1), height }));
    }

    /// The sidebar's width changed (dragging its edge while it peeks out).
    pub(super) fn resize_float(&mut self) {
        let to = self.float_target();
        if let Some(f) = &mut self.float
            && f.slide.is_none()
        {
            f.shown = to;
        }
        self.layout_float();
    }

    /// The part of the floating sidebar that shows when it has finished sliding.
    fn float_target(&self) -> i32 {
        if self.peeking { self.state.settings.sidebar_width as i32 } else { consts::SIDEBAR_COLLAPSED_WIDTH }
    }

    /// Slides the floating sidebar out (peeking) or back to the edge strip.
    pub(super) fn slide_float(&mut self) {
        let to = self.float_target();
        let Some(f) = &mut self.float else { return };
        f.generation += 1;
        f.slide = Some(Slide { from: f.shown, to, start: Instant::now() });
        let generation = f.generation;
        self.slide_frame(generation);
    }

    fn slide_frame(&mut self, generation: u64) {
        let Some(f) = &mut self.float else { return };
        let Some(s) = f.slide.as_ref().filter(|_| f.generation == generation) else { return };
        let t = (s.start.elapsed().as_secs_f64() * 1000.0 / SLIDE_MS).min(1.0);
        // Ease out: fast at first, settling at the end.
        let eased = 1.0 - (1.0 - t).powi(3);
        f.shown = s.from + ((s.to - s.from) as f64 * eased).round() as i32;
        let done = t >= 1.0;
        if done {
            f.slide = None;
        }
        self.layout_float();
        if done {
            // The page draws the collapsed strip only once the sidebar is back at the edge.
            self.push_state();
        } else {
            task::post_ui_delayed(FRAME_MS, move || {
                super::with(|b| b.slide_frame(generation));
            });
        }
    }

    /// The hidden sidebar is fully back at its edge strip (the page then draws only the strip).
    pub(super) fn float_at_rest(&self) -> bool {
        self.float.as_ref().is_none_or(|f| f.slide.is_none() && f.shown <= consts::SIDEBAR_COLLAPSED_WIDTH)
    }
}
