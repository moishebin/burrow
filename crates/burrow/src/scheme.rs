//! `burrow://`: Burrow's own pages and pictures (D-032).
//!
//! - `burrow://ui/<file>`: the sidebar (`ui/` next to the exe).
//! - `burrow://icons/<host>.png`: saved site icons (D-025).
//! - `burrow://ext/<id>/<file>`: files of an installed extension (its icon in the sidebar).
//!
//! The sidebar used to be a `file://` page. Chromium gives unpacked extensions access to file
//! URLs, so their content scripts ran inside the sidebar. No extension can match `burrow://`.
//! The handler is registered for the global request context only, which holds the sidebar;
//! web pages live in the spaces' profiles and can't load these addresses.

use std::path::{Component, Path, PathBuf};

use cef::{wrapper::stream_resource_handler::StreamResourceHandler, *};

use crate::paths;

pub const SCHEME: &str = "burrow";

/// Called in every process before anything loads, so the renderer knows the scheme too.
pub fn register_custom_scheme(registrar: &mut SchemeRegistrar) {
    let options = SchemeOptions::STANDARD.get_raw() | SchemeOptions::SECURE.get_raw();
    registrar.add_custom_scheme(Some(&CefString::from(SCHEME)), options as _);
}

/// Called once in the browser process, before the sidebar loads.
pub fn register_handler() {
    let mut factory = Files::new();
    register_scheme_handler_factory(Some(&CefString::from(SCHEME)), None, Some(&mut factory));
}

/// The file behind a `burrow://` address, if it is one Burrow serves.
fn file_for(url: &str) -> Option<PathBuf> {
    let rest = url.strip_prefix("burrow://")?;
    let rest = rest.split(['?', '#']).next()?;
    let (host, path) = rest.split_once('/')?;
    let path = percent_decode(path)?;
    let rel = Path::new(&path);
    // Only plain names: no `..`, no root, no drive letters.
    if path.is_empty() || !rel.components().all(|c| matches!(c, Component::Normal(_))) {
        return None;
    }
    let base = match host {
        "ui" => paths::ui_dir(),
        "icons" => paths::favicons_dir(),
        "ext" => paths::extensions_dir(),
        _ => return None,
    };
    Some(base.join(rel))
}

fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn mime_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref() {
        Some("html" | "htm") => "text/html",
        Some("css") => "text/css",
        Some("js" | "mjs") => "text/javascript",
        Some("json") => "application/json",
        Some("png") => "image/png",
        Some("svg") => "image/svg+xml",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("ico") => "image/x-icon",
        _ => "application/octet-stream",
    }
}

wrap_scheme_handler_factory! {
    struct Files {}
    impl SchemeHandlerFactory {
        fn create(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            _scheme_name: Option<&CefString>,
            request: Option<&mut Request>,
        ) -> Option<ResourceHandler> {
            let url = CefString::from(&request?.url()).to_string();
            let path = file_for(&url).filter(|p| p.is_file());
            let stream = path.as_ref().and_then(|p| stream_reader_create_for_file(Some(&CefString::from(p.to_string_lossy().as_ref()))));
            Some(match (path, stream) {
                (Some(p), Some(stream)) => StreamResourceHandler::new_with_stream(mime_type(&p).into(), stream),
                _ => StreamResourceHandler::new(404, "Not Found".into(), "text/plain".into(), None, None),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serves_only_plain_paths_under_known_folders() {
        assert_eq!(file_for("burrow://ui/sidebar.html"), Some(paths::ui_dir().join("sidebar.html")));
        assert_eq!(file_for("burrow://icons/github.com.png?v=3"), Some(paths::favicons_dir().join("github.com.png")));
        assert_eq!(
            file_for("burrow://ext/abc/icons/a%20b.png"),
            Some(paths::extensions_dir().join("abc").join("icons").join("a b.png"))
        );
        assert_eq!(file_for("burrow://ui/../state.json"), None);
        assert_eq!(file_for("burrow://ui/%2e%2e/state.json"), None);
        assert_eq!(file_for("burrow://ext/abc/..%2f..%2fstate.json"), None);
        assert_eq!(file_for("burrow://other/x"), None);
        assert_eq!(file_for("burrow://ui/"), None);
        assert_eq!(file_for("https://ui/x"), None);
    }

    #[test]
    fn mime_types() {
        assert_eq!(mime_type(Path::new("a/B.JS")), "text/javascript");
        assert_eq!(mime_type(Path::new("x.png")), "image/png");
        assert_eq!(mime_type(Path::new("x")), "application/octet-stream");
    }
}
