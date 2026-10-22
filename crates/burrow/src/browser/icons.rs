//! Site icons kept on disk (D-025), so every tab shows its real icon, even pinned tabs and
//! favorites that haven't loaded since Burrow started (e.g. right after an Arc import).
//!
//! A loaded page reports its icon and Burrow saves it. For a site with no saved icon, Burrow
//! fetches the tab's page in its space's profile (so signed-in apps answer as usual), reads the
//! `<link rel=icon>` tags, and saves the best icon that downloads.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{Arc, Mutex},
};

use burrow_core::{favicon, links, Id};
use cef::*;
use serde::Serialize;

use super::{cef, later, Burrow};
use crate::{paths, task};

/// Pages fetched at the same time while looking for icons.
const PARALLEL: usize = 4;
/// Enough of a page to find the `<link>` tags in its `<head>`.
const MAX_PAGE_BYTES: usize = 512 * 1024;
const ICON_PIXELS: u32 = 128;

#[derive(Default)]
pub(super) struct Icons {
    /// Hosts with an icon on disk, and a version that changes when the file does.
    saved: HashMap<String, u64>,
    /// Hosts already looked up (or refreshed from a loaded page) since Burrow started.
    tried: HashSet<String>,
    refreshed: HashSet<String>,
    queue: VecDeque<(String, String, Id)>,
    in_flight: usize,
    scan_pending: bool,
}

/// What the sidebar needs to show saved icons: `base` + `files[host]`.
#[derive(Serialize)]
pub(super) struct IconsView<'a> {
    base: String,
    files: HashMap<&'a str, String>,
}

impl Icons {
    /// Reads which icons are already saved.
    pub(super) fn load() -> Self {
        let mut icons = Icons::default();
        if let Ok(entries) = std::fs::read_dir(paths::favicons_dir()) {
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                let Some(host) = name.strip_suffix(".png") else { continue };
                let version = e
                    .metadata()
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(1, |d| d.as_secs());
                icons.saved.insert(host.to_string(), version);
            }
        }
        icons
    }

    pub(super) fn view(&self) -> IconsView<'_> {
        let base = format!("{}://icons/", crate::scheme::SCHEME);
        let files = self.saved.iter().map(|(h, v)| (h.as_str(), format!("{}?v={v}", favicon::file_name(h)))).collect();
        IconsView { base, files }
    }
}

fn host_key(url: &str) -> Option<String> {
    let host = links::host_of(url);
    (!host.is_empty() && (url.starts_with("https://") || url.starts_with("http://"))).then_some(host)
}

impl Burrow {
    /// Looks for icons of sites that have none saved, soon (debounced).
    pub(super) fn schedule_icon_scan(&mut self) {
        if !self.icons.scan_pending {
            self.icons.scan_pending = true;
            task::post_ui_delayed(1500, || {
                super::with(|b| b.scan_icons());
            });
        }
    }

    fn scan_icons(&mut self) {
        self.icons.scan_pending = false;
        let mut wanted: Vec<(String, String, Id)> = vec![];
        for space in &self.state.spaces {
            for tab in space.tabs() {
                let url = tab.home_url.as_deref().unwrap_or(&tab.url);
                let Some(host) = host_key(url) else { continue };
                if self.icons.saved.contains_key(&host) || self.icons.tried.contains(&host) {
                    continue;
                }
                if !self.contexts.get(&space.profile_id).is_some_and(|p| p.ready) {
                    continue; // asked again once the profile is ready
                }
                self.icons.tried.insert(host.clone());
                wanted.push((host, url.to_string(), space.profile_id.clone()));
            }
        }
        self.icons.queue.extend(wanted);
        self.pump_icons();
    }

    fn pump_icons(&mut self) {
        while self.icons.in_flight < PARALLEL {
            let Some((host, url, profile_id)) = self.icons.queue.pop_front() else { break };
            let Some(mut context) = self.contexts.get(&profile_id).map(|p| p.context.clone()) else { continue };
            self.icons.in_flight += 1;
            if !fetch_page(&url, &mut context, host.clone()) {
                self.icons.in_flight -= 1;
            }
        }
    }

    /// A page was fetched (or failed, with empty HTML): try its icons in order.
    fn on_page_fetched(&mut self, host: String, page_url: String, html: String) {
        let candidates = favicon::candidates(&html, &page_url);
        self.try_icons(host, candidates.into(), true);
    }

