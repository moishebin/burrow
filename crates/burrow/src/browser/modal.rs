//! Dialogs over the whole window (D-039). The sidebar's dialogs (History, Extensions, Air Traffic
//! Control, …) used to open inside the sidebar, squeezed to its width. They now open centered
//! over the window, in two overlays of it (never a second window): a dimmed backdrop, a native
//! button that closes the dialog when clicked, and above it a card holding the sidebar's own page
//! in "modal" mode, where the same code builds the same dialog. The card takes the dialog's size
//! (CEF auto-resize). CEF paints pages in overlays opaque, so the page itself can't be the
//! backdrop. Both are made for each dialog and destroyed when it closes.

use std::sync::atomic::{AtomicBool, Ordering};

use cef::*;
use serde_json::Value;

use super::{cef, js_string, Burrow};
use crate::{clients, paths, task, views};

/// Dims the window behind a dialog (ARGB), like a Peek's surroundings.
const DIM: u32 = 0x521E1A2E;
const BORDER: u32 = 0xFFB9B4CC;
/// The backdrop's button id (`views::WindowButtonDelegate` reports presses by id).
pub const BACKDROP_BUTTON: i32 = 200;
const MIN: Size = Size { width: 200, height: 60 };
const MARGIN: i32 = 40;

/// A dialog is open. The key handler reads it: Escape closes the dialog from any page.
static OPEN: AtomicBool = AtomicBool::new(false);

pub fn dialog_open() -> bool {
    OPEN.load(Ordering::Relaxed)
}

pub(super) struct Modal {
    backdrop: OverlayController,
    card: OverlayController,
    view: BrowserView,
    /// Its page has loaded and said so; until then scripts wait in `pending`.
    ready: bool,
    pending: Vec<String>,
    /// The dialog page's own size.
    size: Size,
}

impl Burrow {
    /// Makes the overlays for one dialog, hidden until its page is ready. Like a Peek, every
    /// dialog gets new ones: an overlay that is hidden and shown again can take the keyboard
    /// away from every page (seen on Linux), so Esc stopped working.
    fn create_modal(&mut self, pending: String) -> Option<()> {
        let window = self.window.clone()?;
        let mut delegate = views::WindowButtonDelegate::new();
        let dim = label_button_create(Some(&mut delegate), Some(&cef("")))?;
        View::from(&dim).set_id(BACKDROP_BUTTON);
        View::from(&dim).set_focusable(0);
        let backdrop = window.add_overlay_view(Some(&mut View::from(&dim)), DockingMode::CUSTOM, 0)?;
        // CEF resets the colors of views it puts in an overlay (D-022).
        View::from(&dim).set_background_color(DIM);
        backdrop.set_visible(0);

        let mut client = clients::UiClient::new();
        let mut delegate = views::OverlayViewDelegate::new();
        let view = browser_view_create(
            Some(&mut client),
            Some(&cef(&format!("{}?modal", paths::ui_url("sidebar.html")))),
            Some(&BrowserSettings { background_color: 0xFFFFFFFF, ..Default::default() }),
            None,
            None,
            Some(&mut delegate),
        )?;
        let frame = panel_create(None)?;
        frame.set_to_fill_layout();
        frame.set_insets(Some(&Insets { top: 1, left: 1, bottom: 1, right: 1 }));
        frame.add_child_view(Some(&mut View::from(&view)));
        let card = window.add_overlay_view(Some(&mut View::from(&frame)), DockingMode::CUSTOM, 1)?;
        frame.set_background_color(BORDER);
        card.set_visible(0);
        self.modal = Some(Modal { backdrop, card, view, ready: false, pending: vec![pending], size: Size { width: 460, height: 300 } });
        Some(())
    }

    /// The backdrop covers the window; the card sits in the middle at the dialog's size.
    pub(super) fn layout_modal(&self) {
        let Some(m) = &self.modal else { return };
        let Some(w) = &self.window else { return };
        let area = View::from(w).bounds();
        m.backdrop.set_bounds(Some(&Rect { x: 0, y: 0, width: area.width, height: area.height }));
        let width = (m.size.width + 2).min(area.width - 2 * MARGIN).max(1);
        let height = (m.size.height + 2).min(area.height - 2 * MARGIN).max(1);
        m.card.set_bounds(Some(&Rect { x: (area.width - width) / 2, y: (area.height - height) / 2, width, height }));
    }

