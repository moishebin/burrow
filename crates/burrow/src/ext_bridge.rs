//! The extension bridge (D-041): the local endpoint that Burrow's script in each extension's
//! background (`burrow_core::extensions::bridge_hook`) talks to. It reports badges and asks for
//! tabs and windows, which Burrow's tabs can't give the extension APIs directly.
//!
//! Its port and secret are chosen once and kept in `<data>/bridge.json`: the hook is written
//! into the extensions' folders before Chromium starts, and a second `burrow.exe` (which runs
//! that code too before handing over to the running one) must write the very same hook.
//! Only requests from an extension (`Origin: chrome-extension://<id>`) that carry the secret
//! are served; the extension's id comes from the Origin, which pages can't fake.

use std::{
    net::{TcpListener, TcpStream},
    path::PathBuf,
    sync::OnceLock,
    time::Duration,
};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{agent, paths};

#[derive(Clone, Serialize, Deserialize)]
struct Config {
    port: u16,
    secret: String,
}

static CONFIG: OnceLock<Config> = OnceLock::new();

fn config_file() -> PathBuf {
    paths::data_dir().join("bridge.json")
}

fn new_config() -> Config {
    // A free port now; it is very likely still free at the next start.
    let port = TcpListener::bind("127.0.0.1:0").and_then(|l| l.local_addr()).map_or(0, |a| a.port());
    Config { port, secret: burrow_core::model::new_id().replace('-', "") }
}

fn config() -> &'static Config {
    CONFIG.get_or_init(|| {
        let saved = std::fs::read(config_file()).ok().and_then(|b| serde_json::from_slice::<Config>(&b).ok());
        saved.filter(|c| c.port != 0).unwrap_or_else(|| {
            let c = new_config();
            save(&c);
            c
        })
    })
}

fn save(c: &Config) {
    let _ = std::fs::create_dir_all(paths::data_dir());
    if let Err(e) = std::fs::write(config_file(), serde_json::to_vec_pretty(c).unwrap_or_default()) {
        eprintln!("Burrow: could not write {}: {e}", config_file().display());
    }
}

/// The script added to every extension's background.
pub fn hook() -> String {
    let c = config();
    burrow_core::extensions::bridge_hook(c.port, &c.secret)
}

/// Serves the bridge. If its port is taken, it picks another for the next start (extensions get
/// the new hook then; until a restart they can't reach Burrow).
pub fn start() {
    let c = config().clone();
    let listener = match TcpListener::bind(("127.0.0.1", c.port)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Burrow: extension bridge port {} unavailable ({e}); a new one is used after a restart", c.port);
            save(&new_config());
            return;
        }
    };
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let c = c.clone();
            std::thread::spawn(move || serve(stream, &c));
        }
    });
}

/// `chrome-extension://<32 letters a–p>` → the id.
fn extension_id(origin: &str) -> Option<String> {
    let id = origin.strip_prefix("chrome-extension://")?.trim_end_matches('/');
    (id.len() == 32 && id.bytes().all(|b| (b'a'..=b'p').contains(&b))).then(|| id.to_string())
}

fn serve(mut stream: TcpStream, c: &Config) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let fail = |stream: &mut TcpStream, status: u16, error: &str| agent::respond_with(stream, status, &json!({ "ok": false, "error": error }), "");
    let Ok(req) = agent::read_request(&mut stream) else { return fail(&mut stream, 400, "bad request") };
    let host_ok = req.header("host").is_some_and(|h| h == format!("127.0.0.1:{}", c.port));
    let origin = req.header("origin").unwrap_or_default().to_string();
    let Some(ext) = extension_id(&origin).filter(|_| host_ok && req.method == "POST" && req.path == "/ext") else {
        return fail(&mut stream, 403, "forbidden");
    };
    let Ok(msg) = serde_json::from_slice::<Value>(&req.body) else { return fail(&mut stream, 400, "bad JSON") };
    if msg["secret"].as_str() != Some(c.secret.as_str()) {
        return fail(&mut stream, 403, "forbidden");
    }
    let result = agent::run_on_ui(move |b, reply| b.extension_bridge(&ext, &msg, reply));
    let body = match result {
        Ok(result) => json!({ "ok": true, "result": result }),
        Err(error) => json!({ "ok": false, "error": error }),
    };
    agent::respond_with(&mut stream, 200, &body, &format!("Access-Control-Allow-Origin: {origin}\r\n"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_extension_origins() {
        assert_eq!(extension_id("chrome-extension://abcdefghijklmnopabcdefghijklmnop").as_deref(), Some("abcdefghijklmnopabcdefghijklmnop"));
        assert_eq!(extension_id("https://example.com"), None);
        assert_eq!(extension_id("chrome-extension://short"), None);
        assert_eq!(extension_id("chrome-extension://zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz"), None);
    }
}
