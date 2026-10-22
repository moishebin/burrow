//! Peek (D-022): a page shown in a card floating over the content area, for links that would
//! open a new tab and for links from pinned tabs that leave their site. It lives in an overlay
//! of the one window, never a window of its own. "Open as Tab" moves its browser into a tab.

use std::sync::atomic::{AtomicI32, Ordering};

use burrow_core::{Id, Location, Section};
use cef::*;

use super::{cef, Burrow};
use crate::{clients, task, views};

/// The Peek's browser id, `i32::MAX` while it isn't known yet, or -1 with no Peek. The key handler
/// reads it to close the Peek on Escape.
static PEEK_BROWSER: AtomicI32 = AtomicI32::new(-1);

pub fn is_peek_browser(browser_id: i32) -> bool {
    browser_id >= 0 && PEEK_BROWSER.load(Ordering::Relaxed) == browser_id
}

pub const BUTTON_TITLE: i32 = 1;
pub const BUTTON_EXPAND: i32 = 2;
pub const BUTTON_CLOSE: i32 = 3;

/// Dims the page around the Peek (ARGB).
const DIM: u32 = 0x4D1E1A2E;
const HEADER_HEIGHT: i32 = 34;

pub(super) struct Peek {
    overlay: OverlayController,
    card: Panel,
    view: BrowserView,
    title: LabelButton,
    browser_id: Option<i32>,
    pub(super) profile_id: Id,
    /// The tab the link came from; "Open as Tab" puts the new tab in its space.
    source_tab: Option<Id>,
    pub(super) url: String,
}

fn button(text: &str, id: i32, tooltip: &str) -> Option<LabelButton> {
    let mut delegate = views::PeekButtonDelegate::new();
    let b = label_button_create(Some(&mut delegate), Some(&cef(text)))?;
    View::from(&b).set_id(id);
    // The page, not a header button, must get the keyboard when the Peek opens.
    View::from(&b).set_focusable(0);
    b.set_tooltip_text(Some(&cef(tooltip)));
    b.set_enabled_text_colors(0xFF1F1D2B);
    Some(b)
}

impl Burrow {
    pub(super) fn peek_browser_id(&mut self) -> Option<i32> {
        let p = self.peek.as_mut()?;
        if p.browser_id.is_none() {
            p.browser_id = p.view.browser().map(|b| b.identifier());
            if let Some(id) = p.browser_id {
                PEEK_BROWSER.store(id, Ordering::Relaxed);
            }
        }
        p.browser_id
    }

    pub(super) fn is_peek(&mut self, browser_id: i32) -> bool {
        self.peek_browser_id() == Some(browser_id)
    }

    /// Opens `url` in a Peek, in the profile of `source_tab` (or the active space).
    pub fn open_peek(&mut self, url: String, source_tab: Option<Id>) {
        self.close_peek();
        let profile_id = source_tab
            .as_deref()
            .and_then(|t| self.state.profile_of_tab(t))
            .unwrap_or(&self.state.active_space().profile_id)
            .clone();
        let Some(profile) = self.profile(&profile_id) else { return };
        if !profile.ready {
            return self.open_link_as_tab(url, false);
        }
        let mut context = Some(profile.context.clone());
        let mut client = clients::ContentClient::new();
        let mut delegate = views::TabViewDelegate::new();
        let Some(view) = browser_view_create(
            Some(&mut client),
            Some(&cef(&url)),
            Some(&BrowserSettings { background_color: 0xFFFFFFFF, ..Default::default() }),
            None,
            context.as_mut(),
            Some(&mut delegate),
        ) else {
            return eprintln!("Burrow: could not create a peek for {url}");
        };
        self.show_peek(view, profile_id, source_tab, url);
    }

    /// A `window.open` popup (e.g. a sign-in window) shown as a Peek. Its opener keeps working.
    pub(super) fn adopt_popup_as_peek(&mut self, view: BrowserView, source_tab: Option<Id>) -> bool {
        self.close_peek();
        let profile_id = source_tab
            .as_deref()
            .and_then(|t| self.state.profile_of_tab(t))
            .unwrap_or(&self.state.active_space().profile_id)
            .clone();
        self.show_peek(view, profile_id, source_tab, String::new());
        self.peek.is_some()
    }

