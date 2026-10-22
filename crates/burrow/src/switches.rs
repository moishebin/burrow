//! Chromium switches that keep Burrow quiet and Google-free (D-006).

use cef::*;

const SWITCHES: &[&str] = &[
    // No background calls home: component updater, variations, metrics.
    "disable-component-update",
    "disable-background-networking",
    "disable-domain-reliability",
    "disable-breakpad",
    "no-default-browser-check",
    "no-first-run",
    "metrics-recording-only",
    // Burrow restores its own tabs; Chrome must never offer or perform a session restore.
    "disable-session-crashed-bubble",
    "hide-crash-restore-bubble",
    // Never pass --disable-features here: it replaces the list CEF sets itself, and Chrome-style
    // browsers then crash on creation (D-010). Turn features off with preferences instead.
];

pub fn apply(command_line: &mut CommandLine) {
    for s in SWITCHES {
        match s.split_once('=') {
            Some((name, value)) => {
                command_line.append_switch_with_value(Some(&CefString::from(name)), Some(&CefString::from(value)))
            }
            None => command_line.append_switch(Some(&CefString::from(*s))),
        }
    }
}
