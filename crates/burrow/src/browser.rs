//! The controller: owns the model, the window, the sidebar and every tab's BrowserView.
//!
//! Everything here runs on CEF's UI thread. CEF callbacks never touch it directly; they post a
//! closure with [`later`] so we are never re-entered while holding the state (D-009).

use std::{cell::RefCell, collections::HashMap};

use burrow_core::{
    history::{self, History},
    links::{self, Destination, LinkKind},
    store, Command, Effect, Id, Location, Section, State,
};
use cef::*;
use serde::{Deserialize, Serialize};

use crate::{clients, consts, paths, task, views, writer};

mod divider;
mod extensions;
mod icons;
mod modal;
mod peek;
mod pip;
mod tools;
mod update;
mod window_buttons;

pub use extensions::{helper_tab_id, is_popup_browser, load_extension_switch, restart_requested, POPUP_PREFIX};
pub use modal::dialog_open;
pub use peek::is_peek_browser;
pub use pip::BINDING as PIP_BINDING;
pub use update::run_pending_install;

thread_local! {
    static BURROW: RefCell<Option<Burrow>> = const { RefCell::new(None) };
}

/// Set once the window starts closing. Read from CEF's `do_close`, which must answer
/// synchronously and may run while the controller is borrowed.
static CLOSING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn is_closing() -> bool {
    CLOSING.load(std::sync::atomic::Ordering::Relaxed)
}

/// A page is fullscreen (a video). The key handler reads it: Escape leaves fullscreen, which
/// Chrome does itself but CEF's Alloy style doesn't.
static PAGE_FULLSCREEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn page_fullscreen() -> bool {
    PAGE_FULLSCREEN.load(std::sync::atomic::Ordering::Relaxed)
}

/// Runs `f` with the controller, if it exists and isn't already borrowed.
pub fn with<R>(f: impl FnOnce(&mut Burrow) -> R) -> Option<R> {
    BURROW.with(|b| b.try_borrow_mut().ok()?.as_mut().map(f))
}

/// Runs `f` with the controller on the UI thread, after the current callback returns.
pub fn later(f: impl Send + FnOnce(&mut Burrow) + 'static) {
    task::post_ui(move || {
        with(f);
    });
}

/// A tab that has a live browser.
struct LiveTab {
    view: BrowserView,
    browser_id: Option<i32>,
    nav: Nav,
    /// How many page loads have finished; agent tools wait for it to go up (D-018).
    loads: u32,
    /// Keeps the agent tools' DevTools observer attached while the browser lives.
    devtools: Option<Registration>,
    /// An agent gave the hidden page a viewport size; undone when the tab is shown.
    emulated: bool,
    /// The tab's id for the extension APIs, once the helper extension has said (D-032).
    ext_tab: Option<i32>,
}

#[derive(Default, Clone, Copy, Serialize)]
struct Nav {
    loading: bool,
    can_back: bool,
    can_forward: bool,
    muted: bool,
}

#[derive(Clone, Serialize)]
struct Download {
    id: u32,
    name: String,
    path: String,
    percent: i32,
    done: bool,
    canceled: bool,
}

/// A profile's request context. Chrome-style contexts initialize asynchronously and can't host
/// browsers until they're ready, so tabs wait in `pending`.
struct Profile {
    context: RequestContext,
    ready: bool,
    pending: Vec<Id>,
}

/// How to answer a pending permission request.
pub enum PermissionCallback {
    /// Camera/microphone/screen capture; answered with the allowed subset of these bits.
    Media(MediaAccessCallback, u32),
    Prompt(PermissionPromptCallback),
}

impl PermissionCallback {
    fn resolve(&self, allow: bool) {
        match self {
            PermissionCallback::Media(cb, bits) => cb.cont(if allow { *bits } else { 0 }),
            PermissionCallback::Prompt(cb) => cb.cont(if allow {
                PermissionRequestResult::ACCEPT
            } else {
                PermissionRequestResult::DENY
            }),
        }
    }
}

struct PendingPermission {
    id: u64,
    tab_id: Id,
    profile_id: Id,
    origin: String,
    names: Vec<&'static str>,
    callback: PermissionCallback,
}

#[derive(Serialize)]
struct PermissionView<'a> {
    id: u64,
    origin: &'a str,
    names: &'a [&'static str],
    tab_title: String,
}

/// Names for cef_media_access_permission_types_t bits.
pub fn media_permission_names(bits: u32) -> Vec<&'static str> {
    let mut v = vec![];
    if bits & 1 != 0 {
        v.push("microphone");
    }
    if bits & 2 != 0 {
        v.push("camera");
    }
    if bits & (4 | 8) != 0 {
        v.push("screen");
    }
    v
}

/// Names for cef_permission_request_types_t bits.
pub fn prompt_permission_names(bits: u32) -> Vec<&'static str> {
    const NAMES: &[(u32, &str)] = &[
        (4, "camera"),
        (4096, "microphone"),
        (2, "camera"),
        (256, "location"),
        (32768, "notifications"),
        (16, "clipboard"),
        (8192, "midi"),
        (16384, "downloads"),
        (1048576, "storage"),
        (32, "storage"),
        (128, "fonts"),
        (8388608, "windows"),
        (2048, "idle"),
        (16777216, "files"),
        (65536, "keyboard"),
        (131072, "pointer"),
        (262144, "protected-media"),
        (524288, "protocol-handler"),
        (67108864, "local-network"),
        (134217728, "local-network"),
        (268435456, "sensors"),
    ];
    let mut v: Vec<&'static str> = vec![];
    let mut known = 0;
    for (bit, name) in NAMES {
        known |= bit;
        if bits & bit != 0 && !v.contains(name) {
            v.push(name);
        }
    }
    if bits & !known != 0 {
        v.push("other");
    }
    v
}

#[derive(Default, Clone, Copy, Serialize)]
struct FindState {
    count: i32,
    current: i32,
}

pub struct Burrow {
    state: State,
    window: Option<Window>,
    sidebar: BrowserView,
    sidebar_visible: bool,
    /// The hidden sidebar is temporarily shown because the mouse is over its edge strip.
    peeking: bool,
    content: Panel,
    tabs: HashMap<Id, LiveTab>,
    contexts: HashMap<Id, Profile>,
    shown: Option<Id>,
    downloads: Vec<Download>,
    save_pending: bool,
    push_pending: bool,
    sidebar_ready: bool,
    toast: Option<String>,
    /// The window is closing: browsers are shutting down and must not change the model.
    closing: bool,
    sidebar_closed: bool,
    permissions: Vec<PendingPermission>,
    /// Ids for media requests, which CEF doesn't number (kept apart from CEF's prompt ids).
    next_media_request: u64,
    find: FindState,
    arc_available: bool,
    /// The sidebar's drag areas, in sidebar coordinates.
    drag_regions: Vec<(Rect, bool)>,
    /// Agent tools waiting for a DevTools answer, by message id (D-018).
    devtools_pending: HashMap<i32, tools::Done>,
    next_devtools_id: i32,
    /// An agent called a tool in the last few seconds; the sidebar shows it.
    agent_active: bool,
    agent_generation: u64,
    /// The tabs in the content area, left to right (two in split view).
    visible: Vec<Id>,
    peek: Option<peek::Peek>,
    history: History,
    history_pending: bool,
    /// The window is fullscreen: Burrow's own (F11) or because a page asked (videos).
    fullscreen: bool,
    /// The tab whose page went fullscreen; it alone fills the window.
    page_fullscreen: Option<Id>,
    /// Browsers of closed Peeks and extension popups that haven't finished closing; the window
    /// waits for them.
    dying_peeks: Vec<i32>,
    icons: icons::Icons,
    /// The open extension popup (D-032).
    ext_popup: Option<extensions::Popup>,
    /// Extensions being downloaded or unpacked, by a running number.
    ext_installs: HashMap<u64, extensions::Install>,
    next_install: u64,
    /// Minimize, maximize and close in the corner, for the frameless window (D-035).
    window_buttons: Option<window_buttons::WindowButtons>,
    /// The tab whose video is in picture in picture, as far as Burrow knows (D-036).
    pip_tab: Option<Id>,
    updater: update::Updater,
    /// The split view's divider, made the first time a split shows (D-038).
    divider: Option<divider::Divider>,
    /// Dialogs over the whole window (D-039).
    modal: Option<modal::Modal>,
}

