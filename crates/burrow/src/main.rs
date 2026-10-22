#![cfg_attr(all(not(debug_assertions), target_os = "windows"), windows_subsystem = "windows")]

mod agent;
mod app;
mod browser;
mod clients;
mod clipboard;
mod consts;
mod ext_bridge;
mod external;
mod notify;
mod paths;
mod scheme;
mod switches;
mod task;
mod views;
mod writer;

use cef::*;

fn main() {
    // `burrow --mcp` / `burrow --agent …` are clients of a running Burrow, not the browser.
    if let Some(code) = agent::main_if_requested() {
        std::process::exit(code);
    }
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
    agent::stop_server();
    shutdown();
    // "Restart Burrow" (to load changed extensions, D-032): start again now that this process
    // has let go of the profile. Links it was started with aren't opened twice.
    // A downloaded update installs now (D-037); its installer starts Burrow again if asked to.
    let updating = browser::run_pending_install(browser::restart_requested());
    if !updating && browser::restart_requested() {
        let flags = std::env::args().skip(1).filter(|a| a.starts_with("--") && !a.starts_with("--load-extension"));
        if let Err(e) = std::process::Command::new(std::env::current_exe().unwrap_or_default()).args(flags).spawn() {
            eprintln!("Burrow: could not restart: {e}");
        }
    }
}
