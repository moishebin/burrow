//! Updates (D-037). Burrow asks GitHub for the latest release soon after starting and every six
//! hours. A copy installed by the Windows setup downloads the new setup in the background,
//! checks it against the release's `SHA256SUMS.txt` and offers "Restart to Update"; otherwise
//! the update installs when Burrow quits. Other copies (the zip, Linux, development builds) only
//! say that a new version is out, with a link.

use std::{
    io::Write,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

use burrow_core::update::{self as up, Release};
use cef::*;
use serde::Serialize;

use super::{cef, later, now_ms, tools::Done, Burrow};
use crate::{paths, task};

const FIRST_CHECK_MS: i64 = 20_000;
const CHECK_EVERY_MS: u64 = 6 * 60 * 60 * 1000;
/// GitHub's JSON and the checksum list are small; anything bigger is wrong.
const MAX_SMALL_BYTES: usize = 1 << 20;
const MAX_SETUP_BYTES: usize = 1 << 30;

/// A verified installer waiting to run when Burrow exits, and whether to start Burrow after.
static PENDING: Mutex<Option<PathBuf>> = Mutex::new(None);
static RELAUNCH: AtomicBool = AtomicBool::new(false);

#[derive(Default)]
pub(super) struct Updater {
    checking: bool,
    downloading: bool,
    /// The newest release, when it is newer than this Burrow.
    available: Option<Release>,
    /// Its setup, downloaded and checked.
    ready: Option<PathBuf>,
    error: Option<String>,
    last_check_ms: u64,
    /// The latest manual check wants a toast with the answer.
    report: bool,
}

#[derive(Serialize)]
pub(super) struct UpdateView<'a> {
    current: &'static str,
    available: Option<&'a str>,
    page: Option<&'a str>,
    /// This copy can install updates itself (it came from the setup).
    installable: bool,
    downloading: bool,
    ready: bool,
    checking: bool,
}

pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// A copy the setup installed: Windows, with the setup's uninstaller next to the exe.
fn installable() -> bool {
    cfg!(target_os = "windows")
        && std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("uninstall.exe").exists())).unwrap_or(false)
}

/// GitHub's API, or `BURROW_UPDATE_URL` (a test server answering the same JSON).
fn release_url() -> String {
    std::env::var("BURROW_UPDATE_URL").unwrap_or_else(|_| up::LATEST_RELEASE_URL.to_string())
}

fn updates_dir() -> PathBuf {
    paths::data_dir().join("updates")
}

/// Called once `main` has shut CEF down: runs a waiting installer, which waits for this process
/// to end. It starts Burrow again if the user chose "Restart to Update" (or `relaunch`, another
/// restart was asked for). True if the installer started.
pub fn run_pending_install(relaunch: bool) -> bool {
    let Some(setup) = PENDING.lock().ok().and_then(|mut p| p.take()) else { return false };
    let mut cmd = std::process::Command::new(&setup);
    cmd.arg("/S");
    if relaunch || RELAUNCH.load(Ordering::Relaxed) {
        cmd.arg("/RELAUNCH");
    }
    // Into this copy's folder, whatever the registry remembers. NSIS wants `/D=` last and
    // unquoted, even with spaces.
    #[cfg(target_os = "windows")]
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.to_path_buf())) {
        use std::os::windows::process::CommandExt;
        cmd.raw_arg(format!("/D={}", dir.display()));
    }
    match cmd.spawn() {
        Ok(_) => true,
        Err(e) => {
            eprintln!("Burrow: could not start the update {}: {e}", setup.display());
            false
        }
    }
}

// ---- Downloads ------------------------------------------------------------------------------

type Fetched = Box<dyn FnOnce(&mut Burrow, Result<Vec<u8>, String>) + Send>;

enum Sink {
    Memory(Vec<u8>),
    File(std::fs::File, usize),
}

