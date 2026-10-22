use std::path::PathBuf;

use crate::consts::APP_NAME;

/// Where Burrow keeps `state.json` and browser profiles.
/// `BURROW_DATA_DIR` overrides it (handy for a throwaway test profile).
pub fn data_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("BURROW_DATA_DIR") {
        return PathBuf::from(d);
    }
    #[cfg(target_os = "windows")]
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    #[cfg(not(target_os = "windows"))]
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")));
    base.unwrap_or_else(std::env::temp_dir).join(APP_NAME)
}

pub fn state_file() -> PathBuf {
    data_dir().join("state.json")
}

pub fn history_file() -> PathBuf {
    data_dir().join("history.json")
}

/// Saved site icons, one PNG per host (D-025).
pub fn favicons_dir() -> PathBuf {
    data_dir().join("favicons")
}

pub fn profile_dir(profile_id: &str) -> PathBuf {
    data_dir().join("profiles").join(profile_id)
}

/// The sidebar's HTML lives in `ui/` next to the executable.
pub fn ui_url(page: &str) -> String {
    let exe = std::env::current_exe().unwrap_or_default();
    let dir = exe.parent().map(|p| p.join("ui")).unwrap_or_else(|| PathBuf::from("ui"));
    let path = dir.join(page).to_string_lossy().replace('\\', "/");
    if path.starts_with('/') { format!("file://{path}") } else { format!("file:///{path}") }
}

/// Arc's sidebar file on Windows, if Arc is installed.
pub fn arc_sidebar_file() -> Option<PathBuf> {
    let packages = PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("Packages");
    std::fs::read_dir(packages)
        .ok()?
        .flatten()
        .find(|e| e.file_name().to_string_lossy().starts_with("TheBrowserCompany.Arc_"))
        .map(|e| e.path().join("LocalCache/Local/Arc/StorableSidebar.json"))
        .filter(|p| p.exists())
}

/// The user's Downloads folder.
pub fn downloads_dir() -> PathBuf {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"));
    home.map(|h| PathBuf::from(h).join("Downloads")).unwrap_or_else(std::env::temp_dir)
}

/// Deletes Chrome's own session files in every profile. Burrow restores tabs from `state.json`;
/// if Chrome restored its session too, old pages would come back as hidden windows.
pub fn clear_chrome_sessions() {
    let root = data_dir().join("profiles");
    let Ok(entries) = std::fs::read_dir(&root) else { return };
    for e in entries.flatten() {
        let sessions = e.path().join("Sessions");
        if sessions.is_dir() {
            let _ = std::fs::remove_dir_all(sessions);
        }
    }
}
