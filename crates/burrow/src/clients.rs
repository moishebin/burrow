//! CEF clients: one for web content tabs, one for the sidebar. Callbacks only translate CEF
//! events and post them to the controller (see `browser::later`).

use cef::*;

use burrow_core::links::LinkKind;

use crate::{
    browser::{self, Shortcut},
    consts,
};

fn id_of(browser: Option<&mut Browser>) -> Option<i32> {
    browser.map(|b| b.identifier())
}

fn string(s: Option<&CefString>) -> String {
    s.map(|s| s.to_string()).unwrap_or_default()
}

#[cfg(target_os = "windows")]
type OsEvent = sys::MSG;
#[cfg(not(target_os = "windows"))]
type OsEvent = sys::XEvent;

// Modifier bits of `KeyEvent::modifiers` (cef_event_flags_t).
const SHIFT: u32 = 1 << 1;
const CTRL: u32 = 1 << 2;
const ALT: u32 = 1 << 3;

// Windows virtual-key codes.
const VK_TAB: i32 = 0x09;
const VK_ESCAPE: i32 = 0x1B;
const VK_LEFT: i32 = 0x25;
const VK_RIGHT: i32 = 0x27;
const VK_F5: i32 = 0x74;
const VK_F11: i32 = 0x7A;
const VK_F12: i32 = 0x7B;
const VK_OEM_PLUS: i32 = 0xBB;
const VK_OEM_MINUS: i32 = 0xBD;

const fn key(c: char) -> i32 {
    c as i32
}

/// Burrow's keyboard shortcuts, Arc-compatible where Arc has one: (action, key code, modifiers).
/// Used by the browsers' key handler and as window accelerators, so shortcuts also work when no
/// page has focus (e.g. after closing the last tab).
pub const SHORTCUTS: &[(Shortcut, i32, u32)] = &[
    (Shortcut::NewTab, key('T'), CTRL),
    (Shortcut::NewTab, key('N'), CTRL),
    // Never let Chrome open an incognito *window*.
    (Shortcut::NewTab, key('N'), CTRL | SHIFT),
    (Shortcut::FocusAddress, key('L'), CTRL),
    (Shortcut::CloseTab, key('W'), CTRL),
    (Shortcut::Reload, key('R'), CTRL),
    (Shortcut::Reload, VK_F5, 0),
    (Shortcut::Back, VK_LEFT, ALT),
    (Shortcut::Forward, VK_RIGHT, ALT),
    (Shortcut::Space(0), key('1'), CTRL),
    (Shortcut::Space(1), key('2'), CTRL),
    (Shortcut::Space(2), key('3'), CTRL),
    (Shortcut::Space(3), key('4'), CTRL),
    (Shortcut::Space(4), key('5'), CTRL),
    (Shortcut::Space(5), key('6'), CTRL),
    (Shortcut::Space(6), key('7'), CTRL),
    (Shortcut::Space(7), key('8'), CTRL),
    (Shortcut::Space(8), key('9'), CTRL),
    (Shortcut::CycleTab(1), VK_TAB, CTRL),
    (Shortcut::CycleTab(-1), VK_TAB, CTRL | SHIFT),
    (Shortcut::ReopenClosed, key('T'), CTRL | SHIFT),
    (Shortcut::Find, key('F'), CTRL),
    (Shortcut::TogglePin, key('D'), CTRL),
    (Shortcut::ToggleSidebar, key('S'), CTRL),
    (Shortcut::DevTools, VK_F12, 0),
    (Shortcut::DevTools, key('I'), CTRL | SHIFT),
    (Shortcut::SidebarDevTools, key('I'), CTRL | ALT),
    (Shortcut::Zoom(1), VK_OEM_PLUS, CTRL),
    (Shortcut::Zoom(-1), VK_OEM_MINUS, CTRL),
    (Shortcut::Zoom(0), key('0'), CTRL),
    (Shortcut::Fullscreen, VK_F11, 0),
    (Shortcut::CopyUrl, key('C'), CTRL | SHIFT),
    (Shortcut::Print, key('P'), CTRL),
    (Shortcut::History, key('H'), CTRL),
    (Shortcut::History, key('Y'), CTRL),
];

