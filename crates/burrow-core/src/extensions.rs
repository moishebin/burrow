//! Chrome extensions (D-032): unpacking `.crx` packages from the Chrome Web Store, reading
//! manifests, and the list of installed extensions kept in `State.extensions`.
//!
//! Burrow's tabs are Alloy-style (D-010), which has no chrome://extensions page and no Web Store
//! install button. Burrow installs extensions itself into `<data>/extensions/<id>/<version>_<n>/`
//! and Chromium loads them at startup with `--load-extension`. Every installed extension gets a
//! `key` in its manifest, so its id is known before Chromium loads it and stays the same across
//! restarts. An update goes into a new folder, so the files of the running version are never
//! touched; old folders are deleted at the next start (`clean_up`).

use std::{
    collections::BTreeMap,
    fs,
    io::{Cursor, Read},
    path::{Path, PathBuf},
};

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// The Chromium version Burrow ships, sent to the Web Store so it picks a compatible package.
pub const CHROMIUM_VERSION: &str = "154.0.8037.98";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Extension {
    /// Chromium's extension id: 32 letters `a`–`p`. Also the folder name.
    pub id: String,
    pub name: String,
    pub version: String,
    /// Its folder, relative to the extensions folder (`<id>/<version>_<n>`, with `/`).
    pub dir: String,
    #[serde(default)]
    pub description: String,
    pub enabled: bool,
    pub source: Source,
    /// The largest icon, relative to the extension's folder.
    #[serde(default)]
    pub icon: Option<String>,
    /// The page shown when its toolbar button is clicked, relative to the folder.
    #[serde(default)]
    pub popup: Option<String>,
    /// Its options page, relative to the folder.
    #[serde(default)]
    pub options: Option<String>,
    /// Shown as a button in the sidebar's toolbar, with its badge (D-041).
    #[serde(default)]
    pub pinned: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// Downloaded from the Chrome Web Store; can be updated from there.
    WebStore,
    /// Installed from a folder or a `.crx`/`.zip` file on disk.
    File,
}

impl Extension {
    /// The address of one of its pages, e.g. its popup.
    pub fn url(&self, page: &str) -> String {
        format!("chrome-extension://{}/{}", self.id, page.trim_start_matches('/'))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ExtensionError {
    #[error("not a Chrome extension package (.crx or .zip)")]
    NotAPackage,
    #[error("the package is damaged: {0}")]
    Damaged(String),
    #[error("no manifest.json in the extension")]
    NoManifest,
    #[error("manifest.json can't be read: {0}")]
    BadManifest(String),
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

/// The address the Web Store serves an extension's newest `.crx` from.
pub fn web_store_download_url(id: &str) -> String {
    format!(
        "https://clients2.google.com/service/update2/crx?response=redirect&prodversion={CHROMIUM_VERSION}\
         &acceptformat=crx2,crx3&x=id%3D{id}%26uc"
    )
}

/// The extension id in a Web Store link (`chromewebstore.google.com/detail/name/<id>`), or a
/// bare id.
pub fn web_store_id(input: &str) -> Option<String> {
    let input = input.trim();
    let candidate = |s: &str| s.len() == 32 && s.bytes().all(|b| (b'a'..=b'p').contains(&b));
    if candidate(input) {
        return Some(input.to_string());
    }
    let lower = input.to_ascii_lowercase();
    let on_store = ["chromewebstore.google.com/", "chrome.google.com/webstore/"]
        .iter()
        .any(|h| lower.trim_start_matches("https://").trim_start_matches("http://").starts_with(h));
    if !on_store {
        return None;
    }
    let path = input.split(['?', '#']).next().unwrap_or_default();
    path.split('/').find(|s| candidate(s)).map(String::from)
}

/// Chromium's id for a public key: the first 16 bytes of its SHA-256, one letter `a`–`p` per
/// half-byte.
pub fn id_from_key(public_key: &[u8]) -> String {
    let hash = Sha256::digest(public_key);
    hash[..16]
        .iter()
        .flat_map(|b| [b >> 4, b & 0xF])
        .map(|n| (b'a' + n) as char)
        .collect()
}

/// A package's zip archive and the public key that identifies it (`None` for a plain zip).
pub struct Package<'a> {
    pub zip: &'a [u8],
    pub public_key: Option<Vec<u8>>,
}

/// Splits a `.crx` (version 2 or 3) into its zip and key. A plain `.zip` passes through.
pub fn parse_package(bytes: &[u8]) -> Result<Package<'_>, ExtensionError> {
    if bytes.starts_with(b"PK") {
        return Ok(Package { zip: bytes, public_key: None });
    }
    if !bytes.starts_with(b"Cr24") || bytes.len() < 16 {
        return Err(ExtensionError::NotAPackage);
    }
    let u32_at = |i: usize| -> Result<usize, ExtensionError> {
        let b = bytes.get(i..i + 4).ok_or_else(|| ExtensionError::Damaged("header cut short".into()))?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
    };
    let cut = || ExtensionError::Damaged("header cut short".into());
    match u32_at(4)? {
        2 => {
            let (key_len, sig_len) = (u32_at(8)?, u32_at(12)?);
            let key = bytes.get(16..16 + key_len).ok_or_else(cut)?;
            let zip = bytes.get(16 + key_len + sig_len..).ok_or_else(cut)?;
            Ok(Package { zip, public_key: Some(key.to_vec()) })
        }
        3 => {
            let header_len = u32_at(8)?;
            let header = bytes.get(12..12 + header_len).ok_or_else(cut)?;
            let zip = bytes.get(12 + header_len..).ok_or_else(cut)?;
            Ok(Package { zip, public_key: crx3_key(header) })
        }
        v => Err(ExtensionError::Damaged(format!("unknown .crx version {v}"))),
    }
}

/// The key of a CRX3 header (a `CrxFileHeader` protobuf) whose id matches the signed id, else
/// its first RSA key.
fn crx3_key(header: &[u8]) -> Option<Vec<u8>> {
    let fields = protobuf_fields(header);
    let signed_id = fields
        .iter()
        .find(|(n, _)| *n == 10000)
        .and_then(|(_, data)| protobuf_fields(data).into_iter().find(|(n, _)| *n == 1))
        .map(|(_, id)| id.to_vec());
    let keys: Vec<&[u8]> = fields
        .iter()
        .filter(|(n, _)| *n == 2 || *n == 3)
        .filter_map(|(_, proof)| protobuf_fields(proof).into_iter().find(|(n, _)| *n == 1).map(|(_, k)| k))
        .collect();
    let signed = keys.iter().find(|k| signed_id.as_deref().is_some_and(|id| Sha256::digest(k)[..16] == *id));
    signed.or(keys.first()).map(|k| k.to_vec())
}

/// The length-delimited fields of a protobuf message, as (field number, bytes). Other wire
/// types are skipped; parsing stops at anything malformed.
fn protobuf_fields(mut data: &[u8]) -> Vec<(u64, &[u8])> {
    fn varint(data: &mut &[u8]) -> Option<u64> {
        let mut v = 0u64;
        for shift in (0..64).step_by(7) {
            let (&b, rest) = data.split_first()?;
            *data = rest;
            v |= u64::from(b & 0x7F) << shift;
            if b & 0x80 == 0 {
                return Some(v);
            }
        }
        None
    }
    let mut out = vec![];
    while !data.is_empty() {
        let Some(tag) = varint(&mut data) else { break };
        match tag & 7 {
            0 => {
                if varint(&mut data).is_none() {
                    break;
                }
            }
            1 if data.len() >= 8 => data = &data[8..],
            2 => {
                let Some(len) = varint(&mut data).map(|l| l as usize) else { break };
                if len > data.len() {
                    break;
                }
                out.push((tag >> 3, &data[..len]));
                data = &data[len..];
            }
            5 if data.len() >= 4 => data = &data[4..],
            _ => break,
        }
    }
    out
}

/// What Burrow shows of a manifest.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ManifestInfo {
    pub name: String,
    pub version: String,
    pub description: String,
    pub icon: Option<String>,
    pub popup: Option<String>,
    pub options: Option<String>,
    pub key: Option<String>,
}

