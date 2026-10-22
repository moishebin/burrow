//! Links from other programs (D-024): Burrow as the default browser. Windows starts
//! `burrow.exe <url>` for every link. If Burrow is already running, the new process hands the
//! link to it over the local API (D-018) and exits, so links never open a second window.

use std::sync::Mutex;

static STARTUP_URLS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// The links on the command line: URLs, or paths of files that exist (opened as `file://`).
pub fn urls_from_args(args: &[String]) -> Vec<String> {
    // Chromium's helper processes run this exe too; they are never handed links.
    if args.iter().any(|a| a.starts_with("--type=")) {
        return vec![];
    }
    args.iter()
        .filter(|a| !a.starts_with('-'))
        .filter_map(|a| {
            let lower = a.to_ascii_lowercase();
            if ["http://", "https://", "file://"].iter().any(|p| lower.starts_with(p)) {
                return Some(a.clone());
            }
            let path = std::path::Path::new(a);
            path.is_file().then(|| file_url(&std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())))
        })
        .collect()
}

fn file_url(path: &std::path::Path) -> String {
    let p = path.to_string_lossy().replace('\\', "/");
    let p = p.strip_prefix("//?/").unwrap_or(&p).to_string();
    let p = p.replace(' ', "%20").replace('#', "%23");
    if p.starts_with('/') { format!("file://{p}") } else { format!("file:///{p}") }
}

/// Hands the command line's links to a running Burrow. Returns true if one took them, so this
/// process can exit; otherwise keeps them for this Burrow to open once its window is up.
pub fn forward_or_keep(urls: Vec<String>) -> bool {
    if urls.is_empty() {
        return false;
    }
    if crate::agent::send_external(&urls) {
        return true;
    }
    if let Ok(mut v) = STARTUP_URLS.lock() {
        *v = urls;
    }
    false
}

pub fn take_startup_urls() -> Vec<String> {
    STARTUP_URLS.lock().map(|mut v| std::mem::take(&mut *v)).unwrap_or_default()
}

/// The registry values that make Windows list Burrow as a browser, for the current user only
/// (no admin rights needed): (key, value name or "" for the default value, data).
#[cfg_attr(not(windows), allow(dead_code))]
pub fn registry_entries(exe: &str) -> Vec<(String, String, String)> {
    let app = r"Software\Clients\StartMenuInternet\Burrow";
    let open = format!("\"{exe}\" \"%1\"");
    let icon = format!("\"{exe}\",0");
    let mut v: Vec<(String, &str, String)> = vec![
        (app.into(), "", "Burrow".into()),
        (format!(r"{app}\DefaultIcon"), "", icon.clone()),
        (format!(r"{app}\shell\open\command"), "", format!("\"{exe}\"")),
        (format!(r"{app}\Capabilities"), "ApplicationName", "Burrow".into()),
        (format!(r"{app}\Capabilities"), "ApplicationDescription", "An Arc-style browser with spaces, pinned tabs and folders.".into()),
        (format!(r"{app}\Capabilities"), "ApplicationIcon", icon.clone()),
        (format!(r"{app}\Capabilities\URLAssociations"), "http", "BurrowURL".into()),
        (format!(r"{app}\Capabilities\URLAssociations"), "https", "BurrowURL".into()),
        (format!(r"{app}\Capabilities\FileAssociations"), ".htm", "BurrowHTML".into()),
        (format!(r"{app}\Capabilities\FileAssociations"), ".html", "BurrowHTML".into()),
        (format!(r"{app}\Capabilities\FileAssociations"), ".pdf", "BurrowHTML".into()),
        (r"Software\RegisteredApplications".into(), "Burrow", format!(r"{app}\Capabilities")),
    ];
    for (class, name) in [("BurrowURL", "Burrow URL"), ("BurrowHTML", "Burrow Document")] {
        let key = format!(r"Software\Classes\{class}");
        v.push((key.clone(), "", name.into()));
        if class == "BurrowURL" {
            v.push((key.clone(), "URL Protocol", String::new()));
        }
        v.push((format!(r"{key}\DefaultIcon"), "", icon.clone()));
        v.push((format!(r"{key}\shell\open\command"), "", open.clone()));
    }
    v.into_iter().map(|(k, n, d)| (format!(r"HKCU\{k}"), n.to_string(), d)).collect()
}

/// Registers Burrow with Windows and opens Settings, where the user picks it (Windows doesn't let
/// programs make themselves the default). Returns a message for the sidebar.
pub fn register_default_browser() -> String {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let exe = match std::env::current_exe() {
            Ok(p) => p.to_string_lossy().into_owned(),
            Err(e) => return format!("Couldn't find Burrow's program file: {e}"),
        };
        for (key, name, data) in registry_entries(&exe) {
            let mut cmd = std::process::Command::new("reg");
            cmd.arg("add").arg(&key);
            if name.is_empty() { cmd.arg("/ve") } else { cmd.arg("/v").arg(&name) };
            cmd.args(["/t", "REG_SZ", "/d"]).arg(&data).arg("/f").creation_flags(CREATE_NO_WINDOW);
            match cmd.status() {
                Ok(s) if s.success() => {}
                _ => return format!("Couldn't write {key} to the registry."),
            }
        }
        let _ = std::process::Command::new("cmd")
            .args(["/c", "start", "", "ms-settings:defaultapps?registeredAppUser=Burrow"])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn();
        "Burrow is registered. In Settings, pick Burrow as your web browser.".into()
    }
    #[cfg(not(target_os = "windows"))]
    {
        "Run `xdg-settings set default-web-browser` with a burrow.desktop file to make Burrow the default on Linux.".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_come_from_plain_arguments_only() {
        let args = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(urls_from_args(&args(&["https://a.example/x", "--remote-debugging-port=9222"])), vec!["https://a.example/x"]);
        assert!(urls_from_args(&args(&["--type=renderer", "https://a.example"])).is_empty());
        assert!(urls_from_args(&args(&["not a url", "/no/such/file.html"])).is_empty());
        let file = std::env::temp_dir().join("burrow external test.html");
        std::fs::write(&file, "<p>hi</p>").unwrap();
        let got = urls_from_args(&args(&[&file.to_string_lossy()]));
        assert!(got[0].starts_with("file:///") && got[0].ends_with("burrow%20external%20test.html"), "{got:?}");
        let _ = std::fs::remove_file(file);
    }

    #[test]
    fn registry_opens_links_with_this_exe() {
        let entries = registry_entries(r"C:\Apps\Burrow\burrow.exe");
        let get = |key: &str, name: &str| {
            entries.iter().find(|(k, n, _)| k == key && n == name).map(|(_, _, d)| d.as_str())
        };
        assert_eq!(get(r"HKCU\Software\Classes\BurrowURL\shell\open\command", ""), Some(r#""C:\Apps\Burrow\burrow.exe" "%1""#));
        assert_eq!(get(r"HKCU\Software\Clients\StartMenuInternet\Burrow\Capabilities\URLAssociations", "https"), Some("BurrowURL"));
        assert_eq!(get(r"HKCU\Software\RegisteredApplications", "Burrow"), Some(r"Software\Clients\StartMenuInternet\Burrow\Capabilities"));
        assert_eq!(get(r"HKCU\Software\Classes\BurrowURL", "URL Protocol"), Some(""));
    }
}