fn shortcut_for(event: &KeyEvent) -> Option<Shortcut> {
    if event.type_ != KeyEventType::RAWKEYDOWN && event.type_ != KeyEventType::KEYDOWN {
        return None;
    }
    let m = event.modifiers & (SHIFT | CTRL | ALT);
    SHORTCUTS
        .iter()
        .find(|(_, k, mods)| *k == event.windows_key_code && *mods == m)
        .map(|(s, _, _)| *s)
}

/// Window accelerator ids are offsets into `SHORTCUTS`.
pub const ACCELERATOR_BASE: i32 = 27000;

pub fn register_accelerators(window: &Window) {
    for (i, (_, k, m)) in SHORTCUTS.iter().enumerate() {
        window.set_accelerator(
            ACCELERATOR_BASE + i as i32,
            *k,
            (m & SHIFT != 0) as i32,
            (m & CTRL != 0) as i32,
            (m & ALT != 0) as i32,
            0,
        );
    }
}

pub fn shortcut_for_accelerator(id: i32) -> Option<Shortcut> {
    SHORTCUTS.get(usize::try_from(id - ACCELERATOR_BASE).ok()?).map(|(s, _, _)| *s)
}

wrap_keyboard_handler! {
    struct Keys {}
    impl KeyboardHandler {
        fn on_pre_key_event(
            &self,
            _browser: Option<&mut Browser>,
            event: Option<&KeyEvent>,
            _os_event: Option<&mut OsEvent>,
            _is_keyboard_shortcut: Option<&mut ::std::os::raw::c_int>,
        ) -> ::std::os::raw::c_int {
            // Escape closes a Peek or leaves a fullscreen page (pages keep Escape otherwise).
            if let (Some(e), Some(b)) = (event, _browser.as_ref())
                && e.windows_key_code == VK_ESCAPE
                && e.type_ == KeyEventType::RAWKEYDOWN
                && e.modifiers & (SHIFT | CTRL | ALT) == 0
                && (browser::is_peek_browser(b.identifier())
                    || browser::is_popup_browser(b.identifier())
                    || browser::Burrow::peek_open()
                    || browser::page_fullscreen())
            {
                browser::later(|b| b.shortcut(Shortcut::Escape));
                return 1;
            }
            match event.and_then(shortcut_for) {
                Some(s) => {
                    browser::later(move |b| b.shortcut(s));
                    1
                }
                None => 0,
            }
        }
    }
}

/// Chrome's own tab/window commands (chrome_command_ids.h, 34000–34031). Burrow has one window
/// and its own tabs, so Chrome must never act on these.
fn is_window_or_tab_command(id: i32) -> bool {
    const IDC_FULLSCREEN: i32 = 34030;
    (34000..=34031).contains(&id) && id != IDC_FULLSCREEN
}

wrap_command_handler! {
    struct Commands {}
    impl CommandHandler {
        fn on_chrome_command(
            &self,
            _browser: Option<&mut Browser>,
            command_id: ::std::os::raw::c_int,
            _disposition: WindowOpenDisposition,
        ) -> ::std::os::raw::c_int {
            is_window_or_tab_command(command_id) as _
        }
    }
}

// ---- Content tabs ---------------------------------------------------------------------------

wrap_client! {
    pub struct ContentClient {}
    impl Client {
        fn display_handler(&self) -> Option<DisplayHandler> { Some(ContentDisplay::new()) }
        fn load_handler(&self) -> Option<LoadHandler> { Some(ContentLoad::new()) }
        fn life_span_handler(&self) -> Option<LifeSpanHandler> { Some(ContentLifeSpan::new()) }
        fn keyboard_handler(&self) -> Option<KeyboardHandler> { Some(Keys::new()) }
        fn command_handler(&self) -> Option<CommandHandler> { Some(Commands::new()) }
        fn download_handler(&self) -> Option<DownloadHandler> { Some(Downloads::new()) }
        fn permission_handler(&self) -> Option<PermissionHandler> { Some(Permissions::new()) }
        fn context_menu_handler(&self) -> Option<ContextMenuHandler> { Some(PageMenu::new()) }
        fn find_handler(&self) -> Option<FindHandler> { Some(Find::new()) }
        fn request_handler(&self) -> Option<RequestHandler> { Some(Navigation::new()) }
        fn focus_handler(&self) -> Option<FocusHandler> { Some(Focus::new()) }
    }
}

