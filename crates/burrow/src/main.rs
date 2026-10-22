#![cfg_attr(all(not(debug_assertions), target_os = "windows"), windows_subsystem = "windows")]

mod app;
mod browser;
mod clients;
mod clipboard;
mod consts;
mod paths;
mod switches;
mod task;
mod views;

use cef::*;

fn main() {
    let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);
    let args = args::Args::new();
    let mut app = app::BurrowApp::new();

    // Renderer, GPU and utility processes run this same exe and return here.
    let code = execute_process(Some(args.as_main_args()), Some(&mut app), std::ptr::null_mut());
    if code >= 0 {
        std::process::exit(code);
    }

    paths::clear_chrome_sessions();
    let data = paths::data_dir();
    let settings = Settings {
        no_sandbox: 1,
        // Every profile's cache must live under the root cache path (D-004).
        root_cache_path: CefString::from(data.join("profiles").to_string_lossy().as_ref()),
        cache_path: CefString::from(data.join("profiles").join("_ui").to_string_lossy().as_ref()),
        persist_session_cookies: 1,
        log_file: CefString::from(data.join("cef.log").to_string_lossy().as_ref()),
        log_severity: LogSeverity::WARNING,
        background_color: consts::BACKGROUND,
        ..Default::default()
    };
    if initialize(Some(args.as_main_args()), Some(&settings), Some(&mut app), std::ptr::null_mut()) != 1 {
        // Most often: another Burrow already holds the profile lock.
        eprintln!("Burrow: CEF failed to initialize (is Burrow already running?)");
        std::process::exit(1);
    }
    run_message_loop();
    browser::with(|b| b.save_now());
    shutdown();
}
