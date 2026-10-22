# Decision Log

Every non-obvious choice goes here, newest at the bottom. Format: context → decision → why → consequences.
If a decision is reversed, don't delete it; add a new entry that supersedes it.

---

## D-001 · Engine: Rust + CEF, not a Chromium fork (2026-10-22)

**Context.** We need a Chromium engine without Google services. The options were:
1. Fork ungoogled-chromium and modify the C++ Views UI.
2. Embed Chromium through CEF (Chromium Embedded Framework) and write our own UI.
3. Electron.

**Decision.** Rust app that embeds CEF through the [`cef`](https://crates.io/crates/cef) crate (tauri-apps/cef-rs).

**Why.**
- CEF ships prebuilt Chromium binaries (Spotify CDN), so there is no 100 GB, multi-hour Chromium build. A Rust rebuild takes seconds to minutes.
- CEF has no Google account sign-in, no Sync and no Google API keys baked in, so it's "Google-free" by default (see D-006 for the remaining pings).
- Our differentiators (sidebar, spaces, folders, favorites) are all *chrome UI*, which we own completely with CEF Views. A Chromium fork would put every UI change in C++ with hour-long builds.
- Rust was the user's preference "if it's a good choice". It is: cef-rs is actively maintained (Tauri is building a CEF runtime on it) and tracks CEF releases closely (v154 as of 2026-10).

**Consequences.**
- Chrome extension support is limited compared with real Chrome (user said extensions can wait).
- We depend on cef-rs keeping up with CEF versions.
- If we ever need deep Chromium patches, we can build our own CEF from an ungoogled-chromium tree later. The Rust app would not change.

## D-002 · Platform: Windows first, built from WSL (2026-10-22)

**Context.** The user runs Arc on Windows. Development happens in WSL2 (Ubuntu 22.04).

**Decision.** Windows x64 is the first target. We cross-compile from WSL with `cargo xwin build --target x86_64-pc-windows-msvc` (cef-rs explicitly supports cargo-xwin), using a user-local LLVM in `~/.local/llvm`. Linux builds also work for fast iteration and tests.

**Why.** No admin rights on Windows are needed (a VS Build Tools install needs UAC). One terminal, one repo.

## D-003 · UI architecture: CEF Views window + HTML sidebar (2026-10-22)

**Decision.**
- One top-level `cef::Window` (CEF Views) per app window. The goal is one window, ever.
- Layout: a horizontal `BoxLayout` holds a **sidebar `BrowserView`** (our own HTML/CSS/JS from `ui/`) and a **content `Panel`** that hosts the active tab's `BrowserView`.
- Every tab is its own `BrowserView`. Switching tabs swaps which view is the content panel's child; the others stay alive in memory.
- The sidebar UI talks to Rust over a custom scheme: `burrow://api/<command>` (handled by a Rust `SchemeHandlerFactory`). Rust pushes state back by running JS in the sidebar frame (`window.burrow.render(state)`).

**Why.** HTML/CSS lets us match Arc's visual polish quickly. Native Views widgets are limited and would need us to draw everything by hand. Keeping the browser *content* in native BrowserViews (not iframes) gives real per-tab processes and separate profiles.

## D-004 · Spaces = separate profiles (separate logins) (2026-10-22)

**Decision.** Each Space owns a `profile_id`. Each profile maps to a CEF `RequestContext` with its own `cache_path` (`<data>/profiles/<profile_id>/`), so cookies, local storage and logins are isolated per profile. Several spaces *may* share a profile (as in Arc), but new spaces get a new profile by default.

## D-005 · Data model and persistence (2026-10-22)

**Decision.** Burrow's model lives in `crates/burrow-core`. It is pure Rust with no CEF dependency and is unit-tested on Linux.
- `Space { id, name, icon, color, profile_id, favorites: Vec<Item>, pinned: Vec<Node>, today: Vec<Tab> }`
- `Node = Folder { id, name, children: Vec<Node>, collapsed } | Tab(Tab)`
- **Pinned tabs have a home URL that never changes.** Navigating inside a pinned tab doesn't move it. "Reset to pinned URL" brings it back. This is the user's core need: *"every tab has its place that never changes."*
- Favorites (Arc's "top apps") are **per space**, which the user explicitly asked for. In Arc they are per profile.
- Stored as one JSON file, `<data>/state.json`, written atomically (write temp, then rename) and debounced. It's human-readable and easy to back up; we'll move to SQLite if it ever gets big.

## D-006 · "Google-free" scope (2026-10-22)

CEF removes Google sign-in and sync. Remaining Chromium background traffic (component updater, safe-browsing, variations) is turned off with command-line switches at startup (`--disable-component-update`, `--disable-background-networking`, etc.). The default search engine is user-selectable and defaults to DuckDuckGo. This list is maintained in `crates/burrow/src/switches.rs`.

## D-007 · License: MPL-2.0 (2026-10-22)

The license is file-level copyleft, the same model Firefox uses. Improvements to our files must stay open, but other projects can still embed parts of the code. It is compatible with CEF (BSD) and cef-rs (MIT/Apache).

## D-008 · Working name "Burrow" (2026-10-22)

"Arc" is a trademark of The Browser Company, so we can't ship under it. "Burrow" is the working name: *every tab has its place*. It can be renamed later; the name lives in `Cargo.toml` and `crates/burrow/src/consts.rs`.

## D-009 · Sidebar ↔ Rust messaging over the console, not a custom scheme (2026-10-22)

**Supersedes** the `burrow://api` part of D-003.

**Decision.** The sidebar sends messages by `console.log("__burrow__" + json)`. Only the sidebar's own `DisplayHandler::on_console_message` looks for that prefix, and it runs on the UI thread. Rust answers by running `burrow.render(view)` in the sidebar frame. The sidebar is loaded as a plain `file://…/ui/sidebar.html` next to the exe.

**Why.** A custom-scheme resource handler runs on CEF's IO thread and would need cross-thread plumbing plus a guard so web pages can't call it. The console route needs neither: web content can't write to the sidebar's console.

**Rule.** CEF callbacks never touch the controller directly. They post a closure (`browser::later`) so the controller is never re-entered while borrowed.

## D-010 · Alloy runtime style everywhere; CEF gotchas (2026-10-22)

Found while bringing the app up on Windows:
1. **A CEF window can hold only one *Chrome-style* BrowserView** ("Cannot add multiple Chrome style BrowserViews"). Burrow shows a sidebar plus many tabs, so the window and all BrowserViews use **Alloy** style. Cost: no Chrome UI bubbles (permission prompts, password-manager UI, etc.), and no Chrome extensions system. Burrow must build its own UI for those (tracked in FEATURES.md).
2. **Never pass `--disable-features`.** It replaces the list CEF sets itself, and browser creation then crashes (access violation).
3. **Chrome restores its own session** from `<profile>/Sessions` after a crash and reopens old pages in hidden windows. Burrow deletes those folders at startup (`paths::clear_chrome_sessions`) and passes `--hide-crash-restore-bubble`.
4. **Request contexts initialize asynchronously.** Creating a browser before `on_request_context_initialized` fails, so tabs wait in `Profile::pending`.
5. **`do_close` decides whether closing a browser closes the window.** The default (0) for a Views-hosted browser closes the *parent window*, so closing one tab quit the whole app (found with real input, D-015). Burrow returns 1 normally, and the tab closes when its view is removed. Once the window is closing it returns 0, so CEF finishes the shutdown (`browser::is_closing`).
6. **`can_close` must be implemented.** cef-rs leaves unimplemented callbacks null, and CEF treats a null `can_close` as "no". Closing works in three steps: close all browsers, wait for the last `on_before_close`, then close the window again.

## D-011 · Default search engine: Google (2026-10-22)

The owner's network content filter blocks DuckDuckGo and other search portals, so the D-006 DuckDuckGo default would show a block page. The default is Google search, and the search engine can be changed in the sidebar's + menu. Burrow never tries to get around the filter: CEF uses the system proxy and network settings, just as Chrome does.

## D-012 · Dev loop: build in WSL, run on Windows, inspect over DevTools (2026-10-22)

- `scripts/build-win.sh` cross-compiles with cargo-xwin and copies the result to `%LOCALAPPDATA%\Burrow-dev\app`.
- `scripts/run-win.sh` restarts Burrow there with `--remote-debugging-port=9222` and a throwaway data dir.
- `scripts/cdp.ps1` (run on Windows) lists targets, takes screenshots, evaluates JS and injects keys.
- Screen capture can't see the window while Windows is locked, so DevTools screenshots are the way to check the UI. CDP-injected keys skip CEF's keyboard handler; to test shortcuts, post `WM_KEYDOWN` to the render widget (`postkey.ps1`).

## D-013 · Frameless, Arc-style window (2026-10-22)

**Decision.** By default the window has no Windows title bar. The sidebar's top row is the drag area (CSS `app-region: drag`, forwarded with `Window::set_draggable_regions`) and holds the minimize, maximize and close buttons at its right end. Double-clicking empty space in that row maximizes or restores the window. "Use Windows Title Bar" in the + menu brings back the native frame after a restart (`settings.system_title_bar`).

**Why.** Arc has no title bar; the sidebar is the chrome. The native frame is still available in case resizing or dragging the frameless window misbehaves on some setups: those behaviours couldn't be checked by hand while the machine was locked during development.

## D-014 · Permissions and page menu built by Burrow (2026-10-22)

Because of Alloy style (D-010), Burrow builds its own UI:
- **Permission prompts** (camera, microphone, screen, notifications, location, …) appear as cards at the top of the sidebar with Allow/Block and "Remember". Answers are stored per profile and origin in `state.site_permissions`.
- **The page's right-click menu** adds Open Link in New Tab / Background Tab, Copy Link Address, Search for "…" and Inspect to CEF's default menu.
- **Find in page** (Ctrl+F) reuses the sidebar's address box in "find" mode and shows a match counter.

## D-015 · Real-input testing on a virtual display (Xvfb) (2026-10-22)

**Decision.** Burrow also builds for Linux. `scripts/run-xvfb.sh` runs it on an Xvfb display (`:99`) inside WSL, driven by `xdotool`, which sends real mouse and keyboard input. `scripts/xshot.py` takes screenshots and `scripts/cdp.mjs` inspects pages. The owner asked for this so testing can continue while they're away.

**Why.** DevTools-injected input skips the code paths that matter: CEF's keyboard handler, HTML5 drag and drop, and window focus. The first real-input run found a release-blocking bug, closing a tab quit the app (D-010 #5), plus a focus bug: shortcuts died after closing the last tab.

**Limits.** Windows-only behaviour (the frameless title bar, Windows paths, the installer) still needs the Windows build and `cdp.ps1` (D-012). Xvfb has no window manager, so maximize, minimize and window dragging can't be tested there.

## D-016 · Shortcuts work everywhere; a hidden sidebar can always come back (2026-10-22)

**Problem (reported by the owner on first use).** Clicking "hide sidebar" with no page open left a window with no controls. Ctrl+S, the only way back, did nothing, because shortcuts were handled only by the focused page and no page had focus.

**Decision.**
- Every shortcut lives in one table (`clients::SHORTCUTS`) and is also registered as a **window accelerator**, so it works whatever has focus.
- Hiding the sidebar shrinks it to a 10px edge strip instead of removing it. **Hovering the strip peeks the sidebar out**, as in Arc, and moving away hides it again. The window buttons stay reachable while peeking.

## D-017 · A Google-free ("ungoogled") release is an engine swap, any time (2026-10-22)

**Context.** The owner asked whether a release without Google is possible later. Burrow currently uses the standard CEF binaries, which are regular Chromium without Google sign-in, sync or API keys (D-006). Regular Chromium still contains code that can contact Google in edge cases (component and dictionary downloads, etc.). ungoogled-chromium removes that at the source level.

**Decision.** Keep developing on standard CEF. When a Google-free release is wanted, build our own CEF package from Chromium with ungoogled-chromium's patches, at the **same Chromium version** cef-rs targets (currently 154), and build Burrow with `CEF_PATH` pointing at it. Burrow's code doesn't change. We can ship two flavors from one codebase ("Burrow" and "Burrow Ungoogled"), or switch over completely.

**Cost.** About 100 GB of disk and several hours of compiling per Chromium version, on a strong machine or a self-hosted/large CI runner (GitHub's free runners can't build Chromium). Before then, measure what standard Burrow actually sends to Google and switch off what we can with settings (option 1, still to do).