/// Messages from the sidebar that aren't model commands.
#[derive(Deserialize)]
#[serde(tag = "act", rename_all = "snake_case")]
enum Act {
    Ready,
    Back,
    Forward,
    Reload,
    Stop,
    /// Load what the user typed: in the active tab, or in a new one.
    Navigate { input: String, new_tab: bool },
    /// Open in a new tab the URL of something the sidebar already knows (e.g. a favorite).
    OpenUrl { url: String },
    DevTools { sidebar: bool },
    ImportArc,
    SetSearch { url: String },
    FocusContent,
    ToggleSidebar,
    /// The mouse entered (or left) the hidden sidebar's edge strip.
    PeekSidebar { on: bool },
    OpenDownload { path: String },
    /// Answer a permission prompt shown in the sidebar.
    Permission { id: u64, allow: bool, remember: bool },
    /// Find in the active tab. Empty text stops finding.
    Find { text: String, forward: bool, next: bool },
    FindStop,
    SetArchiveHours { hours: u32 },
    WindowMinimize,
    WindowMaximize,
    WindowClose,
    SetSystemTitleBar { on: bool },
    SetAgentPageAccess { on: bool },
    /// History matches for what is typed in the address box; answered with `burrow.suggestions`.
    Suggest { query: String },
    /// The History dialog's list; answered with `burrow.history`.
    HistoryList { query: String },
    DeleteHistory { url: String },
    ClearHistory,
    SetMuted { tab_id: Id, muted: bool },
    SetPeek { new_tabs: bool, from_pinned: bool },
    ClosePeek,
    ExpandPeek,
    /// Reset the shown tab's zoom to 100%.
    ResetZoom,
    /// Custom CSS for a host ("Boost"); empty removes it.
    SetBoost { host: String, css: String },
    Print,
    /// Register Burrow as a browser with Windows and open the Default Apps settings.
    MakeDefaultBrowser,
    /// The sidebar's right edge is being dragged; `save` once the mouse is released (D-033).
    SetSidebarWidth { width: i64, save: bool },
    /// Show an extension's popup (D-032).
    OpenExtension { id: String },
    CloseExtensionPopup,
    /// Install from a Chrome Web Store link or id.
    InstallExtension { input: String },
    /// Ask for an extension folder, or a .crx/.zip file, and install it.
    ChooseExtensionFile { folder: bool },
    SetExtensionEnabled { id: String, enabled: bool },
    RemoveExtension { id: String },
    UpdateExtensions,
    Restart,
    /// Float the tab's video in picture in picture, or put it back (D-036).
    TogglePip { tab_id: Id },
    SetAutoPip { on: bool },
    /// Updates (D-037).
    CheckForUpdates,
    InstallUpdate,
    SetAutoUpdate { on: bool },
    /// The split view's divider is dragged `dx` pixels (phase start, move or end), D-038.
    SplitDrag { dx: f64, phase: String },
    /// Double-click on the divider: equal halves.
    SplitReset,
    /// Show one of the sidebar's dialogs over the window (D-039); `args` go to its function.
    OpenDialog { name: String, #[serde(default)] args: serde_json::Value },
    CloseDialog,
    /// The dialog was clicked: typing goes there now.
    FocusDialog,
    /// The dialog overlay's page has loaded.
    ModalReady,
}

/// Starts Burrow: loads the state and creates the main window. Called once, on the UI thread.
pub fn start() {
    let state = store::load(&paths::state_file()).unwrap_or_else(|e| {
        eprintln!("Burrow: could not read state ({e}); starting fresh");
        State::default()
    });
    views::set_sidebar_width(state.settings.sidebar_width as i32);

    let mut sidebar_client = clients::SidebarClient::new();
    let mut sidebar_delegate = views::SidebarViewDelegate::new();
    let sidebar = browser_view_create(
        Some(&mut sidebar_client),
        Some(&CefString::from(paths::ui_url("sidebar.html").as_str())),
        Some(&BrowserSettings {
            background_color: consts::BACKGROUND,
            ..Default::default()
        }),
        None,
        None,
        Some(&mut sidebar_delegate),
    )
    .expect("create sidebar");

    let content = panel_create(None).expect("create content panel");
    // Hidden tabs are skipped by the layout, so one tab fills the area and two split it, with
    // the divider between them as the gap (D-038).
    content.set_to_box_layout(Some(&BoxLayoutSettings {
        horizontal: 1,
        cross_axis_alignment: AxisAlignment::STRETCH,
        default_flex: 1,
        ..Default::default()
    }));
    content.set_background_color(0xFFFFFFFF);

    BURROW.with(|b| {
        *b.borrow_mut() = Some(Burrow {
            state,
            window: None,
            sidebar,
            sidebar_visible: true,
            peeking: false,
            content,
            tabs: HashMap::new(),
            contexts: HashMap::new(),
            shown: None,
            downloads: Vec::new(),
            save_pending: false,
            push_pending: false,
            sidebar_ready: false,
            toast: None,
            closing: false,
            sidebar_closed: false,
            permissions: Vec::new(),
            next_media_request: 1 << 40, // stays exact as a JS number (< 2^53)
            find: FindState::default(),
            arc_available: paths::arc_sidebar_file().is_some(),
            drag_regions: Vec::new(),
            devtools_pending: HashMap::new(),
            next_devtools_id: 1,
            agent_active: false,
            agent_generation: 0,
            visible: Vec::new(),
            peek: None,
            history: history::load(&paths::history_file()),
            history_pending: false,
            fullscreen: false,
            page_fullscreen: None,
            dying_peeks: Vec::new(),
            icons: icons::Icons::load(),
            ext_popup: None,
            ext_installs: HashMap::new(),
            next_install: 1,
            window_buttons: None,
            pip_tab: None,
            updater: Default::default(),
            divider: None,
            modal: None,
        })
    });
    crate::agent::start_server();

    let mut window_delegate = views::MainWindowDelegate::new();
    window_create_top_level(Some(&mut window_delegate));
}

fn cef(s: &str) -> CefString {
    CefString::from(s)
}

/// A string as a JavaScript literal.
fn js_string(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_default()
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// Converts a CSS `#rrggbb` color to CEF's ARGB.
fn argb(hex: &str) -> u32 {
    let h = hex.trim_start_matches('#');
    u32::from_str_radix(h.get(..6).unwrap_or("c9c2f0"), 16).map_or(consts::BACKGROUND, |rgb| 0xFF00_0000 | rgb)
}

fn set_icon(window: &Window) {
    let Some(mut image) = image_create() else { return };
    image.add_png(1.0, Some(include_bytes!("../resources/icon-32.png")));
    image.add_png(2.0, Some(include_bytes!("../resources/icon-64.png")));
    window.set_window_icon(Some(&mut image));
    let Some(mut app) = image_create() else { return };
    app.add_png(1.0, Some(include_bytes!("../resources/icon-256.png")));
    window.set_window_app_icon(Some(&mut app));
}

/// Blends an ARGB color toward white by `amount` (0 = unchanged, 1 = white).
fn mix_with_white(c: u32, amount: f32) -> u32 {
    let ch = |shift: u32| {
        let v = ((c >> shift) & 0xFF) as f32;
        ((v + (255.0 - v) * amount).round() as u32) << shift
    };
    0xFF00_0000 | ch(16) | ch(8) | ch(0)
}

impl Burrow {
    /// Lays out the window once CEF has created it.
    pub fn attach(&mut self, window: &mut Window) {
        window.add_child_view(Some(&mut View::from(&self.sidebar)));
        window.add_child_view(Some(&mut View::from(&self.content)));
        self.window = Some(window.clone());
        self.layout_window();
        set_icon(window);
        clients::register_accelerators(window);
        let profiles: Vec<Id> = self.state.profiles.iter().map(|p| p.id.clone()).collect();
        for p in profiles {
            self.profile(&p);
        }
        self.apply_theme();
        self.sync_view();
        window.show();
        self.create_window_buttons();
        self.schedule_update_checks();
        self.archive_tick();
        // Links that started Burrow (Burrow as the default browser).
        for url in crate::external::take_startup_urls() {
            self.open_external(url);
        }
    }

    /// The sidebar next to the content card; in fullscreen the content alone fills the window.
    fn layout_window(&self) {
        let Some(window) = &self.window else { return };
        let m = if self.fullscreen { 0 } else { consts::CARD_MARGIN };
        let layout = window.set_to_box_layout(Some(&BoxLayoutSettings {
            horizontal: 1,
            inside_border_insets: Insets { top: m, left: 0, bottom: m, right: m },
            cross_axis_alignment: AxisAlignment::STRETCH,
            ..Default::default()
        }));
        View::from(&self.sidebar).set_visible(!self.fullscreen as i32);
        if let Some(layout) = layout {
            layout.set_flex_for_view(Some(&mut View::from(&self.sidebar)), 0);
            layout.set_flex_for_view(Some(&mut View::from(&self.content)), 1);
        }
        window.layout();
        self.layout_window_buttons();
    }

    /// Archives stale Today tabs now and every ten minutes.
    fn archive_tick(&mut self) {
        let effects = self.state.archive_stale(now_ms());
        if !effects.is_empty() {
            for e in effects {
                self.effect(e);
            }
            self.sync_view();
            self.changed();
        }
        task::post_ui_delayed(10 * 60 * 1000, || {
            with(|b| b.archive_tick());
        });
    }

    /// The user asked to close the window. Browsers must close first, so the first call closes
    /// them all and says no; the window is closed again once the last browser is gone.
    pub fn request_close(&mut self) -> bool {
        if self.all_closed() {
            return true;
        }
        if !self.closing {
            self.closing = true;
            CLOSING.store(true, std::sync::atomic::Ordering::Relaxed);
            self.close_peek();
            self.close_extension_popup();
            self.save_now();
            // Remember which browser is whose now: views may forget their browser while closing.
            self.tabs.retain(|_, t| match t.view.browser() {
                Some(b) => {
                    t.browser_id = Some(b.identifier());
                    true
                }
                None => false,
            });
            for tab in self.tabs.values() {
                if let Some(host) = tab.view.browser().and_then(|b| b.host()) {
                    host.close_browser(1);
                }
            }
            if let Some(host) = self.sidebar.browser().and_then(|b| b.host()) {
                host.close_browser(1);
            }
            for b in [self.divider_browser(), self.modal_browser()].into_iter().flatten() {
                self.dying_peeks.push(b.identifier());
                if let Some(host) = b.host() {
                    host.close_browser(1);
                }
            }
        }
        false
    }

    fn all_closed(&self) -> bool {
        self.tabs.is_empty() && self.sidebar_closed && self.dying_peeks.is_empty()
    }

    /// Called as each browser finishes closing during shutdown.
    fn maybe_finish_close(&self) {
        if self.closing && self.all_closed() {
            task::post_ui(|| {
                if let Some(w) = with(|b| b.window.clone()).flatten() {
                    w.close();
                }
            });
        }
    }

    pub fn on_sidebar_closed(&mut self) {
        self.sidebar_closed = true;
        self.maybe_finish_close();
    }

    pub fn detach(&mut self) {
        self.save_now();
        for (_, tab) in self.tabs.drain() {
            if let Some(host) = tab.view.browser().and_then(|b| b.host()) {
                host.close_browser(1);
            }
        }
        self.window = None;
    }

    // ---- Sidebar messages -------------------------------------------------------------------

    pub fn on_sidebar_message(&mut self, json: &str) {
        let value: serde_json::Value = match serde_json::from_str(json) {
            Ok(v) => v,
            Err(e) => return eprintln!("Burrow: bad sidebar message: {e}"),
        };
        if value.get("cmd").is_some() {
            match serde_json::from_value::<Command>(value) {
                Ok(cmd) => self.command(cmd),
                Err(e) => eprintln!("Burrow: bad command: {e}"),
            }
            return;
        }
        match serde_json::from_value::<Act>(value) {
            Ok(act) => self.act(act),
            Err(e) => eprintln!("Burrow: bad action: {e}"),
        }
    }

    fn act(&mut self, act: Act) {
        match act {
            Act::Ready => {
                self.sidebar_ready = true;
                self.push_state();
            }
            Act::Back => self.with_active_browser(|b| b.go_back()),
            Act::Forward => self.with_active_browser(|b| b.go_forward()),
            Act::Reload => self.with_active_browser(|b| b.reload()),
            Act::Stop => self.with_active_browser(|b| b.stop_load()),
            Act::Navigate { input, new_tab } => {
                let url = burrow_core::omnibox::resolve(&input, &self.state.settings.search_url);
                if url.is_empty() {
                    return;
                }
                match self.shown.clone().filter(|_| !new_tab) {
                    Some(id) => self.navigate_tab(&id, &url),
                    None => self.command(Command::NewTab { url }),
                }
                self.focus_content();
            }
            Act::OpenUrl { url } => self.command(Command::NewTab { url }),
            Act::DevTools { sidebar } => {
                let view = if sidebar { Some(&self.sidebar) } else { self.shown.as_ref().and_then(|id| self.tabs.get(id)).map(|t| &t.view) };
                if let Some(host) = view.and_then(|v| v.browser()).and_then(|b| b.host()) {
                    host.show_dev_tools(None, None, None, None);
                }
            }
            Act::ImportArc => self.import_arc(),
            Act::SetSearch { url } => {
                if url.contains("{}") {
                    self.state.settings.search_url = url;
                    self.changed();
                }
            }
            Act::FocusContent => self.focus_content(),
            Act::ToggleSidebar => self.toggle_sidebar(),
            Act::PeekSidebar { on } => {
                if !self.sidebar_visible && self.peeking != on {
                    self.peeking = on;
                    self.resize_sidebar();
                }
            }
            Act::OpenDownload { path } => open_path(&path),
            Act::Permission { id, allow, remember } => self.answer_permission(id, allow, remember),
            Act::Find { text, forward, next } => {
                if text.is_empty() {
                    return self.act(Act::FindStop);
                }
                self.with_active_browser(|b| {
                    if let Some(h) = b.host() {
                        h.find(Some(&cef(&text)), forward as i32, 0, next as i32);
                    }
                });
            }
            Act::FindStop => {
                self.with_active_browser(|b| {
                    if let Some(h) = b.host() {
                        h.stop_finding(1);
                    }
                });
                self.find = FindState::default();
                self.push_state();
            }
            Act::WindowMinimize => {
                if let Some(w) = &self.window {
                    w.minimize();
                }
            }
            Act::WindowMaximize => {
                if let Some(w) = &self.window {
                    if w.is_maximized() != 0 { w.restore() } else { w.maximize() }
                }
            }
            // Closing re-enters `can_close`, which needs the controller: do it after this returns.
            Act::WindowClose => task::post_ui(|| {
                if let Some(w) = with(|b| b.window.clone()).flatten() {
                    w.close();
                }
            }),
            Act::SetSystemTitleBar { on } => {
                self.state.settings.system_title_bar = on;
                self.toast = Some("Restart Burrow to switch the title bar.".into());
                self.changed();
            }
            Act::SetAgentPageAccess { on } => {
                self.state.settings.agent_page_access = on;
                self.toast = Some(if on {
                    "Agents can read and use pages again."
                } else {
                    "Agents can no longer read or use pages. They can still manage tabs and spaces."
                }.into());
                self.changed();
            }
            Act::SetArchiveHours { hours } => {
                self.state.settings.archive_after_hours = hours;
                self.changed();
            }
            Act::Suggest { query } => {
                let hits = self.history.search(&query, 8, now_ms());
                let json = serde_json::to_string(&hits).unwrap_or_default();
                self.run_in_sidebar(&format!("window.burrow && burrow.suggestions({}, {json})", js_string(&query)));
            }
            Act::HistoryList { query } => {
                let hits = self.history.search(&query, 300, now_ms());
                let json = serde_json::to_string(&hits).unwrap_or_default();
                let js = format!("window.burrow && burrow.history({}, {json})", js_string(&query));
                self.run_in_sidebar(&js);
                self.run_in_modal(js);
            }
            Act::DeleteHistory { url } => {
                self.history.remove(&url);
                self.schedule_history_save();
            }
            Act::ClearHistory => {
                self.history = History::default();
                self.schedule_history_save();
                self.toast = Some("History cleared.".into());
                self.push_state();
            }
            Act::SetMuted { tab_id, muted } => {
                self.set_muted(&tab_id, muted);
            }
            Act::SetPeek { new_tabs, from_pinned } => {
                self.state.settings.peek_new_tabs = new_tabs;
                self.state.settings.peek_from_pinned = from_pinned;
                self.changed();
            }
            Act::ClosePeek => self.close_peek(),
            Act::ExpandPeek => self.expand_peek(),
            Act::ResetZoom => {
                self.with_active_browser(|b| {
                    if let Some(h) = b.host() {
                        h.set_zoom_level(0.0);
                    }
                });
                self.remember_zoom();
            }
            Act::SetBoost { host, css } => {
                self.set_boost(&host, &css);
            }
            Act::Print => self.with_active_browser(|b| {
                if let Some(h) = b.host() {
                    h.print();
                }
            }),
            Act::MakeDefaultBrowser => {
                self.toast = Some(crate::external::register_default_browser());
                self.push_state();
            }
            Act::SetSidebarWidth { width, save } => {
                self.set_sidebar_width(width);
                if save {
                    self.changed();
                }
            }
            Act::OpenExtension { id } => {
                if let Err(e) = self.open_extension_popup(&id) {
                    self.toast = Some(e);
                    self.push_state();
                }
            }
            Act::CloseExtensionPopup => self.close_extension_popup(),
            Act::InstallExtension { input } => self.install_from_web_store(&input, None),
            Act::ChooseExtensionFile { folder } => self.choose_extension_file(folder),
            Act::SetExtensionEnabled { id, enabled } => {
                self.set_extension_enabled(&id, enabled);
            }
            Act::RemoveExtension { id } => {
                self.remove_extension(&id);
            }
            Act::UpdateExtensions => {
                if self.update_extensions(None) == 0 {
                    self.toast = Some("No extensions from the Chrome Web Store to update.".into());
                    self.push_state();
                }
            }
            Act::Restart => self.restart(),
            Act::TogglePip { tab_id } => self.toggle_pip(&tab_id),
            Act::SetAutoPip { on } => {
                self.state.settings.auto_pip = on;
                self.changed();
            }
            Act::CheckForUpdates => self.check_for_update(true, None),
            Act::InstallUpdate => {
                if let Err(e) = self.install_update() {
                    self.toast = Some(e);
                    self.push_state();
                }
            }
            Act::SetAutoUpdate { on } => {
                self.state.settings.auto_update = on;
                self.changed();
            }
            Act::SplitDrag { dx, phase } => self.split_drag(dx, &phase),
            Act::SplitReset => self.set_split_ratio(0.5, true),
            Act::OpenDialog { name, args } => {
                let args = if args.is_array() { args } else { serde_json::json!([]) };
                self.open_dialog(&name, &args);
            }
            Act::CloseDialog => self.close_modal(),
            Act::FocusDialog => self.focus_modal(),
            Act::ModalReady => self.on_modal_ready(),
        }
    }

    // ---- Model ------------------------------------------------------------------------------

    pub fn command(&mut self, cmd: Command) {
        match self.state.apply(cmd) {
            Ok(effects) => {
                for e in effects {
                    self.effect(e);
                }
            }
            Err(e) => eprintln!("Burrow: {e}"),
        }
        self.sync_view();
        self.changed();
    }

    fn effect(&mut self, effect: Effect) {
        match effect {
            Effect::CloseBrowser { tab_id } => self.unload(&tab_id),
            Effect::Navigate { tab_id, url } => self.navigate_tab(&tab_id, &url),
            Effect::Rehome { tab_id, profile_changed } => {
                if profile_changed {
                    self.unload(&tab_id);
                }
            }
            Effect::ProfileCreated { profile_id } => {
                self.profile(&profile_id);
            }
        }
    }

    /// Call after any state change: saves soon and refreshes the sidebar.
    fn changed(&mut self) {
        self.schedule_save();
        self.schedule_icon_scan();
        self.push_state();
    }

    fn schedule_save(&mut self) {
        if !self.save_pending {
            self.save_pending = true;
            task::post_ui_delayed(1500, || {
                with(|b| b.save(false));
            });
        }
    }

    /// Saves before returning (on shutdown).
    pub fn save_now(&mut self) {
        self.save(true);
    }

    /// Serializes here, but writes on another thread unless `wait` (D-034).
    fn save(&mut self, wait: bool) {
        self.save_pending = false;
        match store::encode(&self.state) {
            Ok(json) if wait => writer::write_now(&paths::state_file(), &json),
            Ok(json) => writer::write_soon(paths::state_file(), json),
            Err(e) => eprintln!("Burrow: could not save state: {e}"),
        }
        if self.history_pending {
            self.save_history(wait);
        }
    }

    /// History changes on every page load, so it is written less often than the state.
    fn schedule_history_save(&mut self) {
        if !self.history_pending {
            self.history_pending = true;
            task::post_ui_delayed(10_000, || {
                with(|b| b.save_history(false));
            });
        }
    }

    fn save_history(&mut self, wait: bool) {
        self.history_pending = false;
        match history::encode(&self.history) {
            Ok(json) if wait => writer::write_now(&paths::history_file(), &json),
            Ok(json) => writer::write_soon(paths::history_file(), json),
            Err(e) => eprintln!("Burrow: could not save history: {e}"),
        }
    }

    fn import_arc(&mut self) {
        let msg = match paths::arc_sidebar_file() {
            None => "Arc's data wasn't found on this computer.".to_string(),
            Some(path) => match std::fs::read_to_string(&path)
                .map_err(|e| e.to_string())
                .and_then(|j| burrow_core::import_arc::import_arc_sidebar(&j).map_err(|e| e.to_string()))
            {
                Ok(imported) => {
                    self.state.absorb(imported.state);
                    self.sync_view();
                    format!("Imported from Arc: {}", imported.summary)
                }
                Err(e) => format!("Arc import failed: {e}"),
            },
        };
        self.toast = Some(msg);
        self.apply_theme();
        self.changed();
    }

    // ---- Views ------------------------------------------------------------------------------

    /// Makes the content area show `State::active_tab()` (and its split partner), creating
    /// browsers as needed.
    fn sync_view(&mut self) {
        let want = self.state.active_tab().map(|t| t.id.clone());
        let mut visible = self.state.visible_tabs();
        if let Some(fs) = &self.page_fullscreen {
            visible.retain(|id| id == fs);
        }
        if want != self.shown || visible != self.visible {
            // A Peek belongs to the page it was opened over.
            if want != self.shown {
                self.close_peek();
                self.close_extension_popup();
            }
            for id in &self.visible {
                if !visible.contains(id)
                    && let Some(t) = self.tabs.get(id) {
                        t.view.set_visible(0);
                    }
            }
            for (i, id) in visible.iter().enumerate() {
                self.ensure_live(id);
                if let Some(t) = self.tabs.get(id) {
                    self.content.reorder_child_view(Some(&mut View::from(&t.view)), i as i32);
                    t.view.set_visible(1);
                }
            }
            let left: Vec<Id> = self.visible.iter().filter(|id| !visible.contains(id)).cloned().collect();
            let back = self.pip_tab.clone().filter(|p| visible.contains(p) && !self.visible.contains(p));
            let focus = want != self.shown;
            self.shown = want;
            self.visible = visible;
            self.layout_split();
            self.content.layout();
            if focus {
                self.focus_content();
            }
            for id in self.visible.clone() {
                self.end_emulation(&id);
            }
            // Arc-style picture in picture: a video floats out of the tab we left, and goes back
            // into a tab we return to.
            if let Some(id) = back {
                self.stop_pip(&id, Box::new(|_, _| {}));
            }
            self.auto_pip(left);
        } else if self.shown.is_none() {
            // E.g. the last tab was just closed: keep keyboard focus somewhere that has shortcuts.
            self.sidebar.request_focus();
        } else if self.visible.len() == 2 {
            // The same split, maybe with its divider moved (by an agent, say).
            self.layout_split();
            self.content.layout();
        }
        self.apply_theme();
    }

    fn ensure_live(&mut self, tab_id: &str) {
        if self.tabs.contains_key(tab_id) {
            return;
        }
        let Some(tab) = self.state.tab(tab_id) else { return };
        let url = tab.url.clone();
        let profile_id = self.state.profile_of_tab(tab_id).cloned().unwrap_or_default();
        let Some(profile) = self.profile(&profile_id) else {
            return eprintln!("Burrow: could not create profile {profile_id}");
        };
        if !profile.ready {
            if !profile.pending.iter().any(|p| p == tab_id) {
                profile.pending.push(tab_id.to_string());
            }
            return;
        }
        let mut context = Some(profile.context.clone());
        let mut client = clients::ContentClient::new();
        let mut delegate = views::TabViewDelegate::new(true);
        let Some(view) = browser_view_create(
            Some(&mut client),
            Some(&cef(&url)),
            Some(&BrowserSettings { background_color: 0xFFFFFFFF, ..Default::default() }),
            None,
            context.as_mut(),
            Some(&mut delegate),
        ) else {
            return eprintln!("Burrow: could not create a browser for {url}");
        };
        self.adopt_view(tab_id.to_string(), view);
    }

    fn adopt_view(&mut self, tab_id: Id, view: BrowserView) {
        view.set_visible(0);
        self.content.add_child_view(Some(&mut View::from(&view)));
        self.tabs.insert(
            tab_id,
            LiveTab {
                view,
                browser_id: None,
                nav: Nav::default(),
                loads: 0,
                devtools: None,
                emulated: false,
                ext_tab: None,
            },
        );
    }

    /// The request context (cookie jar, storage, logins) of a profile (D-004), created on first use.
    fn profile(&mut self, profile_id: &str) -> Option<&mut Profile> {
        if !self.contexts.contains_key(profile_id) {
            let dir = paths::profile_dir(profile_id);
            let settings = RequestContextSettings {
                cache_path: cef(&dir.to_string_lossy()),
                persist_session_cookies: 1,
                ..Default::default()
            };
            let mut handler = clients::ContextReady::new(profile_id.to_string());
            let context = request_context_create_context(Some(&settings), Some(&mut handler))?;
            let profile = Profile { context, ready: false, pending: Vec::new() };
            self.contexts.insert(profile_id.to_string(), profile);
        }
        self.contexts.get_mut(profile_id)
    }

    /// A profile finished initializing: create the tabs that were waiting for it.
    pub fn on_profile_ready(&mut self, profile_id: &str) {
        let Some(profile) = self.contexts.get_mut(profile_id) else { return };
        profile.ready = true;
        let pending = std::mem::take(&mut profile.pending);
        for tab_id in pending {
            self.ensure_live(&tab_id);
        }
        for (i, id) in self.visible.clone().iter().enumerate() {
            if let Some(t) = self.tabs.get(id) {
                self.content.reorder_child_view(Some(&mut View::from(&t.view)), i as i32);
                t.view.set_visible(1);
            }
        }
        self.layout_split();
        self.content.layout();
        self.focus_content();
        self.schedule_icon_scan();
        self.push_state();
    }

    /// Closes a tab's browser, keeping the tab in the model.
    fn unload(&mut self, tab_id: &str) {
        // A tab still waiting for its profile must not get a browser later.
        for profile in self.contexts.values_mut() {
            profile.pending.retain(|p| p != tab_id);
        }
        // Its open permission prompts die with it.
        self.permissions.retain(|p| {
            let keep = p.tab_id != tab_id;
            if !keep {
                p.callback.resolve(false);
            }
            keep
        });
        let Some(tab) = self.tabs.remove(tab_id) else { return };
        if let Some(host) = tab.view.browser().and_then(|b| b.host()) {
            host.close_browser(1);
        }
        self.content.remove_child_view(Some(&mut View::from(&tab.view)));
        if self.shown.as_deref() == Some(tab_id) {
            self.shown = None;
        }
        self.visible.retain(|v| v != tab_id);
        if self.pip_tab.as_deref() == Some(tab_id) {
            self.pip_tab = None;
        }
        if self.page_fullscreen.as_deref() == Some(tab_id) {
            self.set_fullscreen(false);
        }
    }

    fn navigate_tab(&mut self, tab_id: &str, url: &str) {
        if let Some(t) = self.state.tab_mut(tab_id) {
            t.url = url.to_string();
        }
        match self.tabs.get(tab_id).and_then(|t| t.view.browser()).and_then(|b| b.main_frame()) {
            Some(frame) => frame.load_url(Some(&cef(url))),
            None => self.sync_view(),
        }
        self.changed();
    }

    fn with_active_browser(&self, f: impl FnOnce(&Browser)) {
        if let Some(b) = self.shown.as_ref().and_then(|id| self.tabs.get(id)).and_then(|t| t.view.browser()) {
            f(&b);
        }
    }

    /// Focuses the shown tab, or the sidebar when there is none, so shortcuts keep working.
    fn focus_content(&self) {
        match self.shown.as_ref().and_then(|id| self.tabs.get(id)) {
            Some(t) => t.view.request_focus(),
            None => self.sidebar.request_focus(),
        }
    }

    pub fn focus_sidebar_input(&mut self, new_tab: bool) {
        if !self.sidebar_visible {
            self.toggle_sidebar();
        }
        self.sidebar.request_focus();
        self.run_in_sidebar(&format!("window.burrow && burrow.focusInput({new_tab})"));
    }

    /// Hiding the sidebar leaves a thin edge strip; hovering it peeks the sidebar back out.
    pub fn toggle_sidebar(&mut self) {
        self.sidebar_visible = !self.sidebar_visible;
        self.peeking = false;
        self.resize_sidebar();
    }

    fn sidebar_shown(&self) -> bool {
        self.sidebar_visible || self.peeking
    }

    fn resize_sidebar(&mut self) {
        self.apply_sidebar_width();
        self.push_state();
    }

    /// Sizes the sidebar view: its chosen width, or the edge strip while hidden.
    fn apply_sidebar_width(&mut self) {
        views::set_sidebar_width(if self.sidebar_shown() {
            self.state.settings.sidebar_width as i32
        } else {
            consts::SIDEBAR_COLLAPSED_WIDTH
        });
        View::from(&self.sidebar).invalidate_layout();
        if let Some(w) = &self.window {
            w.layout();
        }
        self.apply_drag_regions();
        self.layout_window_buttons();
        self.layout_peek();
        self.layout_extension_popup();
    }

    /// Sets the sidebar's width (D-033), leaving the page at least `MIN_CONTENT_WIDTH`. Returns
    /// the width it got.
    pub fn set_sidebar_width(&mut self, width: i64) -> u32 {
        const MIN_CONTENT_WIDTH: i64 = 320;
        let window_width = self.window.as_ref().map_or(i64::MAX, |w| w.bounds().width as i64);
        let room = (window_width - MIN_CONTENT_WIDTH).max(burrow_core::model::SIDEBAR_WIDTH_MIN as i64);
        let width = burrow_core::model::clamp_sidebar_width(width.min(room));
        if width != self.state.settings.sidebar_width {
            self.state.settings.sidebar_width = width;
            if self.sidebar_shown() {
                self.apply_sidebar_width();
            }
        }
        width
    }

    pub fn frameless(&self) -> bool {
        !self.state.settings.system_title_bar
    }

    pub fn set_drag_regions(&mut self, regions: Vec<(Rect, bool)>) {
        self.drag_regions = regions;
        self.apply_drag_regions();
    }

    /// Maps the sidebar's drag areas into window coordinates and hands them to the window.
    fn apply_drag_regions(&self) {
        let Some(w) = &self.window else { return };
        if !self.frameless() {
            return;
        }
        let mut origin = Point { x: 0, y: 0 };
        View::from(&self.sidebar).convert_point_to_window(Some(&mut origin));
        let mut regions: Vec<DraggableRegion> = if self.sidebar_shown() {
            self.drag_regions
                .iter()
                .map(|(r, drag)| DraggableRegion {
                    bounds: Rect { x: r.x + origin.x, y: r.y + origin.y, width: r.width, height: r.height },
                    draggable: *drag as i32,
                })
                .collect()
        } else {
            vec![]
        };
        // The strip above the card moves the window too, also with the sidebar hidden; the
        // corner belongs to the window buttons.
        if !self.fullscreen {
            let card = View::from(&self.content).bounds();
            regions.push(DraggableRegion {
                bounds: Rect { x: 0, y: 0, width: (card.x + card.width - 3 * 46).max(0), height: consts::CARD_MARGIN },
                draggable: 1,
            });
        }
        w.set_draggable_regions(Some(&regions));
    }

    pub fn on_window_resized(&mut self) {
        self.apply_drag_regions();
        self.layout_modal();
        self.layout_window_buttons();
        self.layout_peek();
        self.layout_extension_popup();
        self.push_state();
    }

    /// Fullscreen hides the sidebar and the card's margins. A page's fullscreen (a video) also
    /// hides the other half of a split view.
    fn set_fullscreen(&mut self, on: bool) {
        if !on {
            // Tell the page too, if it is the one that went fullscreen.
            if let Some(id) = self.page_fullscreen.take()
                && let Some(h) = self.tabs.get(&id).and_then(|t| t.view.browser()).and_then(|b| b.host()) {
                    h.exit_fullscreen(1);
                }
        }
        PAGE_FULLSCREEN.store(on && self.page_fullscreen.is_some(), std::sync::atomic::Ordering::Relaxed);
        if on {
            self.close_extension_popup();
        }
        if self.fullscreen != on {
            self.fullscreen = on;
            if let Some(w) = &self.window {
                w.set_fullscreen(on as i32);
            }
            self.layout_window();
        }
        self.sync_view();
        self.layout_peek();
        self.push_state();
    }

    pub fn on_page_fullscreen(&mut self, browser_id: i32, on: bool) {
        let Some(id) = self.tab_of_browser(browser_id) else { return };
        if on {
            self.page_fullscreen = Some(id);
            self.set_fullscreen(true);
        } else if self.page_fullscreen.as_deref() == Some(&*id) {
            // The page already left fullscreen; don't ask it to again.
            self.page_fullscreen = None;
            self.set_fullscreen(false);
        }
    }

    /// Remembers the shown tab's zoom for its site, so the site opens at that zoom next time.
    fn remember_zoom(&mut self) {
        let Some(id) = self.shown.clone() else { return };
        let Some(level) = self.tabs.get(&id).and_then(|t| t.view.browser()).and_then(|b| b.host()).map(|h| h.zoom_level()) else {
            return;
        };
        let host = self.state.tab(&id).map(|t| links::host_of(&t.url)).unwrap_or_default();
        if host.is_empty() {
            return;
        }
        if level.abs() < 0.01 {
            self.state.zoom.remove(&host);
        } else {
            self.state.zoom.insert(host, level);
        }
        self.changed();
    }

    /// Applies the remembered zoom of the tab's site.
    fn apply_zoom(&self, tab_id: &str) {
        let Some(tab) = self.state.tab(tab_id) else { return };
        let level = self.state.zoom.get(&links::host_of(&tab.url)).copied().unwrap_or(0.0);
        if let Some(h) = self.tabs.get(tab_id).and_then(|t| t.view.browser()).and_then(|b| b.host())
            && (h.zoom_level() - level).abs() > 0.01 {
                h.set_zoom_level(level);
            }
    }

    /// Mutes or unmutes a loaded tab. False if the tab isn't loaded.
    fn set_muted(&mut self, tab_id: &str, muted: bool) -> bool {
        let Some(t) = self.tabs.get_mut(tab_id) else { return false };
        if let Some(h) = t.view.browser().and_then(|b| b.host()) {
            h.set_audio_muted(muted as i32);
        }
        t.nav.muted = muted;
        self.push_state();
        true
    }

    /// Sets (or, with empty CSS, removes) a site's Boost and restyles its open pages now.
    /// Returns the host it was stored under, or None if `host` isn't a site.
    fn set_boost(&mut self, host: &str, css: &str) -> Option<String> {
        let host = links::host_of(&format!("https://{}", host.trim().trim_start_matches("https://").trim_start_matches("http://")));
        if host.is_empty() {
            return None;
        }
        if css.trim().is_empty() {
            self.state.boosts.remove(&host);
        } else {
            self.state.boosts.insert(host.clone(), css.to_string());
        }
        let ids: Vec<Id> = self.tabs.keys().cloned().collect();
        for id in ids {
            if self.state.tab(&id).is_some_and(|t| links::host_of(&t.url) == host) {
                self.apply_boost(&id);
            }
        }
        self.changed();
        Some(host)
    }

    /// Zooms a loaded tab and remembers it for the site. False if the tab isn't loaded.
    fn set_zoom_percent(&mut self, tab_id: &str, percent: f64) -> bool {
        let Some(h) = self.tabs.get(tab_id).and_then(|t| t.view.browser()).and_then(|b| b.host()) else { return false };
        // Chromium zoom levels: each step of 1 is a factor of 1.2.
        let level = (percent.clamp(25.0, 500.0) / 100.0).ln() / 1.2f64.ln();
        h.set_zoom_level(level);
        let site = self.state.tab(tab_id).map(|t| links::host_of(&t.url)).unwrap_or_default();
        if !site.is_empty() {
            if level.abs() < 0.01 {
                self.state.zoom.remove(&site);
            } else {
                self.state.zoom.insert(site, level);
            }
        }
        self.changed();
        true
    }

    /// Adds the site's Boost (the user's CSS) to the tab's page, replacing an older one.
    fn apply_boost(&self, tab_id: &str) {
        let Some(tab) = self.state.tab(tab_id) else { return };
        let css = self.state.boosts.get(&links::host_of(&tab.url)).cloned().unwrap_or_default();
        let Some(frame) = self.tabs.get(tab_id).and_then(|t| t.view.browser()).and_then(|b| b.main_frame()) else { return };
        let js = format!(
            "(() => {{ let s = document.getElementById('__burrow_boost'); const css = {}; \
             if (!css) {{ if (s) s.remove(); return; }} \
             if (!s) {{ s = document.createElement('style'); s.id = '__burrow_boost'; (document.head || document.documentElement).append(s); }} \
             s.textContent = css; }})()",
            js_string(&css)
        );
        frame.execute_java_script(Some(&cef(&js)), Some(&CefString::from(&frame.url())), 0);
    }

    /// Shows the window in front: Burrow was launched again (D-029) or got a link (D-024).
    pub fn bring_to_front(&mut self) {
        let Some(w) = &self.window else { return };
        if w.is_minimized() != 0 {
            w.restore();
        }
        w.show();
        w.activate();
        w.bring_to_top();
    }

    fn apply_theme(&self) {
        let space = self.state.active_space();
        // With no tab open, the content card shows a light tint of the space color.
        self.content.set_background_color(mix_with_white(argb(&space.color), 0.75));
        if let Some(w) = &self.window {
            w.set_background_color(argb(&space.color));
            let title = match self.state.active_tab() {
                Some(t) => format!("{} — {}", t.display_title(), consts::APP_NAME),
                None => format!("{} — {}", space.name, consts::APP_NAME),
            };
            w.set_title(Some(&cef(&title)));
        }
        self.layout_window_buttons();
        self.paint_divider();
    }

    // ---- Keyboard ---------------------------------------------------------------------------

    pub fn shortcut(&mut self, s: Shortcut) {
        match s {
            Shortcut::NewTab => self.focus_sidebar_input(true),
            Shortcut::FocusAddress => self.focus_sidebar_input(false),
            Shortcut::CloseTab => {
                if self.peek.is_some() {
                    return self.close_peek();
                }
                if let Some(id) = self.shown.clone() {
                    self.command(Command::CloseTab { tab_id: id });
                }
            }
            Shortcut::Escape => {
                if self.modal_open() {
                    self.close_modal();
                } else if self.ext_popup.is_some() {
                    self.close_extension_popup();
                } else if self.peek.is_some() {
                    self.close_peek();
                } else if self.page_fullscreen.is_some() {
                    self.set_fullscreen(false);
                }
            }
            Shortcut::Fullscreen => {
                let on = !self.fullscreen;
                self.set_fullscreen(on);
            }
            Shortcut::CopyUrl => {
                if let Some(t) = self.state.active_tab() {
                    crate::clipboard::set(&t.url);
                    self.toast = Some("Link copied".into());
                    self.push_state();
                }
            }
            Shortcut::Print => self.act(Act::Print),
            Shortcut::History => self.open_dialog("history", &serde_json::json!([])),
            Shortcut::Reload => self.with_active_browser(|b| b.reload()),
            Shortcut::Back => self.with_active_browser(|b| b.go_back()),
            Shortcut::Forward => self.with_active_browser(|b| b.go_forward()),
            Shortcut::Space(n) => {
                if let Some(s) = self.state.spaces.get(n) {
                    let space_id = s.id.clone();
                    self.command(Command::SwitchSpace { space_id });
                }
            }
            Shortcut::CycleTab(step) => {
                let space = self.state.active_space();
                let ids: Vec<Id> = space.tabs().iter().map(|t| t.id.clone()).collect();
                if ids.is_empty() {
                    return;
                }
                let cur = space.active_tab.as_ref().and_then(|a| ids.iter().position(|i| i == a));
                let next = match cur {
                    Some(i) => (i as i32 + step).rem_euclid(ids.len() as i32) as usize,
                    None => 0,
                };
                self.command(Command::SelectTab { tab_id: ids[next].clone() });
            }
            Shortcut::TogglePin => {
                let Some(id) = self.shown.clone() else { return };
                let section = match self.state.active_space().section_of(&id) {
                    Some(Section::Today) => Section::Pinned,
                    _ => Section::Today,
                };
                self.command(Command::MoveNode {
                    node_id: id,
                    to: burrow_core::Location {
                        space_id: None,
                        section,
                        parent: None,
                        index: (section == Section::Today).then_some(0),
                    },
                });
            }
            Shortcut::ToggleSidebar => self.toggle_sidebar(),
            Shortcut::ReopenClosed => self.command(Command::ReopenClosed),
            Shortcut::Find => {
                if self.shown.is_some() {
                    if !self.sidebar_visible {
                        self.toggle_sidebar();
                    }
                    self.sidebar.request_focus();
                    self.run_in_sidebar("window.burrow && burrow.findMode()");
                }
            }
            Shortcut::DevTools => self.act(Act::DevTools { sidebar: false }),
            Shortcut::PictureInPicture => {
                if let Some(id) = self.shown.clone() {
                    self.toggle_pip(&id);
                }
            }
            Shortcut::SidebarDevTools => self.act(Act::DevTools { sidebar: true }),
            Shortcut::Zoom(dir) => {
                self.with_active_browser(|b| {
                    if let Some(h) = b.host() {
                        h.set_zoom_level(if dir == 0 { 0.0 } else { h.zoom_level() + 0.5 * dir as f64 });
                    }
                });
                self.remember_zoom();
            }
        }
    }

    // ---- Events from tab browsers -----------------------------------------------------------

    /// Finds the tab that owns a browser, remembering the answer.
    fn tab_of_browser(&mut self, browser_id: i32) -> Option<Id> {
        if let Some((id, _)) = self.tabs.iter().find(|(_, t)| t.browser_id == Some(browser_id)) {
            return Some(id.clone());
        }
        let (id, tab) = self
            .tabs
            .iter_mut()
            .find(|(_, t)| t.view.browser().is_some_and(|b| b.identifier() == browser_id))?;
        tab.browser_id = Some(browser_id);
        Some(id.clone())
    }

    pub fn on_title(&mut self, browser_id: i32, title: String) {
        if self.is_peek(browser_id) {
            if let Some(url) = self.peek.as_ref().map(|p| p.url.clone()) {
                self.history.set_title(&url, &title);
            }
            return self.on_peek_title(&title);
        }
        let Some(id) = self.tab_of_browser(browser_id) else { return };
        if let Some(t) = self.state.tab_mut(&id) {
            self.history.set_title(&t.url, &title);
            t.title = title;
        }
        self.apply_theme();
        self.changed();
    }

    pub fn on_address(&mut self, browser_id: i32, url: String) {
        if self.is_peek(browser_id) {
            self.history.record(&url, "", now_ms());
            self.schedule_history_save();
            return self.on_peek_address(url);
        }
        let Some(id) = self.tab_of_browser(browser_id) else { return };
        let mut new_site = false;
        if let Some(t) = self.state.tab_mut(&id) {
            new_site = links::host_of(&t.url) != links::host_of(&url);
            t.url = url.clone();
        }
        let title = self.state.tab(&id).map(|t| t.title.clone()).unwrap_or_default();
        self.history.record(&url, if new_site { "" } else { &title }, now_ms());
        self.schedule_history_save();
        if new_site {
            self.apply_zoom(&id);
        }
        self.changed();
    }

    pub fn on_favicon(&mut self, browser_id: i32, url: String) {
        let Some(id) = self.tab_of_browser(browser_id) else { return };
        if let Some(t) = self.state.tab_mut(&id) {
            t.favicon = Some(url.clone());
        }
        if let Some(page) = self.state.tab(&id).map(|t| t.url.clone()) {
            self.on_page_icon(&page, url);
        }
        self.changed();
    }

    pub fn on_loading(&mut self, browser_id: i32, loading: bool, can_back: bool, can_forward: bool) {
        let Some(id) = self.tab_of_browser(browser_id) else { return };
        if let Some(t) = self.tabs.get_mut(&id) {
            t.nav = Nav { loading, can_back, can_forward, ..t.nav };
            if !loading {
                t.loads += 1;
            }
        }
        if !loading {
            let host = self.state.tab(&id).map(|t| links::host_of(&t.url)).unwrap_or_default();
            if self.state.boosts.contains_key(&host) {
                self.apply_boost(&id);
            }
            self.apply_zoom(&id);
        }
        self.push_state();
    }

    /// A link wants a new tab (target=_blank, Ctrl+click, the page menu) or came from another
    /// program. Air Traffic Control may send it to another space; otherwise it may open in a
    /// Peek (D-021, D-022).
    pub fn open_link(&mut self, from_browser: Option<i32>, url: String, kind: LinkKind) {
        let from_peek = from_browser.is_some_and(|id| self.is_peek(id));
        let source = from_browser.and_then(|id| self.tab_of_browser(id));
        let source = source.or_else(|| from_peek.then(|| self.shown.clone()).flatten());
        let kind = if from_peek && kind == LinkKind::NewTab { LinkKind::ForegroundTab } else { kind };
        match self.state.link_destination(source.as_deref(), &url, kind) {
            Destination::Peek => self.open_peek(url, source),
            Destination::Tab { space_id, foreground, routed } => {
                if routed {
                    let name = self.state.space(&space_id).map(|s| s.name.clone()).unwrap_or_default();
                    self.toast = Some(if foreground {
                        format!("Opened in {name} (Air Traffic Control)")
                    } else {
                        format!("Sent to {name} (Air Traffic Control)")
                    });
                }
                self.open_in_space(url, &space_id, foreground);
            }
        }
    }

    /// Opens a Today tab at the top of a space. A background tab loads right away, hidden.
    fn open_in_space(&mut self, url: String, space_id: &str, foreground: bool) {
        let to = Location { space_id: Some(space_id.to_string()), section: Section::Today, parent: None, index: Some(0) };
        match self.state.open_tab(url, Some(to), foreground) {
            Ok((id, effects)) => {
                for e in effects {
                    self.effect(e);
                }
                if foreground && self.state.active_space != space_id {
                    self.apply_theme();
                }
                self.ensure_live(&id);
                self.sync_view();
                self.changed();
            }
            Err(e) => eprintln!("Burrow: {e}"),
        }
    }

    /// "Open Link in Peek" from the page menu.
    pub fn peek_link(&mut self, from_browser: Option<i32>, url: String) {
        let source = from_browser.and_then(|id| self.tab_of_browser(id));
        self.open_peek(url, source.or_else(|| self.shown.clone()));
    }

    /// A tab in the active space, for links that must not peek (e.g. a Peek that is closing).
    fn open_link_as_tab(&mut self, url: String, background: bool) {
        let space_id = self.state.active_space.clone();
        self.open_in_space(url, &space_id, !background);
    }

    /// A link from another program (Burrow is the default browser): its own tab, in the space
    /// Air Traffic Control picks, and the window comes to the front.
    pub fn open_external(&mut self, url: String) {
        self.open_link(None, url, LinkKind::External);
        self.bring_to_front();
    }

    /// Whether a link click in a pinned tab should open in a Peek instead (CEF asks before the
    /// navigation starts, so this answers synchronously).
    pub fn intercept_navigation(&mut self, browser_id: i32, from_url: &str, to_url: &str) -> bool {
        if self.is_peek(browser_id) {
            return false;
        }
        let Some(id) = self.tab_of_browser(browser_id) else { return false };
        if !self.state.peeks_navigation(&id, from_url, to_url) || links::looks_like_sign_in(to_url) {
            return false;
        }
        let url = to_url.to_string();
        later(move |b| b.open_peek(url, Some(id)));
        true
    }

    /// `window.open` popups keep their opener (needed for sign-in flows). They show as a Peek,
    /// or become tabs when Peek is off or the popup came from a Peek.
    pub fn adopt_popup(&mut self, opener: Option<i32>, view: BrowserView) {
        let from_peek = opener.is_some_and(|id| self.is_peek(id));
        let source = opener.and_then(|id| self.tab_of_browser(id));
        if self.state.settings.peek_new_tabs && !from_peek && self.adopt_popup_as_peek(view.clone(), source) {
            return;
        }
        self.state
            .apply(Command::NewTab { url: "about:blank".into() })
            .expect("new tab never fails");
        let id = self.state.active_tab().map(|t| t.id.clone()).expect("new tab is active");
        self.adopt_view(id, view);
        self.sync_view();
        self.changed();
    }

    /// A browser went away on its own (e.g. the page called `window.close()`).
    pub fn on_browser_closed(&mut self, browser_id: i32) {
        if let Some(i) = self.dying_peeks.iter().position(|p| *p == browser_id) {
            self.dying_peeks.remove(i);
            return self.maybe_finish_close();
        }
        if self.is_peek(browser_id) {
            return self.on_peek_closed();
        }
        if self.is_extension_popup(browser_id) {
            return self.on_extension_popup_closed();
        }
        let Some(id) = self.tab_of_browser(browser_id) else { return };
        self.tabs.remove(&id);
        if self.closing {
            return self.maybe_finish_close();
        }
        if self.shown.as_deref() == Some(&*id) {
            self.shown = None;
        }
        // Same as the user closing it: Today tabs go away; pinned ones unload and return home.
        self.command(Command::CloseTab { tab_id: id });
    }

    /// A page asked to close (`window.close()`, e.g. a finished sign-in popup).
    pub fn on_close_requested(&mut self, browser_id: i32) {
        if self.is_peek(browser_id) {
            return self.close_peek();
        }
        if self.is_extension_popup(browser_id) {
            return self.close_extension_popup();
        }
        if let Some(id) = self.tab_of_browser(browser_id) {
            self.command(Command::CloseTab { tab_id: id });
        }
    }

    /// In split view, clicking into the other half makes it the active tab.
    pub fn on_tab_focused(&mut self, browser_id: i32) {
        // Clicking anywhere else closes an extension popup, as in Chrome.
        if !self.is_extension_popup(browser_id) {
            self.close_extension_popup();
        }
        let Some(id) = self.tab_of_browser(browser_id) else { return };
        if self.visible.len() > 1 && self.visible.contains(&id) && self.shown.as_deref() != Some(&*id) {
            self.state.active_space_mut().active_tab = Some(id.clone());
            self.shown = Some(id);
            self.apply_theme();
            self.changed();
        }
    }

    pub fn on_download(&mut self, d: (u32, String, String, i32, bool, bool)) {
        let (id, name, path, percent, done, canceled) = d;
        let entry = Download { id, name, path, percent, done, canceled };
        match self.downloads.iter_mut().find(|x| x.id == id) {
            Some(x) => *x = entry,
            None => self.downloads.insert(0, entry),
        }
        self.downloads.truncate(20);
        self.push_state();
    }

    // ---- Permissions, find, search -----------------------------------------------------------

    /// A page asked for a permission. Answers from memory, or queues a prompt in the sidebar.
    pub fn request_permission(
        &mut self,
        browser_id: i32,
        prompt_id: Option<u64>,
        origin: String,
        names: Vec<&'static str>,
        callback: PermissionCallback,
    ) {
        let Some(tab_id) = self.tab_of_browser(browser_id) else {
            return callback.resolve(false);
        };
        let profile_id = self.state.profile_of_tab(&tab_id).cloned().unwrap_or_default();
        let origin = origin.trim_end_matches('/').to_string();
        if let Some(allow) = self.state.permission_decision(&profile_id, &origin, &names) {
            return callback.resolve(allow);
        }
        let id = prompt_id.unwrap_or_else(|| {
            self.next_media_request += 1;
            self.next_media_request
        });
        self.permissions.push(PendingPermission { id, tab_id, profile_id, origin, names, callback });
        self.push_state();
    }

    fn answer_permission(&mut self, id: u64, allow: bool, remember: bool) {
        let Some(i) = self.permissions.iter().position(|p| p.id == id) else { return };
        let p = self.permissions.remove(i);
        p.callback.resolve(allow);
        if remember {
            self.state.remember_permission(&p.profile_id, &p.origin, &p.names, allow);
            self.schedule_save();
        }
        self.push_state();
    }

    pub fn dismiss_permission(&mut self, prompt_id: u64) {
        self.permissions.retain(|p| p.id != prompt_id);
        self.push_state();
    }

    pub fn on_find_result(&mut self, count: i32, current: i32) {
        self.find = FindState { count, current };
        self.push_state();
    }

    pub fn search_in_new_tab(&mut self, text: &str) {
        let url = self.state.settings.search_url.replace("{}", &burrow_core::omnibox::encode_query(text.trim()));
        self.open_link_as_tab(url, false);
    }

    // ---- Sidebar rendering ------------------------------------------------------------------

    /// Sends the whole state to the sidebar, at most once per UI-thread turn.
    fn push_state(&mut self) {
        if !self.sidebar_ready || self.push_pending {
            return;
        }
        self.push_pending = true;
        later(|b| {
            b.push_pending = false;
            #[derive(Serialize)]
            struct View<'a> {
                state: &'a State,
                live: HashMap<&'a str, Nav>,
                shown: Option<&'a str>,
                downloads: &'a [Download],
                toast: Option<String>,
                arc_available: bool,
                permissions: Vec<PermissionView<'a>>,
                find: FindState,
                frameless: bool,
                maximized: bool,
                sidebar_collapsed: bool,
                peeking: bool,
                agent_active: bool,
                icons: icons::IconsView<'a>,
                /// The tabs in the content area (two in split view).
                visible: &'a [Id],
                /// The shown tab's zoom in percent.
                zoom: i32,
                peek: Option<&'a str>,
                fullscreen: bool,
                windows: bool,
                pip: Option<&'a str>,
                update: update::UpdateView<'a>,
                extensions: extensions::ExtensionsView<'a>,
            }
            let view = View {
                state: &b.state,
                live: b.tabs.iter().map(|(id, t)| (id.as_str(), t.nav)).collect(),
                shown: b.shown.as_deref(),
                downloads: &b.downloads,
                toast: b.toast.take(),
                arc_available: b.arc_available,
                permissions: b
                    .permissions
                    .iter()
                    .map(|p| PermissionView {
                        id: p.id,
                        origin: &p.origin,
                        names: &p.names,
                        tab_title: b.state.tab(&p.tab_id).map(|t| t.display_title().to_string()).unwrap_or_default(),
                    })
                    .collect(),
                find: b.find,
                frameless: b.frameless(),
                maximized: b.window.as_ref().is_some_and(|w| w.is_maximized() != 0),
                sidebar_collapsed: !b.sidebar_shown(),
                peeking: b.peeking,
                agent_active: b.agent_active,
                icons: b.icons.view(),
                visible: &b.visible,
                zoom: b
                    .shown
                    .as_ref()
                    .and_then(|id| b.tabs.get(id))
                    .and_then(|t| t.view.browser())
                    .and_then(|br| br.host())
                    .map_or(100, |h| (1.2f64.powf(h.zoom_level()) * 100.0).round() as i32),
                peek: b.peek.as_ref().map(|p| p.url.as_str()),
                fullscreen: b.fullscreen,
                windows: cfg!(target_os = "windows"),
                pip: b.pip_tab.as_deref(),
                update: b.update_view(),
                extensions: b.extensions_view(),
            };
            match serde_json::to_string(&view) {
                Ok(json) => {
                    let js = format!("window.burrow && burrow.render({json})");
                    b.run_in_sidebar(&js);
                    if b.modal_open() {
                        b.run_in_modal(js);
                    }
                }
                Err(e) => eprintln!("Burrow: {e}"),
            }
        });
    }

    fn run_in_sidebar(&self, js: &str) {
        if let Some(frame) = self.sidebar.browser().and_then(|b| b.main_frame()) {
            let url = CefString::from(&frame.url());
            frame.execute_java_script(Some(&cef(js)), Some(&url), 0);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shortcut {
    NewTab,
    FocusAddress,
    CloseTab,
    Reload,
    Back,
    Forward,
    Space(usize),
    CycleTab(i32),
    TogglePin,
    ToggleSidebar,
    DevTools,
    SidebarDevTools,
    Zoom(i32),
    ReopenClosed,
    Find,
    /// Escape while a Peek or a fullscreen page is open.
    Escape,
    Fullscreen,
    CopyUrl,
    Print,
    History,
    PictureInPicture,
}

fn open_path(path: &str) {
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("explorer").arg(path).spawn();
    #[cfg(not(target_os = "windows"))]
    let _ = std::process::Command::new("xdg-open").arg(path).spawn();
}
