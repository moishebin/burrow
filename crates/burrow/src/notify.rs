//! System notifications for web pages' notifications (D-042): Windows toasts, `notify-send` on
//! Linux. On Windows Burrow registers its own app id for toasts (`HKCU\Software\Classes\
//! AppUserModelId\Burrow.Browser`, a name and an icon), so toasts work without a Store package
//! or a special shortcut. Clicking a toast calls back while Burrow runs.

/// Shows a notification: `title`, `body`, and where it came from. `on_click` runs (on some
/// thread) when the user clicks it.
pub fn show(title: &str, body: &str, source: &str, on_click: Box<dyn Fn() + Send + Sync>) {
    #[cfg(target_os = "windows")]
    win::show(title, body, source, on_click);
    #[cfg(not(target_os = "windows"))]
    {
        let _ = on_click;
        // Best effort: most desktops have notify-send (libnotify); clicks aren't reported.
        let body = if source.is_empty() { body.to_string() } else { format!("{body}\n{source}") };
        let _ = std::process::Command::new("notify-send")
            .args(["--app-name", crate::consts::APP_NAME, title, &body])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }
}

/// Registers Burrow's toast identity (Windows). Cheap; done at every start, on its own thread.
pub fn register() {
    #[cfg(target_os = "windows")]
    std::thread::spawn(win::register);
}

#[cfg(target_os = "windows")]
mod win {
    use std::sync::Mutex;

    use windows::{
        core::{IInspectable, HSTRING},
        Data::Xml::Dom::XmlDocument,
        Foundation::TypedEventHandler,
        UI::Notifications::{ToastNotification, ToastNotificationManager},
        Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED},
    };

    const AUMID: &str = "Burrow.Browser";

    /// Toasts stay alive while they may still be clicked (their click handler lives with them).
    static SHOWN: Mutex<Vec<ToastNotification>> = Mutex::new(Vec::new());

    fn escape(s: &str) -> String {
        s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
    }

    pub fn register() {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let icon = crate::paths::data_dir().join("burrow-icon.png");
        let _ = std::fs::write(&icon, include_bytes!("../resources/icon-256.png"));
        let key = format!(r"HKCU\Software\Classes\AppUserModelId\{AUMID}");
        for (name, data) in [("DisplayName", crate::consts::APP_NAME.to_string()), ("IconUri", icon.to_string_lossy().into_owned())] {
            let _ = std::process::Command::new("reg")
                .args(["add", &key, "/v", name, "/t", "REG_SZ", "/d", &data, "/f"])
                .creation_flags(CREATE_NO_WINDOW)
                .status();
        }
    }

    pub fn show(title: &str, body: &str, source: &str, on_click: Box<dyn Fn() + Send + Sync>) {
        let xml = format!(
            r#"<toast><visual><binding template="ToastGeneric"><text>{}</text><text>{}</text><text placement="attribution">{}</text></binding></visual></toast>"#,
            escape(title),
            escape(body),
            escape(source)
        );
        // WinRT wants an apartment; a short-lived thread of its own keeps Chromium's UI thread out of it.
        std::thread::spawn(move || {
            let shown = (|| -> windows::core::Result<ToastNotification> {
                unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok()? };
                let doc = XmlDocument::new()?;
                doc.LoadXml(&HSTRING::from(xml))?;
                let toast = ToastNotification::CreateToastNotification(&doc)?;
                toast.Activated(&TypedEventHandler::<ToastNotification, IInspectable>::new(move |_, _| {
                    on_click();
                    Ok(())
                }))?;
                ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(AUMID))?.Show(&toast)?;
                Ok(toast)
            })();
            match shown {
                Ok(toast) => {
                    if let Ok(mut v) = SHOWN.lock() {
                        v.push(toast);
                        let excess = v.len().saturating_sub(30);
                        v.drain(..excess);
                    }
                }
                Err(e) => eprintln!("Burrow: notification not shown: {e}"),
            }
        });
    }
}