wrap_request_handler! {
    struct Navigation {}
    impl RequestHandler {
        /// A link click in a pinned tab that leaves its site opens in a Peek instead (D-022).
        /// CEF needs the answer now, so this is one of the few synchronous `browser::with` calls.
        fn on_before_browse(
            &self,
            browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            request: Option<&mut Request>,
            user_gesture: ::std::os::raw::c_int,
            is_redirect: ::std::os::raw::c_int,
        ) -> ::std::os::raw::c_int {
            let (Some(browser), Some(frame), Some(request)) = (browser, frame, request) else { return 0 };
            let source = request.transition_type().get_raw() & TransitionType::SOURCE_MASK.get_raw();
            if user_gesture == 0 || is_redirect != 0 || frame.is_main() == 0 || source != TransitionType::LINK.get_raw() {
                return 0;
            }
            let to = CefString::from(&request.url()).to_string();
            let from = CefString::from(&frame.url()).to_string();
            let id = browser.identifier();
            browser::with(|b| b.intercept_navigation(id, &from, &to)).unwrap_or(false) as _
        }
    }
}

wrap_focus_handler! {
    struct Focus {}
    impl FocusHandler {
        fn on_got_focus(&self, browser: Option<&mut Browser>) {
            let Some(id) = id_of(browser) else { return };
            browser::later(move |b| b.on_tab_focused(id));
        }
    }
}

// ---- Permissions (camera, microphone, notifications, …) ---------------------------------------

wrap_permission_handler! {
    struct Permissions {}
    impl PermissionHandler {
        fn on_request_media_access_permission(
            &self,
            browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            requesting_origin: Option<&CefString>,
            requested_permissions: u32,
            callback: Option<&mut MediaAccessCallback>,
        ) -> ::std::os::raw::c_int {
            let (Some(id), Some(cb)) = (id_of(browser), callback) else { return 0 };
            let origin = string(requesting_origin);
            let cb = browser::PermissionCallback::Media(cb.clone(), requested_permissions);
            let names = browser::media_permission_names(requested_permissions);
            browser::with(|b| b.request_permission(id, None, origin, names, cb)).map_or(0, |_| 1)
        }

        fn on_show_permission_prompt(
            &self,
            browser: Option<&mut Browser>,
            prompt_id: u64,
            requesting_origin: Option<&CefString>,
            requested_permissions: u32,
            callback: Option<&mut PermissionPromptCallback>,
        ) -> ::std::os::raw::c_int {
            let (Some(id), Some(cb)) = (id_of(browser), callback) else { return 0 };
            let origin = string(requesting_origin);
            let cb = browser::PermissionCallback::Prompt(cb.clone());
            let names = browser::prompt_permission_names(requested_permissions);
            browser::with(|b| b.request_permission(id, Some(prompt_id), origin, names, cb)).map_or(0, |_| 1)
        }

        fn on_dismiss_permission_prompt(
            &self,
            _browser: Option<&mut Browser>,
            prompt_id: u64,
            _result: PermissionRequestResult,
        ) {
            browser::later(move |b| b.dismiss_permission(prompt_id));
        }
    }
}

// ---- Page context menu ------------------------------------------------------------------------

const MENU_OPEN_LINK: i32 = 26501;
const MENU_OPEN_LINK_BACKGROUND: i32 = 26502;
const MENU_COPY_LINK: i32 = 26503;
const MENU_SEARCH: i32 = 26504;
const MENU_INSPECT: i32 = 26505;
const MENU_OPEN_LINK_PEEK: i32 = 26506;