/// Reads the parts of `manifest.json` Burrow needs. `messages` looks up `__MSG_name__`
/// strings in the extension's default locale.
pub fn read_manifest(manifest: &Value, messages: &dyn Fn(&str) -> Option<String>) -> ManifestInfo {
    let text = |key: &str| -> String {
        let raw = manifest.get(key).and_then(Value::as_str).unwrap_or_default();
        match raw.strip_prefix("__MSG_").and_then(|r| r.strip_suffix("__")) {
            Some(name) => messages(name).unwrap_or_else(|| raw.to_string()),
            None => raw.to_string(),
        }
    };
    let largest = |v: Option<&Value>| -> Option<String> {
        match v? {
            Value::String(s) => Some(s.clone()),
            Value::Object(sizes) => sizes
                .iter()
                .filter_map(|(size, path)| Some((size.parse::<u32>().unwrap_or(0), path.as_str()?)))
                .max_by_key(|(size, _)| *size)
                .map(|(_, p)| p.to_string()),
            _ => None,
        }
    };
    let action = manifest.get("action").or_else(|| manifest.get("browser_action")).or_else(|| manifest.get("page_action"));
    let page = |v: Option<&Value>| v.and_then(Value::as_str).filter(|s| !s.is_empty()).map(|s| s.trim_start_matches('/').to_string());
    ManifestInfo {
        name: text("name"),
        version: manifest.get("version").and_then(Value::as_str).unwrap_or("0").to_string(),
        description: text("description"),
        icon: largest(manifest.get("icons")).or_else(|| largest(action.and_then(|a| a.get("default_icon")))),
        popup: page(action.and_then(|a| a.get("default_popup"))),
        options: page(manifest.get("options_ui").and_then(|o| o.get("page"))).or_else(|| page(manifest.get("options_page"))),
        key: manifest.get("key").and_then(Value::as_str).map(String::from),
    }
}

/// Reads an extension folder's manifest, with its localized name and description.
pub fn read_folder(dir: &Path) -> Result<(Value, ManifestInfo), ExtensionError> {
    let raw = fs::read_to_string(dir.join("manifest.json")).map_err(|_| ExtensionError::NoManifest)?;
    // Some manifests start with a byte-order mark, which serde_json rejects.
    let manifest: Value =
        serde_json::from_str(raw.trim_start_matches('\u{feff}')).map_err(|e| ExtensionError::BadManifest(e.to_string()))?;
    let locale = manifest.get("default_locale").and_then(Value::as_str).unwrap_or("en").to_string();
    let messages: BTreeMap<String, String> = fs::read_to_string(dir.join("_locales").join(&locale).join("messages.json"))
        .ok()
        .and_then(|j| serde_json::from_str::<Value>(j.trim_start_matches('\u{feff}')).ok())
        .and_then(|v| v.as_object().cloned())
        .map(|o| {
            o.into_iter()
                .filter_map(|(k, v)| Some((k.to_lowercase(), v.get("message")?.as_str()?.to_string())))
                .collect()
        })
        .unwrap_or_default();
    let info = read_manifest(&manifest, &|name| messages.get(&name.to_lowercase()).cloned());
    Ok((manifest, info))
}

