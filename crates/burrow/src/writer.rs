//! Writes Burrow's data files off the UI thread, so a slow disk (or a virus scanner) never
//! stalls the window (D-034).

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use burrow_core::store;

static NEXT: AtomicU64 = AtomicU64::new(1);
/// The newest version written to each file. Writes hold this lock, so two never overlap.
static WRITTEN: Mutex<BTreeMap<PathBuf, u64>> = Mutex::new(BTreeMap::new());

/// Writes `contents` to `path` on a background thread.
pub fn write_soon(path: PathBuf, contents: String) {
    let version = NEXT.fetch_add(1, Ordering::Relaxed);
    std::thread::spawn(move || write(&path, &contents, version));
}

/// Writes `contents` to `path` before returning (on shutdown).
pub fn write_now(path: &Path, contents: &str) {
    write(path, contents, NEXT.fetch_add(1, Ordering::Relaxed));
}

fn write(path: &Path, contents: &str, version: u64) {
    let mut written = WRITTEN.lock().unwrap_or_else(|e| e.into_inner());
    if written.get(path).is_some_and(|&v| v > version) {
        return; // a newer version is already on disk
    }
    match store::write_atomic(path, contents) {
        Ok(()) => {
            written.insert(path.to_path_buf(), version);
        }
        Err(e) => eprintln!("Burrow: could not save {}: {e}", path.display()),
    }
}