wrap_urlrequest_client! {
    struct Fetch {
        sink: Arc<Mutex<Option<Sink>>>,
        limit: usize,
        failed: Arc<AtomicBool>,
        done: Arc<Mutex<Option<Fetched>>>,
    }
    impl UrlrequestClient {
        fn on_download_data(&self, _request: Option<&mut Urlrequest>, data: *const u8, data_length: usize) {
            if data.is_null() {
                return;
            }
            // Safe: CEF hands us `data_length` readable bytes for the duration of the call.
            let bytes = unsafe { std::slice::from_raw_parts(data, data_length) };
            let Ok(mut sink) = self.sink.lock() else { return };
            let ok = match sink.as_mut() {
                Some(Sink::Memory(v)) if v.len() + bytes.len() <= self.limit => {
                    v.extend_from_slice(bytes);
                    true
                }
                Some(Sink::File(f, n)) if *n + bytes.len() <= self.limit => {
                    *n += bytes.len();
                    f.write_all(bytes).is_ok()
                }
                _ => false,
            };
            if !ok {
                self.failed.store(true, Ordering::Relaxed);
            }
        }

        fn on_request_complete(&self, request: Option<&mut Urlrequest>) {
            let status = request.as_ref().map(|r| r.request_status());
            let http = request.as_ref().and_then(|r| r.response()).map_or(0, |r| r.status());
            let sink = self.sink.lock().ok().and_then(|mut s| s.take());
            let result = if status != Some(UrlrequestStatus::SUCCESS) {
                Err("couldn't reach the server".to_string())
            } else if http != 200 {
                Err(format!("the server answered {http}"))
            } else if self.failed.load(Ordering::Relaxed) {
                Err("the download was too large or couldn't be saved".to_string())
            } else {
                match sink {
                    Some(Sink::Memory(v)) => Ok(v),
                    Some(Sink::File(f, _)) => f.sync_all().map(|_| Vec::new()).map_err(|e| e.to_string()),
                    None => Err("nothing was downloaded".to_string()),
                }
            };
            if let Some(done) = self.done.lock().ok().and_then(|mut d| d.take()) {
                later(move |b| done(b, result));
            }
        }
    }
}

/// GETs `url` into memory, or into `file` (then the answer's bytes are empty).
fn fetch(url: &str, file: Option<std::fs::File>, limit: usize, done: Fetched) {
    let done = Arc::new(Mutex::new(Some(done)));
    let started = (|| {
        let mut request = request_create()?;
        request.set_url(Some(&cef(url)));
        request.set_method(Some(&cef("GET")));
        let sink = match file {
            Some(f) => Sink::File(f, 0),
            None => Sink::Memory(Vec::new()),
        };
        let mut client = Fetch::new(Arc::new(Mutex::new(Some(sink))), limit, Arc::new(AtomicBool::new(false)), done.clone());
        urlrequest_create(Some(&mut request), Some(&mut client), None)
    })();
    if started.is_none()
        && let Some(done) = done.lock().ok().and_then(|mut d| d.take())
    {
        later(move |b| done(b, Err("the download couldn't start".into())));
    }
}

impl Burrow {
    /// Starts the automatic checks (not in development builds, which have nothing to update to).
    pub(super) fn schedule_update_checks(&mut self) {
        if cfg!(debug_assertions) && std::env::var_os("BURROW_UPDATE_URL").is_none() {
            return;
        }
        task::post_ui_delayed(FIRST_CHECK_MS, || {
            super::with(|b| b.update_tick());
        });
    }

    fn update_tick(&mut self) {
        if self.state.settings.auto_update && now_ms().saturating_sub(self.updater.last_check_ms) >= CHECK_EVERY_MS - 60_000 {
            self.check_for_update(false, None);
        }
        task::post_ui_delayed(CHECK_EVERY_MS as i64, || {
            super::with(|b| b.update_tick());
        });
    }

    /// Asks GitHub for the latest release. `manual` reports the answer in a toast.
    pub(super) fn check_for_update(&mut self, manual: bool, done: Option<Done>) {
        self.updater.report |= manual;
        if self.updater.checking || self.updater.downloading {
            if let Some(done) = done {
                done(self, Ok(self.update_status()));
            }
            return;
        }
        self.updater.checking = true;
        self.updater.last_check_ms = now_ms();
        self.push_state();
        fetch(
            &release_url(),
            None,
            MAX_SMALL_BYTES,
            Box::new(move |b, r| {
                b.updater.checking = false;
                let found = r.and_then(|bytes| up::newer_release(&String::from_utf8_lossy(&bytes), current_version()));
                match found {
                    Ok(Some(release)) => b.on_release_found(release),
                    Ok(None) => {
                        if std::mem::take(&mut b.updater.report) {
                            b.toast = Some(format!("Burrow is up to date ({}).", current_version()));
                        }
                    }
                    Err(e) => {
                        b.updater.error = Some(e.clone());
                        if std::mem::take(&mut b.updater.report) {
                            b.toast = Some(format!("Couldn't check for updates: {e}"));
                        }
                    }
                }
                b.push_state();
                if let Some(done) = done {
                    let status = b.update_status();
                    done(b, Ok(status));
                }
            }),
        );
    }

    fn on_release_found(&mut self, release: Release) {
        let same = self.updater.available.as_ref().is_some_and(|r| r.version == release.version);
        if same && (self.updater.ready.is_some() || self.updater.downloading) {
            self.updater.report = false;
            return;
        }
        let installable = installable() && release.setup.is_some() && release.sums.is_some();
        self.updater.available = Some(release);
        if installable {
            self.download_update();
        } else if std::mem::take(&mut self.updater.report) {
            let v = self.updater.available.as_ref().map(|r| r.version.clone()).unwrap_or_default();
            self.toast = Some(format!("Burrow {v} is out."));
        }
    }