/// Extracts a zip into `dir`, skipping paths that would land outside it.
fn extract_zip(zip: &[u8], dir: &Path) -> Result<(), ExtensionError> {
    let mut archive = zip::ZipArchive::new(Cursor::new(zip)).map_err(|e| ExtensionError::Damaged(e.to_string()))?;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|e| ExtensionError::Damaged(e.to_string()))?;
        let Some(rel) = file.enclosed_name() else { continue };
        let out = dir.join(rel);
        if file.is_dir() {
            fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut bytes = Vec::with_capacity(file.size() as usize);
        file.read_to_end(&mut bytes).map_err(|e| ExtensionError::Damaged(e.to_string()))?;
        fs::write(out, bytes)?;
    }
    Ok(())
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        let kind = entry.file_type()?;
        if kind.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else if kind.is_file() {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

/// A package's manifest may sit in a subfolder (zips of a project folder often do).
fn manifest_dir(dir: &Path) -> PathBuf {
    if dir.join("manifest.json").is_file() {
        return dir.to_path_buf();
    }
    let subdirs: Vec<PathBuf> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    match subdirs.as_slice() {
        [only] if only.join("manifest.json").is_file() => only.clone(),
        _ => dir.to_path_buf(),
    }
}

/// What to install from.
pub enum InstallFrom<'a> {
    /// The bytes of a `.crx` or `.zip`.
    Package(&'a [u8]),
    /// An unpacked extension's folder (with `manifest.json`).
    Folder(&'a Path),
}

/// Installs an extension into a new folder `root/<id>/<version>_<n>/`. Returns it (enabled) for
/// `State::add_extension`.
pub fn install(from: InstallFrom, root: &Path, source: Source) -> Result<Extension, ExtensionError> {
    fs::create_dir_all(root)?;
    let staging = root.join(format!(".install-{}", uuid::Uuid::new_v4()));
    let result = install_into(from, root, &staging, source);
    let _ = fs::remove_dir_all(&staging);
    result
}

fn install_into(from: InstallFrom, root: &Path, staging: &Path, source: Source) -> Result<Extension, ExtensionError> {
    let package_key = match from {
        InstallFrom::Package(bytes) => {
            let package = parse_package(bytes)?;
            extract_zip(package.zip, staging)?;
            package.public_key
        }
        InstallFrom::Folder(dir) => {
            copy_dir(dir, staging)?;
            None
        }
    };
    let dir = manifest_dir(staging);
    // Chromium refuses to load unpacked extensions with names starting with `_` that it
    // reserves; the Web Store's signatures live in `_metadata`.
    let _ = fs::remove_dir_all(dir.join("_metadata"));
    let (mut manifest, info) = read_folder(&dir)?;

    // The key decides the id. Keep the manifest's own; else the package's (the Web Store id);
    // else make one up, so the id doesn't depend on the folder's path.
    let engine = base64::engine::general_purpose::STANDARD;
    let key = match (&info.key, package_key) {
        (Some(k), _) => engine
            .decode(k.lines().filter(|l| !l.starts_with("-----")).collect::<String>().trim())
            .map_err(|e| ExtensionError::BadManifest(format!("bad key: {e}")))?,
        (None, Some(k)) => k,
        (None, None) => (0..4).flat_map(|_| *uuid::Uuid::new_v4().as_bytes()).collect(),
    };
    if info.key.is_none() {
        manifest["key"] = Value::String(engine.encode(&key));
        let json = serde_json::to_string_pretty(&manifest).map_err(|e| ExtensionError::BadManifest(e.to_string()))?;
        fs::write(dir.join("manifest.json"), json)?;
    }
    let id = id_from_key(&key);
    let safe_version: String = info.version.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '.').collect();
    fs::create_dir_all(root.join(&id))?;
    let rel = (0..)
        .map(|n| format!("{id}/{safe_version}_{n}"))
        .find(|rel| !root.join(rel).exists())
        .expect("some folder name is free");
    fs::rename(&dir, root.join(&rel))?;
    Ok(Extension {
        id,
        name: if info.name.is_empty() { "Extension".into() } else { info.name },
        version: info.version,
        dir: rel,
        description: info.description,
        enabled: true,
        source,
        icon: info.icon,
        popup: info.popup,
        options: info.options,
        pinned: false,
    })
}

/// What Burrow's helper extension's content script logs, before `<secret>:<tab id>`.
pub const HELPER_PREFIX: &str = "__burrow_tab__";

/// The files of Burrow's own helper extension, loaded whenever other extensions are. In
/// Alloy-style tabs the id the extension APIs use for a tab is not CEF's browser id, and CEF
/// doesn't say what it is. The helper's content script asks its service worker for its tab's id
/// and logs it with `secret`, which pages can't read, so they can't fake it.
pub fn helper_files(secret: &str) -> Vec<(&'static str, String)> {
    let key = base64::engine::general_purpose::STANDARD.encode(Sha256::digest(b"burrow tab helper"));
    let manifest = serde_json::json!({
        "manifest_version": 3,
        "name": "Burrow",
        "version": "1",
        "description": "Tells extension popups which tab you are on.",
        "key": key,
        "background": { "service_worker": "worker.js" },
        "content_scripts": [{ "matches": ["<all_urls>"], "js": ["tab.js"], "run_at": "document_start" }]
    });
    let tab_js = format!(
        "chrome.runtime.sendMessage(0, (id) => {{ void chrome.runtime.lastError; \
         if (typeof id === 'number') console.log({}); }});",
        serde_json::to_string(&format!("{HELPER_PREFIX}{secret}:")).unwrap_or_default() + " + id"
    );
    vec![
        ("manifest.json", serde_json::to_string_pretty(&manifest).unwrap_or_default()),
        ("worker.js", "chrome.runtime.onMessage.addListener((m, sender, send) => send(sender.tab ? sender.tab.id : null));".into()),
        ("tab.js", tab_js),
    ]
}

// ---- The bridge (D-041) ---------------------------------------------------------------------------
//
// Burrow's tabs aren't Chrome tabs (D-032), so an extension's background can't open tabs or
// windows, `chrome.tabs.query` finds nothing, and its badge shows nowhere. Burrow adds a small
// script to each extension's background that sends those to Burrow over a local connection.

/// The bridge script, loaded first in the extension's background.
pub const BRIDGE_HOOK: &str = "burrow_hook.js";
/// The service worker Burrow puts in place: it loads the hook, then the extension's own.
pub const BRIDGE_WORKER: &str = "burrow_bg.js";
/// Remembers the extension's own service worker once the manifest names Burrow's.
pub const BRIDGE_ORIGINAL: &str = "burrow_bg_original.txt";

/// What adding the bridge to an extension writes, besides the hook and the manifest.
#[derive(Debug, PartialEq)]
pub struct BridgePlan {
    /// `burrow_bg.js`, for a service worker.
    pub worker: Option<String>,
    /// The extension's own service worker, to remember.
    pub original: Option<String>,
}

/// Points the manifest's background at the bridge. `original` is the remembered service worker
/// when the manifest already names Burrow's. None: nothing runs in the background to bridge
/// (or it's a background *page*, which isn't supported).
pub fn bridge_manifest(manifest: &mut Value, original: Option<&str>) -> Option<BridgePlan> {
    let bg = manifest.get_mut("background")?.as_object_mut()?;
    if let Some(sw) = bg.get("service_worker").and_then(Value::as_str) {
        let own = if sw == BRIDGE_WORKER { original?.to_string() } else { sw.to_string() };
        let own = own.trim_start_matches("./").trim_start_matches('/').to_string();
        let module = bg.get("type").and_then(Value::as_str) == Some("module");
        let script = serde_json::to_string(&own).ok()?;
        let worker = if module {
            format!("import \"./{BRIDGE_HOOK}\";\nimport {};\n", serde_json::to_string(&format!("./{own}")).ok()?)
        } else {
            format!("importScripts(\"{BRIDGE_HOOK}\", {script});\n")
        };
        bg.insert("service_worker".into(), Value::String(BRIDGE_WORKER.into()));
        return Some(BridgePlan { worker: Some(worker), original: Some(own) });
    }
    let scripts = bg.get_mut("scripts")?.as_array_mut()?;
    if !scripts.iter().any(|s| s.as_str() == Some(BRIDGE_HOOK)) {
        scripts.insert(0, Value::String(BRIDGE_HOOK.into()));
    }
    Some(BridgePlan { worker: None, original: None })
}

/// Adds the bridge to an installed extension's folder (again at every start: the hook carries
/// the connection details). Does nothing for extensions without a background script.
pub fn add_bridge(dir: &Path, hook: &str) -> Result<bool, ExtensionError> {
    let manifest_path = dir.join("manifest.json");
    let mut manifest: Value = serde_json::from_slice(&fs::read(&manifest_path)?).map_err(|e| ExtensionError::BadManifest(e.to_string()))?;
    let original = fs::read_to_string(dir.join(BRIDGE_ORIGINAL)).ok();
    let Some(plan) = bridge_manifest(&mut manifest, original.as_deref().map(str::trim)) else { return Ok(false) };
    fs::write(dir.join(BRIDGE_HOOK), hook)?;
    if let Some(worker) = plan.worker {
        fs::write(dir.join(BRIDGE_WORKER), worker)?;
    }
    if let Some(original) = plan.original {
        fs::write(dir.join(BRIDGE_ORIGINAL), original)?;
    }
    fs::write(&manifest_path, serde_json::to_string_pretty(&manifest).unwrap_or_default())?;
    Ok(true)
}

/// The bridge script: reports badges and sends tab and window requests to Burrow at
/// `127.0.0.1:port`, with `secret`, which only extensions' files hold.
pub fn bridge_hook(port: u16, secret: &str) -> String {
    format!(
        r#"// Added by Burrow (D-041). Burrow's tabs aren't Chrome's, so this sends what the extension
// asks of tabs and windows to Burrow, and shows its badge there.
(() => {{
  if (globalThis.__burrowBridge || !globalThis.chrome) return;
  globalThis.__burrowBridge = true;
  const URL = "http://127.0.0.1:{port}/ext", SECRET = {secret};
  // A "simple" request (text body, no custom headers): no CORS preflight.
  const call = (what, args, tries = 0) =>
    fetch(URL, {{ method: "POST", body: JSON.stringify({{ secret: SECRET, what, ...args }}) }})
      .then((r) => r.json())
      .then((r) => {{ if (!r.ok) throw new Error(r.error); return r.result; }})
      .catch((e) => {{
        // Burrow may still be starting.
        if (tries < 5) return new Promise((ok) => setTimeout(ok, 1000 * (tries + 1))).then(() => call(what, args, tries + 1));
        throw e;
      }});
  const answer = (p, cb) => {{ if (typeof cb === "function") {{ p.then(cb, () => cb()); return; }} return p; }};
  const color = (c) => Array.isArray(c) ? `rgba(${{c[0]}},${{c[1]}},${{c[2]}},${{(c[3] ?? 255) / 255}})` : c;

  const action = chrome.action || chrome.browserAction;
  if (action) {{
    for (const [name, key] of [["setBadgeText", "text"], ["setBadgeBackgroundColor", "color"]]) {{
      const own = action[name];
      if (typeof own !== "function") continue;
      action[name] = function (details, ...rest) {{
        const value = details && details[key];
        call("badge", {{ tab: details && details.tabId != null ? details.tabId : null, [key]: key === "color" ? color(value) : value }}).catch(() => {{}});
        return own.call(action, details, ...rest);
      }};
    }}
  }}

  const tabs = chrome.tabs, windows = chrome.windows;
  if (tabs) {{
    tabs.create = (props, cb) => {{
      props = props || {{}};
      return answer(call("open", {{ url: props.url || "", active: props.active !== false }}).then(() => ({{ id: -1, url: props.url || "", active: props.active !== false }})), cb);
    }};
    const query = tabs.query.bind(tabs);
    const filters = ["url", "title", "status", "audible", "muted", "pinned", "index", "groupId", "discarded", "windowType"];
    tabs.query = (q, cb) => answer(query(q || {{}}).then(async (found) => {{
      q = q || {{}};
      if (found.length) return found;
      const {{ active, all }} = await call("tabs", {{}});
      const wantsActive = q.active === true || q.highlighted === true || q.currentWindow === true || q.lastFocusedWindow === true;
      const ids = wantsActive && q.active !== false ? (active == null ? [] : [active]) : filters.some((k) => q[k] !== undefined) ? [] : all;
      const list = (await Promise.all(ids.map((id) => tabs.get(id).catch(() => null)))).filter(Boolean);
      return list.map((t) => Object.assign(t, {{ active: t.id === active, highlighted: t.id === active, selected: t.id === active }}));
    }}), cb);
  }}
  if (windows) {{
    // One window, always: a new window opens as a tab.
    windows.create = (data, cb) => {{
      data = data || {{}};
      const urls = Array.isArray(data.url) ? data.url : [data.url || ""];
      const opened = urls.reduce((p, url, i) => p.then(() => call("open", {{ url, active: i === 0 && data.focused !== false }})), Promise.resolve());
      return answer(opened.then(() => ({{ id: -1, focused: true, type: "normal", state: "normal", incognito: false, alwaysOnTop: false, tabs: [] }})), cb);
    }};
    const current = async () => {{
      const {{ active }} = await call("tabs", {{}});
      const t = active == null ? null : await tabs.get(active).catch(() => null);
      return {{ id: t ? t.windowId : -1, focused: true, type: "normal", state: "normal", incognito: false, alwaysOnTop: false }};
    }};
    for (const name of ["getCurrent", "getLastFocused"]) {{
      const own = windows[name].bind(windows);
      windows[name] = (info, cb) => {{
        if (typeof info === "function") [cb, info] = [info, {{}}];
        // Without a Chrome window it answers -1 ("no window"), or fails.
        return answer(own(info || {{}}).then((w) => (w && w.id >= 0 ? w : current())).catch(current), cb);
      }};
    }}
  }}
}})();
"#,
        secret = serde_json::to_string(secret).unwrap_or_default(),
    )
}

/// The tab id in a console message from the helper extension, if it carries `secret`.
pub fn helper_tab_id(message: &str, secret: &str) -> Option<i32> {
    message.strip_prefix(HELPER_PREFIX)?.strip_prefix(secret)?.strip_prefix(':')?.parse().ok()
}

/// Deletes folders under `root` that no extension in `keep` uses: old versions, removed
/// extensions and unfinished installs. Run before Chromium loads anything.
pub fn clean_up(root: &Path, keep: &[Extension]) {
    let Ok(entries) = fs::read_dir(root) else { return };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let path = e.path();
        if !keep.iter().any(|x| x.id == name) {
            let _ = if path.is_dir() { fs::remove_dir_all(&path) } else { fs::remove_file(&path) };
            continue;
        }
        for v in fs::read_dir(&path).into_iter().flatten().flatten() {
            let rel = format!("{name}/{}", v.file_name().to_string_lossy());
            if !keep.iter().any(|x| x.dir == rel) {
                let _ = fs::remove_dir_all(v.path());
            }
        }
    }
}