    /// Opens one of the sidebar's dialogs by name (`DIALOGS` in sidebar.js).
    pub(super) fn open_dialog(&mut self, name: &str, args: &Value) {
        self.close_modal();
        // Overlays made later would sit on top of the dialog.
        self.close_extension_popup();
        self.close_peek();
        let js = format!("window.burrow && burrow.showDialog({}, {})", js_string(name), args);
        if self.create_modal(js).is_none() {
            return;
        }
        OPEN.store(true, Ordering::Relaxed);
        self.layout_modal();
    }

    /// The card follows the dialog page's size, up to the window's.
    fn modal_auto_resize(&self) {
        let (Some(m), Some(w)) = (&self.modal, &self.window) else { return };
        let area = View::from(w).bounds();
        let max = Size { width: (area.width - 2 * MARGIN).max(MIN.width), height: (area.height - 2 * MARGIN).max(MIN.height) };
        if let Some(host) = m.view.browser().and_then(|b| b.host()) {
            host.set_auto_resize_enabled(1, Some(&MIN), Some(&max));
        }
    }

    pub fn on_modal_resized(&mut self, browser_id: i32, size: Size) {
        let Some(m) = self.modal.as_mut().filter(|m| m.view.browser().is_some_and(|b| b.identifier() == browser_id)) else {
            return;
        };
        m.size = size;
        self.layout_modal();
    }

    pub(super) fn close_modal(&mut self) {
        let Some(m) = self.modal.take() else { return };
        OPEN.store(false, Ordering::Relaxed);
        if let Some(b) = m.view.browser() {
            // The window waits for it to finish closing, like a Peek's.
            self.dying_peeks.push(b.identifier());
            if let Some(host) = b.host() {
                host.close_browser(1);
            }
        }
        m.card.destroy();
        m.backdrop.destroy();
        self.focus_content();
    }

    /// The dialog's page loaded: send it the state, build the dialog, then show it.
    pub(super) fn on_modal_ready(&mut self) {
        let Some(m) = &mut self.modal else { return };
        m.ready = true;
        let pending = std::mem::take(&mut m.pending);
        self.modal_auto_resize();
        // `push_state` and these run later, in this order.
        self.push_state();
        super::later(move |b| {
            for js in pending {
                b.run_in_modal(js);
            }
        });
        task::post_ui_delayed(40, || {
            super::with(|b| {
                if let Some(m) = &b.modal {
                    m.backdrop.set_visible(1);
                    m.card.set_visible(1);
                }
            });
        });
        // Showing an overlay takes the keyboard; the page gets it back, so Esc closes the
        // dialog. Focusing into an overlay before it is clicked loses the keyboard altogether
        // (D-022's note on Peeks); a click in the dialog moves typing there (`focus_modal`).
        task::post_ui_delayed(90, || {
            super::with(|b| b.focus_content());
        });
    }

    pub(super) fn run_in_modal(&mut self, js: String) {
        let Some(m) = &mut self.modal else { return };
        if !m.ready {
            return m.pending.push(js);
        }
        if let Some(frame) = m.view.browser().and_then(|b| b.main_frame()) {
            frame.execute_java_script(Some(&cef(&js)), Some(&CefString::from(&frame.url())), 0);
        }
    }

    pub(super) fn focus_modal(&mut self) {
        let Some(m) = &self.modal else { return };
        m.view.request_focus();
        if let Some(host) = m.view.browser().and_then(|b| b.host()) {
            host.set_focus(1);
        }
        self.run_in_modal_now("window.burrow && burrow.focusDialog()");
    }

    fn run_in_modal_now(&self, js: &str) {
        if let Some(frame) = self.modal.as_ref().filter(|m| m.ready).and_then(|m| m.view.browser()).and_then(|b| b.main_frame()) {
            frame.execute_java_script(Some(&cef(js)), Some(&CefString::from(&frame.url())), 0);
        }
    }

    pub(super) fn modal_open(&self) -> bool {
        self.modal.is_some()
    }

    /// For shutdown: it closes with the window.
    pub(super) fn modal_browser(&self) -> Option<Browser> {
        self.modal.as_ref().and_then(|m| m.view.browser())
    }
}