    /// Downloads the checksums, then the setup into `<data>/updates/`, then checks it.
    fn download_update(&mut self) {
        let Some(release) = self.updater.available.clone() else { return };
        let (Some(setup), Some(sums)) = (release.setup.clone(), release.sums.clone()) else { return };
        let dir = updates_dir();
        // Older downloads are no use now.
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join(&setup.name);
        let file = std::fs::create_dir_all(&dir).and_then(|_| std::fs::File::create(&path));
        let file = match file {
            Ok(f) => f,
            Err(e) => return self.update_failed(format!("couldn't save the update: {e}")),
        };
        self.updater.downloading = true;
        self.push_state();
        fetch(
            &sums.url,
            None,
            MAX_SMALL_BYTES,
            Box::new(move |b, r| {
                let sums = match r {
                    Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
                    Err(e) => return b.update_failed(e),
                };
                fetch(
                    &setup.url,
                    Some(file),
                    MAX_SETUP_BYTES,
                    Box::new(move |b, r| {
                        if let Err(e) = r {
                            return b.update_failed(e);
                        }
                        // Hashing ~150 MB takes a moment: not on the UI thread.
                        std::thread::spawn(move || {
                            let checked = std::fs::read(&path)
                                .map_err(|e| e.to_string())
                                .and_then(|bytes| up::verify(&bytes, &sums, &setup.name));
                            later(move |b| match checked {
                                Ok(()) => b.on_update_ready(path),
                                Err(e) => {
                                    let _ = std::fs::remove_file(&path);
                                    b.update_failed(e);
                                }
                            });
                        });
                    }),
                );
            }),
        );
    }

    fn on_update_ready(&mut self, setup: PathBuf) {
        self.updater.downloading = false;
        self.updater.ready = Some(setup.clone());
        self.updater.error = None;
        // Installs when Burrow quits, unless the user restarts for it first.
        if let Ok(mut p) = PENDING.lock() {
            *p = Some(setup);
        }
        self.updater.report = false;
        self.push_state();
    }

    fn update_failed(&mut self, error: String) {
        eprintln!("Burrow: update failed: {error}");
        self.updater.downloading = false;
        self.updater.error = Some(error.clone());
        if std::mem::take(&mut self.updater.report) {
            self.toast = Some(format!("The update didn't download: {error}"));
        }
        self.push_state();
    }

    /// "Restart to Update": closes the window gracefully; `main` runs the setup, which starts
    /// Burrow again.
    pub(super) fn install_update(&mut self) -> Result<(), String> {
        if self.updater.ready.is_none() {
            return Err("No update is ready.".into());
        }
        RELAUNCH.store(true, Ordering::Relaxed);
        task::post_ui(|| {
            if let Some(w) = super::with(|b| b.window.clone()).flatten() {
                w.close();
            }
        });
        Ok(())
    }

    pub(super) fn update_view(&self) -> UpdateView<'_> {
        let u = &self.updater;
        UpdateView {
            current: current_version(),
            available: u.available.as_ref().map(|r| r.version.as_str()),
            page: u.available.as_ref().map(|r| r.page.as_str()),
            installable: installable(),
            downloading: u.downloading,
            ready: u.ready.is_some(),
            checking: u.checking,
        }
    }

    pub(super) fn update_status(&self) -> serde_json::Value {
        let u = &self.updater;
        serde_json::json!({
            "current": current_version(),
            "available": u.available.as_ref().map(|r| &r.version),
            "release_page": u.available.as_ref().map(|r| &r.page),
            "can_install": installable(),
            "downloading": u.downloading,
            "ready_to_install": u.ready.is_some(),
            "checking": u.checking,
            "error": u.error,
            "auto_check": self.state.settings.auto_update,
        })
    }

    /// The `update` agent tool.
    pub(super) fn agent_update(&mut self, args: &serde_json::Value, done: Done) {
        if let Some(on) = args.get("auto").and_then(serde_json::Value::as_bool) {
            self.state.settings.auto_update = on;
            self.changed();
        }
        match args.get("action").and_then(serde_json::Value::as_str).unwrap_or("status") {
            "status" => done(self, Ok(self.update_status())),
            "check" => self.check_for_update(false, Some(done)),
            "install" => {
                let r = self.install_update().map(|_| serde_json::json!({ "restarting": true }));
                done(self, r)
            }
            a => done(self, Err(format!("unknown action `{a}` (use status, check or install)"))),
        }
    }
}