wrap_context_menu_handler! {
    struct PageMenu {}
    impl ContextMenuHandler {
        fn on_before_context_menu(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            params: Option<&mut ContextMenuParams>,
            model: Option<&mut MenuModel>,
        ) {
            let (Some(params), Some(model)) = (params, model) else { return };
            let label = |s: &str| CefString::from(s);
            if !CefString::from(&params.link_url()).to_string().is_empty() {
                model.insert_item_at(0, MENU_OPEN_LINK, Some(&label("Open Link in New Tab")));
                model.insert_item_at(1, MENU_OPEN_LINK_BACKGROUND, Some(&label("Open Link in Background Tab")));
                model.insert_item_at(2, MENU_OPEN_LINK_PEEK, Some(&label("Open Link in Peek")));
                model.insert_item_at(3, MENU_COPY_LINK, Some(&label("Copy Link Address")));
                model.insert_separator_at(4);
            }
            let selection = CefString::from(&params.selection_text()).to_string();
            if !selection.trim().is_empty() && params.is_editable() == 0 {
                let short: String = selection.trim().chars().take(24).collect();
                let ellipsis = if selection.trim().chars().count() > 24 { "…" } else { "" };
                model.add_separator();
                model.add_item(MENU_SEARCH, Some(&label(&format!("Search for “{short}{ellipsis}”"))));
            }
            model.add_separator();
            model.add_item(MENU_INSPECT, Some(&label("Inspect")));
        }

        fn on_context_menu_command(
            &self,
            browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            params: Option<&mut ContextMenuParams>,
            command_id: ::std::os::raw::c_int,
            _event_flags: EventFlags,
        ) -> ::std::os::raw::c_int {
            let Some(params) = params else { return 0 };
            let link = CefString::from(&params.link_url()).to_string();
            let from = browser.as_ref().map(|b| b.identifier());
            match command_id {
                MENU_OPEN_LINK => browser::later(move |b| b.open_link(from, link, LinkKind::ForegroundTab)),
                MENU_OPEN_LINK_BACKGROUND => browser::later(move |b| b.open_link(from, link, LinkKind::BackgroundTab)),
                MENU_OPEN_LINK_PEEK => browser::later(move |b| b.peek_link(from, link)),
                MENU_COPY_LINK => crate::clipboard::set(&link),
                MENU_SEARCH => {
                    let text = CefString::from(&params.selection_text()).to_string();
                    browser::later(move |b| b.search_in_new_tab(&text));
                }
                MENU_INSPECT => {
                    let at = Point { x: params.xcoord(), y: params.ycoord() };
                    if let Some(host) = browser.and_then(|b| b.host()) {
                        host.show_dev_tools(None, None, None, Some(&at));
                    }
                }
                _ => return 0,
            }
            1
        }
    }
}

// ---- Find in page -----------------------------------------------------------------------------

wrap_find_handler! {
    struct Find {}
    impl FindHandler {
        fn on_find_result(
            &self,
            _browser: Option<&mut Browser>,
            _identifier: ::std::os::raw::c_int,
            count: ::std::os::raw::c_int,
            _selection_rect: Option<&Rect>,
            active_match_ordinal: ::std::os::raw::c_int,
            _final_update: ::std::os::raw::c_int,
        ) {
            browser::later(move |b| b.on_find_result(count, active_match_ordinal));
        }
    }
}

