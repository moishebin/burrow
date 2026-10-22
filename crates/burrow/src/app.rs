use cef::*;

use crate::{browser, scheme, switches};

wrap_app! {
    pub struct BurrowApp {}
    impl App {
        fn on_before_command_line_processing(
            &self,
            process_type: Option<&CefString>,
            command_line: Option<&mut CommandLine>,
        ) {
            // Only the browser process (empty type) takes our switches; children inherit them.
            let is_browser = process_type.is_none_or(|t| t.to_string().is_empty());
            if let (true, Some(cl)) = (is_browser, command_line) {
                switches::apply(cl);
                // Installed extensions load at startup (D-032).
                if let Some(dirs) = browser::load_extension_switch() {
                    cl.append_switch_with_value(Some(&CefString::from("load-extension")), Some(&CefString::from(dirs.as_str())));
                }
            }
        }

        fn on_register_custom_schemes(&self, registrar: Option<&mut SchemeRegistrar>) {
            if let Some(r) = registrar {
                scheme::register_custom_scheme(r);
            }
        }

        fn browser_process_handler(&self) -> Option<BrowserProcessHandler> {
            Some(BurrowProcessHandler::new())
        }
    }
}

wrap_browser_process_handler! {
    struct BurrowProcessHandler {}
    impl BrowserProcessHandler {
        fn on_context_initialized(&self) {
            scheme::register_handler();
            browser::start();
        }

        // Burrow started again (e.g. from a shortcut) while running. Returning 0 makes CEF open a
        // plain Chromium window; Burrow keeps one window, so bring it forward instead (D-029).
        fn on_already_running_app_relaunch(
            &self,
            _command_line: Option<&mut CommandLine>,
            _current_directory: Option<&CefString>,
        ) -> ::std::os::raw::c_int {
            browser::later(|b| b.bring_to_front());
            1
        }
    }
}