impl crate::State {
    pub fn extension(&self, id: &str) -> Option<&Extension> {
        self.extensions.iter().find(|e| e.id == id)
    }

    /// Adds a newly installed extension, or updates the one with its id (keeping whether it was
    /// on and where it sits in the list).
    pub fn add_extension(&mut self, ext: Extension) {
        match self.extensions.iter_mut().find(|e| e.id == ext.id) {
            Some(old) => *old = Extension { enabled: old.enabled, pinned: old.pinned, ..ext },
            None => self.extensions.push(ext),
        }
    }

    /// Pins an extension to the toolbar, or unpins it. False if there is no such extension.
    pub fn set_extension_pinned(&mut self, id: &str, pinned: bool) -> bool {
        self.extensions.iter_mut().find(|e| e.id == id).map(|e| e.pinned = pinned).is_some()
    }

    /// False if there is no such extension.
    pub fn set_extension_enabled(&mut self, id: &str, enabled: bool) -> bool {
        self.extensions.iter_mut().find(|e| e.id == id).map(|e| e.enabled = enabled).is_some()
    }

    /// Removes it from the list; the caller deletes its folder.
    pub fn remove_extension(&mut self, id: &str) -> Option<Extension> {
        let i = self.extensions.iter().position(|e| e.id == id)?;
        Some(self.extensions.remove(i))
    }

