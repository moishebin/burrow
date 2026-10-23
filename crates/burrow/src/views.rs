//! CEF Views delegates: the main window, the sidebar view and tab views.

use cef::*;

use std::sync::atomic::{AtomicI32, Ordering};

use crate::{browser, clients, consts};

/// The sidebar's current width; changes when it is hidden (edge strip) or peeking.
static SIDEBAR_WIDTH: AtomicI32 = AtomicI32::new(consts::SIDEBAR_WIDTH);

pub fn set_sidebar_width(width: i32) {
    SIDEBAR_WIDTH.store(width, Ordering::Relaxed);
}

wrap_window_delegate! {
    pub struct MainWindowDelegate {}
    impl ViewDelegate {
        fn preferred_size(&self, _view: Option<&mut View>) -> Size {
            Size { width: 1400, height: 900 }
        }
    }
    impl PanelDelegate {}
    impl WindowDelegate {
        fn on_window_created(&self, window: Option<&mut Window>) {
            let Some(window) = window else { return };
            browser::with(|b| b.attach(window));
        }

        fn on_window_destroyed(&self, _window: Option<&mut Window>) {
            browser::with(|b| b.detach());
            quit_message_loop();
        }

        fn initial_show_state(&self, _window: Option<&mut Window>) -> ShowState {
            ShowState::MAXIMIZED
        }

        /// Alloy, like every view in it (D-010).
        fn window_runtime_style(&self) -> RuntimeStyle {
            RuntimeStyle::ALLOY
        }

        /// Arc-style: no title bar. The sidebar's toolbar is the drag area and holds the window
        /// buttons (D-013).
        fn is_frameless(&self, _window: Option<&mut Window>) -> ::std::os::raw::c_int {
            browser::with(|b| b.frameless()).unwrap_or(false) as _
        }

        fn can_resize(&self, _window: Option<&mut Window>) -> ::std::os::raw::c_int {
            1
        }

        fn can_maximize(&self, _window: Option<&mut Window>) -> ::std::os::raw::c_int {
            1
        }

        fn can_minimize(&self, _window: Option<&mut Window>) -> ::std::os::raw::c_int {
            1
        }

        fn on_window_bounds_changed(&self, _window: Option<&mut Window>, _new_bounds: Option<&Rect>) {
            browser::later(|b| b.on_window_resized());
        }

        /// Window-wide shortcuts: they work even when no page has keyboard focus.
        fn on_accelerator(&self, _window: Option<&mut Window>, command_id: ::std::os::raw::c_int) -> ::std::os::raw::c_int {
            match clients::shortcut_for_accelerator(command_id) {
                Some(s) => {
                    browser::later(move |b| b.shortcut(s));
                    1
                }
                None => 0,
            }
        }

        fn can_close(&self, _window: Option<&mut Window>) -> ::std::os::raw::c_int {
            browser::with(|b| b.request_close()).unwrap_or(true) as _
        }
    }
}

wrap_browser_view_delegate! {
    pub struct SidebarViewDelegate {}
    impl ViewDelegate {
        fn preferred_size(&self, _view: Option<&mut View>) -> Size {
            Size { width: SIDEBAR_WIDTH.load(Ordering::Relaxed), height: 600 }
        }
        fn minimum_size(&self, _view: Option<&mut View>) -> Size {
            Size { width: SIDEBAR_WIDTH.load(Ordering::Relaxed), height: 200 }
        }
    }
    impl BrowserViewDelegate {
        fn browser_runtime_style(&self) -> RuntimeStyle {
            RuntimeStyle::ALLOY
        }
    }
}

// The split view's divider: as wide as the gap between the cards (D-038).
wrap_browser_view_delegate! {
    pub struct DividerViewDelegate {}
    impl ViewDelegate {
        fn preferred_size(&self, _view: Option<&mut View>) -> Size {
            Size { width: consts::CARD_MARGIN, height: 100 }
        }
        fn minimum_size(&self, _view: Option<&mut View>) -> Size {
            Size { width: consts::CARD_MARGIN, height: 10 }
        }
    }
    impl BrowserViewDelegate {
        fn browser_runtime_style(&self) -> RuntimeStyle {
            RuntimeStyle::ALLOY
        }
    }
}

// The dialog overlay (D-039): sized by its overlay.
wrap_browser_view_delegate! {
    pub struct OverlayViewDelegate {}
    impl ViewDelegate {}
    impl BrowserViewDelegate {
        fn browser_runtime_style(&self) -> RuntimeStyle {
            RuntimeStyle::ALLOY
        }
    }
}

wrap_browser_view_delegate! {
    // `fills`: tabs and Peeks take the size their layout gives them. Their natural width would
    // skew a split view's shares, since a box layout shares out the difference from it (D-038).
    // Extension popups keep their natural (auto-resized) size.
    pub struct TabViewDelegate {
        fills: bool,
    }
    impl ViewDelegate {
        fn preferred_size(&self, _view: Option<&mut View>) -> Size {
            // An empty size means "the default".
            if self.fills { Size { width: 1, height: 1 } } else { Size::default() }
        }
    }
    impl BrowserViewDelegate {
        /// `window.open` popups and DevTools: popups become tabs, DevTools keeps its own window.
        fn on_popup_browser_view_created(
            &self,
            browser_view: Option<&mut BrowserView>,
            popup_browser_view: Option<&mut BrowserView>,
            is_devtools: ::std::os::raw::c_int,
        ) -> ::std::os::raw::c_int {
            if is_devtools != 0 {
                return 0;
            }
            let Some(popup) = popup_browser_view else { return 0 };
            let popup = popup.clone();
            let opener = browser_view.and_then(|v| v.browser()).map(|b| b.identifier());
            // Adopt synchronously: CEF needs the view in a window before this returns.
            browser::with(|b| b.adopt_popup(opener, popup)).map_or(0, |_| 1)
        }

        /// A window holds at most one Chrome-style BrowserView, and Burrow shows many (D-010).
        fn browser_runtime_style(&self) -> RuntimeStyle {
            RuntimeStyle::ALLOY
        }
    }
}

// The top bar: the address and minimize, maximize and close; it opens while hovered (D-035, D-045).
wrap_button_delegate! {
    pub struct WindowButtonDelegate {}
    impl ViewDelegate {}
    impl ButtonDelegate {
        fn on_button_pressed(&self, button: Option<&mut Button>) {
            let Some(id) = button.map(|b| View::from(&*b).id()) else { return };
            browser::later(move |b| b.window_button(id));
        }

        fn on_button_state_changed(&self, button: Option<&mut Button>) {
            let Some(button) = button else { return };
            let id = View::from(&*button).id();
            let hovered = matches!(button.state(), ButtonState::HOVERED | ButtonState::PRESSED);
            browser::later(move |b| b.window_button_hover(id, hovered));
        }
    }
}

// The buttons in a Peek's header (D-022).
wrap_button_delegate! {
    pub struct PeekButtonDelegate {}
    impl ViewDelegate {}
    impl ButtonDelegate {
        fn on_button_pressed(&self, button: Option<&mut Button>) {
            let Some(id) = button.map(|b| View::from(&*b).id()) else { return };
            browser::later(move |b| b.peek_button(id));
        }
    }
}