    fn show_peek(&mut self, view: BrowserView, profile_id: Id, source_tab: Option<Id>, url: String) -> Option<()> {
        self.close_extension_popup();
        let window = self.window.clone()?;
        let frame = panel_create(None)?;
        // The card's 1px inset shows its color as a border around the page.
        let card = panel_create(None)?;
        card.set_background_color(0xFFB9B4CC);
        let header = panel_create(None)?;
        header.set_background_color(0xFFF4F3F8);

        let title = button(if url.is_empty() { "Loading…" } else { &url }, BUTTON_TITLE, "Open as tab")?;
        title.set_horizontal_alignment(HorizontalAlignment::LEFT);
        let expand = button("Open as Tab", BUTTON_EXPAND, "Keep this page as a tab (Ctrl+Enter)")?;
        let close = button("✕", BUTTON_CLOSE, "Close (Esc)")?;
        let header_layout = header.set_to_box_layout(Some(&BoxLayoutSettings {
            horizontal: 1,
            inside_border_insets: Insets { top: 0, left: 8, bottom: 0, right: 4 },
            between_child_spacing: 4,
            cross_axis_alignment: AxisAlignment::CENTER,
            minimum_cross_axis_size: HEADER_HEIGHT,
            ..Default::default()
        }));
        for b in [&title, &expand, &close] {
            header.add_child_view(Some(&mut View::from(b)));
        }
        if let Some(l) = header_layout {
            l.set_flex_for_view(Some(&mut View::from(&title)), 1);
        }

        let card_layout = card.set_to_box_layout(Some(&BoxLayoutSettings {
            horizontal: 0,
            inside_border_insets: Insets { top: 1, left: 1, bottom: 1, right: 1 },
            cross_axis_alignment: AxisAlignment::STRETCH,
            ..Default::default()
        }));
        card.add_child_view(Some(&mut View::from(&header)));
        card.add_child_view(Some(&mut View::from(&view)));
        if let Some(l) = card_layout {
            l.set_flex_for_view(Some(&mut View::from(&view)), 1);
        }
        frame.set_to_fill_layout();
        frame.add_child_view(Some(&mut View::from(&card)));
        let overlay = window.add_overlay_view(Some(&mut View::from(&frame)), DockingMode::CUSTOM, 1)?;
        // CEF resets the colors of views it puts in an overlay; set them again.
        frame.set_background_color(DIM);
        card.set_background_color(0xFFB9B4CC);
        header.set_background_color(0xFFF4F3F8);
        // Open, browser id not known yet (popups and new browsers get theirs later).
        PEEK_BROWSER.store(i32::MAX, Ordering::Relaxed);
        self.peek = Some(Peek { overlay, card, view, title, browser_id: None, profile_id, source_tab, url });
        self.layout_peek();
        if let Some(p) = &self.peek {
            p.overlay.set_visible(1);
        }
        // The keyboard stays with the page behind: an overlay can't take focus until it is
        // clicked, and focusing into it before that loses the keyboard altogether. Escape and
        // Burrow's shortcuts work from there; clicking the Peek moves typing into it.
        task::post_ui_delayed(50, || {
            super::with(|b| b.focus_content());
        });
        self.push_state();
        Some(())
    }

    /// Fits the Peek over the content area, leaving a margin of the page around the card.
    pub(super) fn layout_peek(&self) {
        let Some(p) = &self.peek else { return };
        let area = View::from(&self.content).bounds();
        let inset_x = (area.width as f32 * 0.07).max(24.0) as i32;
        let inset_y = (area.height as f32 * 0.04).max(16.0) as i32;
        p.overlay.set_bounds(Some(&area));
        if let Some(frame) = p.overlay.contents_view() {
            frame.set_insets(Some(&Insets { top: inset_y, left: inset_x, bottom: inset_y, right: inset_x }));
            frame.invalidate_layout();
        }
        View::from(&p.card).invalidate_layout();
    }

    pub fn close_peek(&mut self) {
        let id = self.peek_browser_id();
        let Some(p) = self.peek.take() else { return };
        PEEK_BROWSER.store(-1, Ordering::Relaxed);
        if let Some(host) = p.view.browser().and_then(|b| b.host()) {
            self.dying_peeks.extend(id);
            host.close_browser(1);
        }
        p.overlay.destroy();
        self.focus_content();
        self.push_state();
    }

    /// The Peek's page closed itself (e.g. a sign-in popup finished).
    pub(super) fn on_peek_closed(&mut self) {
        if let Some(p) = self.peek.take() {
            PEEK_BROWSER.store(-1, Ordering::Relaxed);
            p.overlay.destroy();
            self.focus_content();
            self.push_state();
        }
    }

    /// "Open as Tab": the Peek's page becomes the top tab of Today, without reloading.
    pub fn expand_peek(&mut self) {
        let Some(p) = self.peek.take() else { return };
        PEEK_BROWSER.store(-1, Ordering::Relaxed);
        let space_id = p
            .source_tab
            .as_deref()
            .and_then(|t| self.state.space_of(t))
            .filter(|s| s.profile_id == p.profile_id)
            .or_else(|| self.state.spaces.iter().find(|s| s.profile_id == p.profile_id))
            .map(|s| s.id.clone());
        let url = p.view.browser().and_then(|b| b.main_frame()).map(|f| CefString::from(&f.url()).to_string());
        let url = url.filter(|u| !u.is_empty()).unwrap_or(p.url.clone());
        let to = Location { space_id: space_id.clone(), section: Section::Today, parent: None, index: Some(0) };
        let Ok((id, _)) = self.state.open_tab(url, Some(to), true) else {
            p.overlay.destroy();
            return;
        };
        if let Some(t) = self.state.tab_mut(&id) {
            t.title = CefString::from(&p.title.text()).to_string();
        }
        // Move the live browser out of the overlay and into the content area.
        p.card.remove_child_view(Some(&mut View::from(&p.view)));
        p.overlay.destroy();
        self.adopt_view(id, p.view);
        self.sync_view();
        self.changed();
    }

    /// A Peek is open (the key handler closes it on Escape from any page).
    pub fn peek_open() -> bool {
        PEEK_BROWSER.load(Ordering::Relaxed) >= 0
    }

    pub(super) fn on_peek_title(&mut self, title: &str) {
        if let Some(p) = &self.peek {
            p.title.set_text(Some(&cef(title)));
        }
    }

    pub(super) fn on_peek_address(&mut self, url: String) {
        if let Some(p) = &mut self.peek {
            if CefString::from(&p.title.text()).to_string() == "Loading…" {
                p.title.set_text(Some(&cef(&url)));
            }
            p.url = url;
        }
    }

    pub fn peek_button(&mut self, id: i32) {
        match id {
            BUTTON_TITLE | BUTTON_EXPAND => self.expand_peek(),
            BUTTON_CLOSE => self.close_peek(),
            _ => {}
        }
    }
}
