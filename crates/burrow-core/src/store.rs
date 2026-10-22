//! Loading and saving `state.json` (D-005).

use std::{fs, io, path::Path};

use crate::model::State;

/// Loads the state. A missing file gives a fresh default state. A file that can't be parsed
/// is kept as `state.json.corrupt-<ms>` and a fresh state is returned, so a bad write never
/// loses the user's sidebar silently.
pub fn load(path: &Path) -> io::Result<State> {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(State::default()),
        Err(e) => return Err(e),
    };
    match serde_json::from_str::<State>(&text) {
        Ok(mut state) => {
            state.normalize();
            Ok(state)
        }
        Err(_) => {
            let ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0);
            let mut backup = path.as_os_str().to_owned();
            backup.push(format!(".corrupt-{ms}"));
            fs::rename(path, &backup)?;
            Ok(State::default())
        }
    }
}

/// Writes the state atomically: to a temp file next to it, then renamed over it.
pub fn save(path: &Path, state: &State) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_string_pretty(state).map_err(io::Error::other)?;
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    fs::write(&tmp, json)?;
    fs::rename(&tmp, path)
}