wrap_display_handler! {
    struct ContentDisplay {}
    impl DisplayHandler {
        fn on_title_change(&self, browser: Option<&mut Browser>, title: Option<&CefString>) {
            let (Some(id), title) = (id_of(browser), string(title)) else { return };
            browser::later(move |b| b.on_title(id, title));
        }

        fn on_address_change(&self, browser: Option<&mut Browser>, frame: Option<&mut Frame>, url: Option<&CefString>) {
            if frame.is_none_or(|f| f.is_main() == 0) {
                return;
            }
            let (Some(id), url) = (id_of(browser), string(url)) else { return };
            browser::later(move |b| b.on_address(id, url));
        }

        fn on_favicon_urlchange(&self, browser: Option<&mut Browser>, icon_urls: Option<&mut CefStringList>) {
            let Some(id) = id_of(browser) else { return };
            let Some(url) = icon_urls.and_then(|l| l.clone().into_iter().next()) else { return };
            browser::later(move |b| b.on_favicon(id, url));
        }

        /// Messages from extensions (D-032): Burrow's helper names a tab's extension id, and an
        /// extension popup asks Burrow to open a tab. Only the popup's own page can log to its
        /// console; web pages can't reach it.
        fn on_console_message(
            &self,
            browser: Option<&mut Browser>,
            _level: LogSeverity,
            message: Option<&CefString>,
            _source: Option<&CefString>,
            _line: ::std::os::raw::c_int,
        ) -> ::std::os::raw::c_int {
            let Some(id) = id_of(browser) else { return 0 };
            let msg = string(message);
            if let Some(tab) = browser::helper_tab_id(&msg) {
                browser::later(move |b| b.on_extension_tab_id(id, tab));
                return 1;
            }
            if !browser::is_popup_browser(id) {
                return 0;
            }
            let Some(json) = msg.strip_prefix(browser::POPUP_PREFIX) else { return 0 };
            let json = json.to_string();
            browser::later(move |b| b.on_extension_popup_message(id, &json));
            1
        }

        /// An extension popup's page wants another size (auto-resize is on only for popups).
        fn on_auto_resize(&self, browser: Option<&mut Browser>, new_size: Option<&Size>) -> ::std::os::raw::c_int {
            let (Some(id), Some(size)) = (id_of(browser), new_size) else { return 0 };
            if !browser::is_popup_browser(id) {
                return 0;
            }
            let size = size.clone();
            browser::later(move |b| b.on_extension_popup_resized(id, size));
            1
        }

        /// A page (usually a video) entered or left fullscreen. Burrow makes the window
        /// fullscreen and hides the sidebar for it.
        fn on_fullscreen_mode_change(&self, browser: Option<&mut Browser>, fullscreen: ::std::os::raw::c_int) {
            let Some(id) = id_of(browser) else { return };
            browser::later(move |b| b.on_page_fullscreen(id, fullscreen != 0));
        }
    }
}

wrap_load_handler! {
    struct ContentLoad {}
    impl LoadHandler {
        fn on_loading_state_change(
            &self,
            browser: Option<&mut Browser>,
            is_loading: ::std::os::raw::c_int,
            can_go_back: ::std::os::raw::c_int,
            can_go_forward: ::std::os::raw::c_int,
        ) {
            let Some(id) = id_of(browser) else { return };
            browser::later(move |b| b.on_loading(id, is_loading != 0, can_go_back != 0, can_go_forward != 0));
        }
    }
}

wrap_life_span_handler! {
    struct ContentLifeSpan {}
    impl LifeSpanHandler {
        fn on_before_popup(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            _popup_id: ::std::os::raw::c_int,
            target_url: Option<&CefString>,
            _target_frame_name: Option<&CefString>,
            target_disposition: WindowOpenDisposition,
            _user_gesture: ::std::os::raw::c_int,
            _popup_features: Option<&PopupFeatures>,
            _window_info: Option<&mut WindowInfo>,
            _client: Option<&mut Option<Client>>,
            _settings: Option<&mut BrowserSettings>,
            _extra_info: Option<&mut Option<DictionaryValue>>,
            _no_javascript_access: Option<&mut ::std::os::raw::c_int>,
        ) -> ::std::os::raw::c_int {
            let url = string(target_url);
            let d = target_disposition;
            let from = _browser.map(|b| b.identifier());
            // Links that want a tab: we open the tab (or Peek) ourselves and cancel Chrome's.
            // target=_blank asks for a foreground tab; Shift+click asks for a window.
            let kind = match d {
                WindowOpenDisposition::NEW_FOREGROUND_TAB => Some(LinkKind::NewTab),
                WindowOpenDisposition::NEW_BACKGROUND_TAB => Some(LinkKind::BackgroundTab),
                WindowOpenDisposition::SINGLETON_TAB
                | WindowOpenDisposition::NEW_WINDOW
                | WindowOpenDisposition::OFF_THE_RECORD => Some(LinkKind::ForegroundTab),
                _ => None,
            };
            if let Some(kind) = kind {
                browser::later(move |b| b.open_link(from, url, kind));
                return 1;
            }
            // Real popups (window.open) keep their opener; TabViewDelegate turns them into tabs.
            0
        }

        /// Closing one tab must not close the window: CEF's default (0) for a Views-hosted browser
        /// closes the parent window. Returning 1 lets just this browser go once its view is
        /// removed. While the whole window is closing, the default is what we want (D-010).
        fn do_close(&self, browser: Option<&mut Browser>) -> ::std::os::raw::c_int {
            if browser::is_closing() {
                return 0;
            }
            // A page closing itself (window.close()): remove its view so the browser can go.
            // When Burrow closed it, the tab or Peek is already gone and this does nothing.
            if let Some(id) = id_of(browser) {
                browser::later(move |b| b.on_close_requested(id));
            }
            1
        }

        fn on_before_close(&self, browser: Option<&mut Browser>) {
            let Some(id) = id_of(browser) else { return };
            browser::later(move |b| b.on_browser_closed(id));
        }
    }
}