    /// A loaded page named its icon: save it, once per site per run (icons change rarely).
    pub(super) fn on_page_icon(&mut self, page_url: &str, icon_url: String) {
        let Some(host) = host_key(page_url) else { return };
        if !self.icons.refreshed.insert(host.clone()) {
            return;
        }
        self.icons.tried.insert(host.clone());
        let fallback = favicon::resolve(page_url, "/favicon.ico").unwrap_or_default();
        self.try_icons(host, VecDeque::from([icon_url, fallback]), false);
    }

    /// Downloads the first icon of `candidates` that works and saves it for `host`.
    fn try_icons(&mut self, host: String, mut candidates: VecDeque<String>, counted: bool) {
        let finish = |b: &mut Burrow| {
            if counted {
                b.icons.in_flight -= 1;
                b.pump_icons();
            }
        };
        let Some(url) = candidates.pop_front().filter(|u| !u.is_empty()) else { return finish(self) };
        let Some(host_api) = self.sidebar.browser().and_then(|b| b.host()) else { return finish(self) };
        let mut callback = IconDownloaded::new(host, Arc::new(Mutex::new(Some((candidates, counted)))));
        host_api.download_image(Some(&cef(&url)), 1, ICON_PIXELS, 0, Some(&mut callback));
    }

    fn on_icon_saved(&mut self, host: String, counted: bool) {
        let version = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(1, |d| d.as_secs());
        self.icons.saved.insert(host, version);
        if counted {
            self.icons.in_flight -= 1;
            self.pump_icons();
        }
        self.push_state();
    }
}

type Rest = Arc<Mutex<Option<(VecDeque<String>, bool)>>>;

wrap_download_image_callback! {
    struct IconDownloaded {
        host: String,
        rest: Rest,
    }
    impl DownloadImageCallback {
        fn on_download_image_finished(
            &self,
            _image_url: Option<&CefString>,
            http_status_code: ::std::os::raw::c_int,
            image: Option<&mut Image>,
        ) {
            let Some((rest, counted)) = self.rest.lock().ok().and_then(|mut r| r.take()) else { return };
            let host = self.host.clone();
            let png = image
                .filter(|i| i.is_empty() == 0 && (http_status_code == 200 || http_status_code == 0))
                .and_then(|i| {
                    // CEF returns nothing unless it can report the size.
                    let (mut w, mut h) = (0, 0);
                    i.as_png(1.0, 1, Some(&mut w), Some(&mut h))
                })
                .map(|bin| {
                    let mut bytes = vec![0u8; bin.size()];
                    bin.data(Some(&mut bytes), 0);
                    bytes
                });
            match png {
                Some(bytes) if !bytes.is_empty() => {
                    let dir = paths::favicons_dir();
                    let saved = std::fs::create_dir_all(&dir)
                        .and_then(|_| std::fs::write(dir.join(favicon::file_name(&host)), &bytes))
                        .is_ok();
                    if saved {
                        later(move |b| b.on_icon_saved(host, counted));
                    } else {
                        later(move |b| b.try_icons(host, VecDeque::new(), counted));
                    }
                }
                _ => later(move |b| b.try_icons(host, rest, counted)),
            }
        }
    }
}

/// Fetches `url` in a profile and reports its HTML to `on_page_fetched`. False if it couldn't
/// start.
fn fetch_page(url: &str, context: &mut RequestContext, host: String) -> bool {
    let Some(mut request) = request_create() else { return false };
    request.set_url(Some(&cef(url)));
    request.set_method(Some(&cef("GET")));
    let mut client = PageFetch::new(host, Arc::new(Mutex::new(Vec::new())));
    urlrequest_create(Some(&mut request), Some(&mut client), Some(context)).is_some()
}

wrap_urlrequest_client! {
    struct PageFetch {
        host: String,
        body: Arc<Mutex<Vec<u8>>>,
    }
    impl UrlrequestClient {
        fn on_download_data(&self, _request: Option<&mut Urlrequest>, data: *const u8, data_length: usize) {
            if data.is_null() {
                return;
            }
            if let Ok(mut body) = self.body.lock()
                && body.len() < MAX_PAGE_BYTES
            {
                // Safe: CEF hands us `data_length` readable bytes for the duration of the call.
                body.extend_from_slice(unsafe { std::slice::from_raw_parts(data, data_length) });
            }
        }

        fn on_request_complete(&self, request: Option<&mut Urlrequest>) {
            let host = self.host.clone();
            let final_url = request
                .and_then(|r| r.response())
                .map(|r| CefString::from(&r.url()).to_string())
                .filter(|u| !u.is_empty())
                .unwrap_or_else(|| format!("https://{host}/"));
            let html = self.body.lock().map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
            later(move |b| b.on_page_fetched(host, final_url, html));
        }
    }
}