    /// The extensions Chromium should load, as (id, version).
    pub fn enabled_extensions(&self) -> Vec<(String, String)> {
        self.extensions.iter().filter(|e| e.enabled).map(|e| (e.id.clone(), e.version.clone())).collect()
    }

    /// Finds an extension by id or by name (ignoring case), for agents.
    pub fn find_extension(&self, query: &str) -> Option<&Extension> {
        let q = query.trim();
        self.extension(q).or_else(|| self.extensions.iter().find(|e| e.name.eq_ignore_ascii_case(q)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;

    fn zip_of(files: &[(&str, &str)]) -> Vec<u8> {
        let mut buf = Cursor::new(Vec::new());
        {
            let mut w = zip::ZipWriter::new(&mut buf);
            let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
            for (name, body) in files {
                w.start_file(*name, opts).unwrap();
                w.write_all(body.as_bytes()).unwrap();
            }
            w.finish().unwrap();
        }
        buf.into_inner()
    }

    fn varint(mut v: usize, out: &mut Vec<u8>) {
        loop {
            let b = (v & 0x7F) as u8;
            v >>= 7;
            if v == 0 {
                out.push(b);
                return;
            }
            out.push(b | 0x80);
        }
    }

    fn field(n: usize, data: &[u8]) -> Vec<u8> {
        let mut out = vec![];
        varint(n << 3 | 2, &mut out);
        varint(data.len(), &mut out);
        out.extend_from_slice(data);
        out
    }

    fn crx3(key: &[u8], other_key: &[u8], zip: &[u8]) -> Vec<u8> {
        let proof = |k: &[u8]| [field(1, k), field(2, b"sig")].concat();
        let signed = field(1, &Sha256::digest(key)[..16]);
        let header = [field(2, &proof(other_key)), field(2, &proof(key)), field(10000, &signed)].concat();
        let mut out = b"Cr24".to_vec();
        out.extend_from_slice(&3u32.to_le_bytes());
        out.extend_from_slice(&(header.len() as u32).to_le_bytes());
        out.extend_from_slice(&header);
        out.extend_from_slice(zip);
        out
    }

    fn temp_root(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("burrow-ext-test-{name}-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn ids_come_from_keys() {
        // Chromium gave this id to a test extension with this key (checked on Xvfb, D-032).
        let key = base64::engine::general_purpose::STANDARD
            .decode("MIGfMA0GCSqGSIb3DQEBAQUAA4GNADCBiQKBgQDcBHwzDvyBQ6bDppkIs9MP4ksKqCMyXQ/A52JivHZKh4YO/9vJsT3oaYhSpDCE9RPocOEQvwsHsFReW2nUEc6OLLyoCFFxIb7KkLGsmfakkut/fFdNJYh0xOTbSN8YvLWcqph09XAY2Y/f0AL7vfO1cuCqtkMt8hFrBGWxDdf9CQIDAQAB")
            .unwrap();
        assert_eq!(id_from_key(&key), "knldjmfmopnpolahpmmgbagdohdnhkik");
        assert_eq!(id_from_key(b"x").len(), 32);
        assert!(id_from_key(b"x").bytes().all(|b| (b'a'..=b'p').contains(&b)));
    }

    #[test]
    fn web_store_links() {
        let id = "nngceckbapebfimnlniiiahkandclblb";
        assert_eq!(web_store_id(id).as_deref(), Some(id));
        assert_eq!(web_store_id(&format!("https://chromewebstore.google.com/detail/bitwarden/{id}?hl=en")).as_deref(), Some(id));
        assert_eq!(web_store_id(&format!("https://chrome.google.com/webstore/detail/{id}")).as_deref(), Some(id));
        assert_eq!(web_store_id(&format!("https://example.com/{id}")), None);
        assert_eq!(web_store_id("nngceckbapebfimnlniiiahkandclblz"), None);
        assert!(web_store_download_url(id).contains(&format!("id%3D{id}%26uc")));
    }

    #[test]
    fn crx3_gives_zip_and_signed_key() {
        let zip = zip_of(&[("manifest.json", "{}")]);
        let crx = crx3(b"the-real-key", b"another-key", &zip);
        let p = parse_package(&crx).unwrap();
        assert_eq!(p.zip, &zip[..]);
        assert_eq!(p.public_key.as_deref(), Some(&b"the-real-key"[..]));
        assert!(parse_package(b"hello").is_err());
        assert!(parse_package(&crx[..20]).is_err());
        assert!(parse_package(&zip).unwrap().public_key.is_none());
    }

    #[test]
    fn manifest_fields_and_messages() {
        let m = json!({
            "name": "__MSG_appName__", "version": "2.1", "description": "plain",
            "icons": { "16": "i16.png", "128": "i128.png", "48": "i48.png" },
            "action": { "default_popup": "/popup/index.html", "default_icon": "a.png" },
            "options_ui": { "page": "options.html" }
        });
        let info = read_manifest(&m, &|n| (n == "appName").then(|| "Pass Keeper".to_string()));
        assert_eq!(info.name, "Pass Keeper");
        assert_eq!(info.icon.as_deref(), Some("i128.png"));
        assert_eq!(info.popup.as_deref(), Some("popup/index.html"));
        assert_eq!(info.options.as_deref(), Some("options.html"));
        let mv2 = json!({ "name": "Old", "version": "1", "browser_action": { "default_icon": { "19": "b19.png", "38": "b38.png" } }, "options_page": "o.html" });
        let info = read_manifest(&mv2, &|_| None);
        assert_eq!(info.icon.as_deref(), Some("b38.png"));
        assert_eq!(info.popup, None);
        assert_eq!(info.options.as_deref(), Some("o.html"));
    }

    #[test]
    fn installs_a_web_store_package_under_its_store_id() {
        let root = temp_root("crx");
        let zip = zip_of(&[
            ("manifest.json", r#"{"name":"__MSG_n__","version":"1.0","default_locale":"en","action":{"default_popup":"p.html"}}"#),
            ("_locales/en/messages.json", r#"{"N":{"message":"Store Ext"}}"#),
            ("_metadata/verified_contents.json", "[]"),
            ("p.html", "<p>hi</p>"),
            ("../escape.txt", "no"),
        ]);
        let crx = crx3(b"store-key", b"x", &zip);
        let ext = install(InstallFrom::Package(&crx), &root, Source::WebStore).unwrap();
        assert_eq!(ext.id, id_from_key(b"store-key"));
        assert_eq!(ext.name, "Store Ext");
        assert_eq!(ext.popup.as_deref(), Some("p.html"));
        let dir = root.join(&ext.dir);
        assert!(ext.dir.starts_with(&format!("{}/1.0_", ext.id)));
        assert!(dir.join("p.html").is_file());
        assert!(!dir.join("_metadata").exists());
        assert!(!root.join("escape.txt").exists());
        // The key is written into the manifest, so Chromium derives the same id.
        let (_, info) = read_folder(&dir).unwrap();
        let key = base64::engine::general_purpose::STANDARD.decode(info.key.unwrap()).unwrap();
        assert_eq!(id_from_key(&key), ext.id);
        // Installing again (an update) goes into a new folder; nothing is left in staging.
        let again = install(InstallFrom::Package(&crx), &root, Source::WebStore).unwrap();
        assert_eq!(again.id, ext.id);
        assert_ne!(again.dir, ext.dir);
        let entries: Vec<_> = fs::read_dir(&root).unwrap().flatten().collect();
        assert_eq!(entries.len(), 1);
        // Cleaning up keeps only the folder in use.
        fs::create_dir_all(root.join(".install-x")).unwrap();
        fs::create_dir_all(root.join("gone/1_0")).unwrap();
        clean_up(&root, std::slice::from_ref(&again));
        assert!(root.join(&again.dir).is_dir());
        assert!(!root.join(&ext.dir).exists());
        assert!(!root.join(".install-x").exists() && !root.join("gone").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_list_keeps_on_off_and_order_across_updates() {
        let ext = |id: &str, version: &str| Extension {
            id: id.into(),
            name: id.to_uppercase(),
            version: version.into(),
            dir: format!("{id}/{version}_0"),
            description: String::new(),
            enabled: true,
            source: Source::WebStore,
            icon: None,
            popup: None,
            options: None,
            pinned: false,
        };
        let mut s = crate::State::default();
        s.add_extension(ext("a", "1"));
        s.add_extension(ext("b", "1"));
        assert!(s.set_extension_enabled("a", false));
        assert!(!s.set_extension_enabled("zzz", false));
        s.add_extension(ext("a", "2"));
        assert_eq!(s.extensions[0].version, "2");
        assert!(!s.extensions[0].enabled);
        assert_eq!(s.enabled_extensions(), vec![("b".to_string(), "1".to_string())]);
        assert_eq!(s.find_extension("B").map(|e| e.id.as_str()), Some("b"));
        assert_eq!(s.remove_extension("b").map(|e| e.id), Some("b".into()));
        assert!(s.remove_extension("b").is_none());
        // Old state files have no extension list.
        let mut v = serde_json::to_value(&s).unwrap();
        v.as_object_mut().unwrap().remove("extensions");
        v["settings"].as_object_mut().unwrap().remove("sidebar_width");
        let old: crate::State = serde_json::from_value(v).unwrap();
        assert!(old.extensions.is_empty());
        assert_eq!(old.settings.sidebar_width, crate::model::SIDEBAR_WIDTH_DEFAULT);
    }

    #[test]
    fn helper_reports_tab_ids_with_its_secret() {
        let files = helper_files("s3cret");
        let manifest: Value = serde_json::from_str(&files[0].1).unwrap();
        assert_eq!(manifest["content_scripts"][0]["js"][0], "tab.js");
        assert!(files[2].1.contains("\"__burrow_tab__s3cret:\" + id"));
        assert_eq!(helper_tab_id("__burrow_tab__s3cret:1238539899", "s3cret"), Some(1238539899));
        assert_eq!(helper_tab_id("__burrow_tab__guess:12", "s3cret"), None);
        assert_eq!(helper_tab_id("__burrow_tab__s3cret:x", "s3cret"), None);
        // Its id never changes.
        let key = base64::engine::general_purpose::STANDARD.decode(manifest["key"].as_str().unwrap()).unwrap();
        assert_eq!(id_from_key(&key), id_from_key(&Sha256::digest(b"burrow tab helper")));
    }

    #[test]
    fn sidebar_width_is_clamped() {
        use crate::model::*;
        assert_eq!(clamp_sidebar_width(10), SIDEBAR_WIDTH_MIN);
        assert_eq!(clamp_sidebar_width(-5), SIDEBAR_WIDTH_MIN);
        assert_eq!(clamp_sidebar_width(320), 320);
        assert_eq!(clamp_sidebar_width(5000), SIDEBAR_WIDTH_MAX);
    }

    #[test]
    fn installs_a_folder_with_a_stable_made_up_key() {
        let root = temp_root("dir");
        let src = temp_root("src");
        fs::write(src.join("manifest.json"), r#"{"name":"Mine","version":"0.1","icons":{"48":"i.png"}}"#).unwrap();
        fs::create_dir_all(src.join("js")).unwrap();
        fs::write(src.join("js/a.js"), "1").unwrap();
        let ext = install(InstallFrom::Folder(&src), &root, Source::File).unwrap();
        assert_eq!(ext.name, "Mine");
        assert_eq!(ext.source, Source::File);
        assert!(root.join(&ext.dir).join("js/a.js").is_file());
        let (_, info) = read_folder(&root.join(&ext.dir)).unwrap();
        let key = base64::engine::general_purpose::STANDARD.decode(info.key.unwrap()).unwrap();
        assert_eq!(id_from_key(&key), ext.id);
        // The source folder is left as it was.
        assert!(!fs::read_to_string(src.join("manifest.json")).unwrap().contains("key"));
        // A zip whose manifest sits in a subfolder installs too.
        let zip = zip_of(&[("proj/manifest.json", r#"{"name":"Zipped","version":"3"}"#)]);
        assert_eq!(install(InstallFrom::Package(&zip), &root, Source::File).unwrap().name, "Zipped");
        assert!(matches!(
            install(InstallFrom::Package(&zip_of(&[("x.txt", "")])), &root, Source::File),
            Err(ExtensionError::NoManifest)
        ));
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(src).unwrap();
    }

    #[test]
    fn bridge_wraps_the_service_worker_once() {
        let mut m = json!({ "manifest_version": 3, "background": { "service_worker": "./bg/sw.js" } });
        let plan = bridge_manifest(&mut m, None).unwrap();
        assert_eq!(m["background"]["service_worker"], BRIDGE_WORKER);
        assert_eq!(plan.original.as_deref(), Some("bg/sw.js"));
        assert_eq!(plan.worker.as_deref(), Some("importScripts(\"burrow_hook.js\", \"bg/sw.js\");\n"));
        // At the next start the manifest names Burrow's worker; the remembered one is used.
        let again = bridge_manifest(&mut m, Some("bg/sw.js")).unwrap();
        assert_eq!(again, plan);
        // Without the remembered name there is nothing safe to do.
        assert_eq!(bridge_manifest(&mut m, None), None);
    }

    #[test]
    fn bridge_handles_modules_mv2_and_no_background() {
        let mut m = json!({ "background": { "service_worker": "sw.js", "type": "module" } });
        let plan = bridge_manifest(&mut m, None).unwrap();
        assert_eq!(plan.worker.as_deref(), Some("import \"./burrow_hook.js\";\nimport \"./sw.js\";\n"));
        let mut mv2 = json!({ "background": { "scripts": ["a.js", "b.js"] } });
        bridge_manifest(&mut mv2, None).unwrap();
        bridge_manifest(&mut mv2, None).unwrap();
        assert_eq!(mv2["background"]["scripts"], json!(["burrow_hook.js", "a.js", "b.js"]));
        assert_eq!(bridge_manifest(&mut json!({ "name": "x" }), None), None);
        assert_eq!(bridge_manifest(&mut json!({ "background": { "page": "bg.html" } }), None), None);
    }

    #[test]
    fn bridge_hook_names_port_and_secret() {
        let js = bridge_hook(4321, "s3cret");
        assert!(js.contains("http://127.0.0.1:4321/ext"));
        assert!(js.contains("SECRET = \"s3cret\""));
    }

    #[test]
    fn add_bridge_rewrites_the_folder() {
        let dir = std::env::temp_dir().join(format!("burrow-bridge-{}", crate::model::new_id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("manifest.json"), r#"{"manifest_version":3,"name":"t","version":"1","background":{"service_worker":"sw.js"}}"#).unwrap();
        assert!(add_bridge(&dir, "// hook").unwrap());
        assert!(add_bridge(&dir, "// hook 2").unwrap());
        let m: Value = serde_json::from_slice(&fs::read(dir.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(m["background"]["service_worker"], BRIDGE_WORKER);
        assert_eq!(fs::read_to_string(dir.join(BRIDGE_HOOK)).unwrap(), "// hook 2");
        assert!(fs::read_to_string(dir.join(BRIDGE_WORKER)).unwrap().contains("\"sw.js\""));
        let _ = fs::remove_dir_all(dir);
    }
}