wrap_download_handler! {
    struct Downloads {}
    impl DownloadHandler {
        // Implemented explicitly: unimplemented callbacks may read as "no" (D-010).
        fn can_download(
            &self,
            _browser: Option<&mut Browser>,
            _url: Option<&CefString>,
            _request_method: Option<&CefString>,
        ) -> ::std::os::raw::c_int {
            1
        }

        fn on_before_download(
            &self,
            _browser: Option<&mut Browser>,
            _download_item: Option<&mut DownloadItem>,
            suggested_name: Option<&CefString>,
            callback: Option<&mut BeforeDownloadCallback>,
        ) -> ::std::os::raw::c_int {
            let Some(callback) = callback else { return 0 };
            let path = crate::paths::downloads_dir().join(sanitize(&string(suggested_name)));
            callback.cont(Some(&CefString::from(path.to_string_lossy().as_ref())), 0);
            1
        }

        fn on_download_updated(
            &self,
            _browser: Option<&mut Browser>,
            download_item: Option<&mut DownloadItem>,
            _callback: Option<&mut DownloadItemCallback>,
        ) {
            let Some(d) = download_item else { return };
            let path = CefString::from(&d.full_path()).to_string();
            let name = std::path::Path::new(&path)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| CefString::from(&d.suggested_file_name()).to_string());
            let info = (d.id(), name, path, d.percent_complete(), d.is_complete() != 0, d.is_canceled() != 0);
            browser::later(move |b| b.on_download(info));
        }
    }
}

fn sanitize(name: &str) -> String {
    let n: String = name.chars().map(|c| if "<>:\"/\\|?*".contains(c) || c.is_control() { '_' } else { c }).collect();
    if n.trim().is_empty() { "download".into() } else { n }
}

// ---- Sidebar ----------------------------------------------------------------------------------

wrap_client! {
    pub struct SidebarClient {}
    impl Client {
        fn display_handler(&self) -> Option<DisplayHandler> { Some(SidebarDisplay::new()) }
        fn keyboard_handler(&self) -> Option<KeyboardHandler> { Some(Keys::new()) }
        fn command_handler(&self) -> Option<CommandHandler> { Some(Commands::new()) }
        fn life_span_handler(&self) -> Option<LifeSpanHandler> { Some(SidebarLifeSpan::new()) }
        fn drag_handler(&self) -> Option<DragHandler> { Some(SidebarDrag::new()) }
    }
}

wrap_drag_handler! {
    struct SidebarDrag {}
    impl DragHandler {
        /// CSS `app-region: drag` areas of the sidebar become the window's drag areas.
        fn on_draggable_regions_changed(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            regions: Option<&[DraggableRegion]>,
        ) {
            let regions: Vec<(Rect, bool)> =
                regions.unwrap_or_default().iter().map(|r| (r.bounds.clone(), r.draggable != 0)).collect();
            browser::later(move |b| b.set_drag_regions(regions));
        }
    }
}

wrap_display_handler! {
    struct SidebarDisplay {}
    impl DisplayHandler {
        /// The sidebar talks to Rust by logging `__burrow__{json}` (D-009).
        fn on_console_message(
            &self,
            _browser: Option<&mut Browser>,
            _level: LogSeverity,
            message: Option<&CefString>,
            _source: Option<&CefString>,
            _line: ::std::os::raw::c_int,
        ) -> ::std::os::raw::c_int {
            let msg = string(message);
            match msg.strip_prefix(consts::IPC_PREFIX) {
                Some(json) => {
                    let json = json.to_string();
                    browser::later(move |b| b.on_sidebar_message(&json));
                    1
                }
                None => 0,
            }
        }
    }
}

wrap_life_span_handler! {
    struct SidebarLifeSpan {}
    impl LifeSpanHandler {
        /// Links the sidebar opens (e.g. "about") go to a new tab, never a window.
        fn on_before_popup(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            _popup_id: ::std::os::raw::c_int,
            target_url: Option<&CefString>,
            _target_frame_name: Option<&CefString>,
            _target_disposition: WindowOpenDisposition,
            _user_gesture: ::std::os::raw::c_int,
            _popup_features: Option<&PopupFeatures>,
            _window_info: Option<&mut WindowInfo>,
            _client: Option<&mut Option<Client>>,
            _settings: Option<&mut BrowserSettings>,
            _extra_info: Option<&mut Option<DictionaryValue>>,
            _no_javascript_access: Option<&mut ::std::os::raw::c_int>,
        ) -> ::std::os::raw::c_int {
            let url = string(target_url);
            browser::later(move |b| b.open_link(None, url, LinkKind::ForegroundTab));
            1
        }

        /// Closing one tab must not close the window: CEF's default (0) for a Views-hosted browser
        /// closes the parent window. Returning 1 lets just this browser go once its view is
        /// removed. While the whole window is closing, the default is what we want (D-010).
        fn do_close(&self, _browser: Option<&mut Browser>) -> ::std::os::raw::c_int {
            (!browser::is_closing()) as _
        }

        fn on_before_close(&self, _browser: Option<&mut Browser>) {
            browser::later(|b| b.on_sidebar_closed());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: char, modifiers: u32) -> KeyEvent {
        KeyEvent {
            type_: KeyEventType::RAWKEYDOWN,
            modifiers,
            windows_key_code: code as i32,
            ..Default::default()
        }
    }

    #[test]
    fn shortcuts() {
        assert_eq!(shortcut_for(&key('T', CTRL)), Some(Shortcut::NewTab));
        assert_eq!(shortcut_for(&key('N', CTRL | SHIFT)), Some(Shortcut::NewTab));
        assert_eq!(shortcut_for(&key('3', CTRL)), Some(Shortcut::Space(2)));
        assert_eq!(shortcut_for(&key('T', 0)), None);
        assert_eq!(shortcut_for(&key('W', CTRL | SHIFT)), None);
    }

    #[test]
    fn chrome_window_commands_are_blocked() {
        assert!(is_window_or_tab_command(34000)); // IDC_NEW_WINDOW
        assert!(is_window_or_tab_command(34014)); // IDC_NEW_TAB
        assert!(!is_window_or_tab_command(34030)); // IDC_FULLSCREEN
        assert!(!is_window_or_tab_command(33000)); // IDC_BACK
    }
}

wrap_request_context_handler! {
    pub struct ContextReady {
        profile_id: String,
    }
    impl RequestContextHandler {
        fn on_request_context_initialized(&self, _request_context: Option<&mut RequestContext>) {
            let id = self.profile_id.clone();
            browser::later(move |b| b.on_profile_ready(&id));
        }
    }
}

// Answers to the DevTools calls the agent tools send to a tab (D-018).
wrap_dev_tools_message_observer! {
    pub struct AgentDevTools {}
    impl DevToolsMessageObserver {
        fn on_dev_tools_method_result(
            &self,
            _browser: Option<&mut Browser>,
            message_id: ::std::os::raw::c_int,
            success: ::std::os::raw::c_int,
            result: Option<&[u8]>,
        ) {
            let json = String::from_utf8_lossy(result.unwrap_or_default()).into_owned();
            browser::later(move |b| b.on_devtools_result(message_id, success != 0, &json));
        }
    }
}
