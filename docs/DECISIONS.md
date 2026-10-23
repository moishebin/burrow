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
- Rust was preferred if it's a good choice. It is: cef-rs is actively maintained (Tauri is building a CEF runtime on it) and tracks CEF releases closely (v154 as of 2026-10).

**Consequences.**
- Chrome extension support is limited compared with real Chrome (extensions could wait at first; see D-032).
- We depend on cef-rs keeping up with CEF versions.
- If we ever need deep Chromium patches, we can build our own CEF from an ungoogled-chromium tree later. The Rust app would not change.

## D-002 · Platform: Windows first, built from WSL (2026-10-22)

**Context.** Burrow targets Windows first, where Arc users are. Development happens in WSL2 (Ubuntu 22.04).

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
- Favorites (Arc's "top apps") are **per space**, a deliberate requirement. In Arc they are per profile.
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

Some network content filters block DuckDuckGo and other search portals, so the D-006 DuckDuckGo default could show a block page. The default is Google search, and the search engine can be changed in the sidebar's + menu. Burrow never tries to get around the filter: CEF uses the system proxy and network settings, just as Chrome does.

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

**Decision.** Burrow also builds for Linux. `scripts/run-xvfb.sh` runs it on an Xvfb display (`:99`) inside WSL, driven by `xdotool`, which sends real mouse and keyboard input. `scripts/xshot.py` takes screenshots and `scripts/cdp.mjs` inspects pages. This lets testing continue with nobody at the screen.

**Why.** DevTools-injected input skips the code paths that matter: CEF's keyboard handler, HTML5 drag and drop, and window focus. The first real-input run found a release-blocking bug, closing a tab quit the app (D-010 #5), plus a focus bug: shortcuts died after closing the last tab.

**Limits.** Windows-only behaviour (the frameless title bar, Windows paths, the installer) still needs the Windows build and `cdp.ps1` (D-012). Xvfb has no window manager, so maximize, minimize and window dragging can't be tested there.

## D-016 · Shortcuts work everywhere; a hidden sidebar can always come back (2026-10-22)

**Problem (reported on first use).** Clicking "hide sidebar" with no page open left a window with no controls. Ctrl+S, the only way back, did nothing, because shortcuts were handled only by the focused page and no page had focus.

**Decision.**
- Every shortcut lives in one table (`clients::SHORTCUTS`) and is also registered as a **window accelerator**, so it works whatever has focus.
- Hiding the sidebar shrinks it to a 10px edge strip instead of removing it. **Hovering the strip peeks the sidebar out**, as in Arc, and moving away hides it again. The window buttons stay reachable while peeking.

## D-017 · A Google-free ("ungoogled") release is an engine swap, any time (2026-10-22)

**Context.** Is a release without Google possible later? Burrow currently uses the standard CEF binaries, which are regular Chromium without Google sign-in, sync or API keys (D-006). Regular Chromium still contains code that can contact Google in edge cases (component and dictionary downloads, etc.). ungoogled-chromium removes that at the source level.

**Decision.** Keep developing on standard CEF. When a Google-free release is wanted, build our own CEF package from Chromium with ungoogled-chromium's patches, at the **same Chromium version** cef-rs targets (currently 154), and build Burrow with `CEF_PATH` pointing at it. Burrow's code doesn't change. We can ship two flavors from one codebase ("Burrow" and "Burrow Ungoogled"), or switch over completely.

**Cost.** About 100 GB of disk and several hours of compiling per Chromium version, on a strong machine or a self-hosted/large CI runner (GitHub's free runners can't build Chromium). Before then, measure what standard Burrow actually sends to Google and switch off what we can with settings (option 1, still to do).

## D-018 · Agent interface: a local API, an MCP server and a CLI, all in `burrow.exe` (2026-10-22)

**Context.** AI agents should be able to drive Burrow: open and close tabs, set up spaces and folders, look at any tab, and use pages.

**Decision.**
- Burrow runs a small HTTP API on `127.0.0.1` at a random port, and writes `{port, token, pid}` to `<data>/agent.json` (readable only by the user on Unix; `%LOCALAPPDATA%` is already private on Windows). `POST /v1/<tool>` with JSON arguments; `GET /v1/tools` lists the tools.
- The same exe is the client. `burrow --mcp` is an MCP server on stdin/stdout, so any MCP agent (Claude Code, etc.) can use Burrow with one line of config. `burrow --agent <tool> '<json>'` calls a tool from a shell. If Burrow isn't running, both start it first.
- The tool list and every tool that only changes the model (spaces, folders, moving, closing) live in `burrow_core::agent` and are unit-tested. The app runs the tools that need a page: `open_tab`, `navigate`, `read_page`, `screenshot`, `click`, `type_text`, `press_key`, `wait_for` and `eval_js`.
- Page tools speak the DevTools protocol to each tab's own browser through CEF (`send_dev_tools_message` + an observer), not through `--remote-debugging-port`. No open debugging port is needed, and every tab can be reached, whichever is shown.
- **Agents work in the background.** `open_tab` doesn't switch tabs unless `focus` is set, and creating a space doesn't switch to it. A hidden tab's view has no size and doesn't paint, which made screenshots time out. Burrow gives the hidden page the content area's size with `Emulation.setDeviceMetricsOverride`, which makes it lay out and render as if shown, and clears the override when the tab is shown. A first attempt rendered the tab *behind* the shown one; it failed when no tab is shown, because a browser view's layer always draws over ordinary painted views, so nothing can cover it.
- **Safety.** Every request needs the token. Requests with an `Origin` header (from web pages) or a foreign `Host` (DNS rebinding) are refused. The sidebar shows a pulsing "Agent" badge while an agent is working. "Turn Off Agent Access" in the + menu turns the whole thing off (`settings.agent_access`, on by default). `read_page format=elements` never includes password field values. Keys sent by agents go straight to the page, so they never trigger Burrow's own shortcuts.

**Why.** One exe means no Node, Python or extra install for the user. MCP over stdio works from WSL calling the Windows exe, because WSL interop passes pipes, and the Windows release build (a GUI program) still reads and writes inherited pipes; this was checked from WSL. A plain HTTP API keeps it usable from any language or curl.

**Consequences.** Anything that runs as the same user can read `agent.json` and control Burrow, including the logged-in sites in every space. That is the same trust level as reading the profiles' cookie files, which such a program can already do. The user can turn agent access off. Tools act on a tab's main frame only.

## D-019 · Agents always manage tabs and spaces; page access can be switched off (2026-10-22)

**Supersedes** the on/off switch in D-018, which turned off all agent access.

**Context.** Is D-018 a security risk? The API itself is closed to web pages and other users. The real risk is prompt injection: an agent reading a hostile page could act with the user's logins. That needs page access (reading, screenshots, clicks, typing, scripts), not tab management.

**Decision.** By design, agents can manage tabs, folders and spaces at any time; there is no switch for that. Page tools (`burrow_core::agent::PAGE_TOOLS`: `read_page`, `screenshot`, `click`, `type_text`, `press_key`, `wait_for`, `eval_js`) are on by default, and **+ → Block Agents from Pages** turns them off (`settings.agent_page_access`). A blocked call tells the agent that tab and space tools still work.

**Consequences.** With pages blocked, an agent can still open tabs and navigate them (it can't see the result). `open_tab` and `navigate` still report the page title, which is what the sidebar shows anyway.

## D-020 · First release (0.0.1): Linux and Windows archives built from WSL; no macOS yet (2026-10-22)

**Decision.** `scripts/package.sh` builds release archives into `dist/`. Each has a `Burrow/` folder with the executable, the CEF runtime, `ui/`, the license, and CEF/Chromium's credits:
- `burrow-<version>-windows-x64.zip`: the same files `install-win.sh` installs. It isn't code-signed, so Windows SmartScreen warns on first run.
- `burrow-<version>-linux-x64.tar.gz`: `libcef.so` is stripped (CEF ships it with ~1.4 GB of debug symbols; stripped, the archive is ~150 MB).

Releases are GitHub releases tagged `v<version>`, with the archives attached. The workspace version starts at 0.0.1.

**No macOS build.** It can't be made from WSL: it needs the macOS SDK, an `.app` bundle with CEF's framework and helper apps, code signing, and changes in `main.rs` (CEF on macOS loads its framework at runtime). It needs a Mac or a macOS CI runner, and someone to test it.

**Verified.** The Linux archive was extracted and run on Xvfb (sidebar, a page, and agent tools). The Windows archive has the same runtime files as the working per-user install. The packaged Windows build wasn't launched, because another Burrow was running on that machine.

## D-021 · Air Traffic Control: rules that send links to a space (2026-10-22)

**Context.** Arc's Air Traffic Control was requested.

**Decision.** `State.routes` holds rules `{pattern, space_id}`, edited in **+ → Air Traffic Control…**, from a tab's menu ("Send This Site to a Space…") or by agents (`air_traffic_control` tool).
- A pattern without `/` is a domain and also matches its subdomains (`figma.com` matches `www.figma.com` and `help.figma.com`, not `notfigma.com`). With `/` it matches addresses starting with it, on whole path segments (`github.com/my-org` doesn't match `github.com/my-organizer`). Scheme and `www.` are ignored. The longest matching pattern wins.
- Rules apply to links that open a new tab or Peek (target=_blank, Ctrl+click, the page menu) and to links from other programs (D-024). A link to a site whose rule points at the space it is already in is not routed.
- Rules do **not** apply to what the user types in the address box, or to links clicked inside a page that stay in the same tab: in both cases the user chose where they are.
- A routed foreground link switches to the target space; a background one just appears there. A toast names the space.
- Deleting a space deletes its rules.

**Why.** Arc's own rules work the same way (domain or "URL contains"). Whole-segment matching avoids surprising matches.

## D-022 · Peek: a page in a card over the current one (2026-10-22)

**Context.** New links should open not as new pages but as a popup over the page, like Arc's Peek.

**Decision.**
- Links that would open a new tab in the foreground (target=_blank), `window.open` popups, and "Open Link in Peek" in the page menu open in a **Peek**: a card over the content area with the page's title, **Open as Tab** and ✕, over a dimmed page. Esc or Ctrl+W closes it. Open as Tab moves the live browser into a new Today tab, without reloading.
- In **pinned tabs and favorites**, a link click to another site opens in a Peek, so the pinned tab stays on its own site (Arc's behaviour). It only catches real link clicks (user gesture, link transition, main frame, not a redirect), never form submissions, and never sign-in pages (`links::looks_like_sign_in`: `login.`, `accounts.`, `/oauth`, `/saml`, …), so a sign-in's redirect back lands in the tab.
- Explicit choices stay tabs: Ctrl/middle-click (background tab), Shift+click and "Open Link in New Tab".
- Both behaviours can be turned off in **+ → Links and Peek…** (`settings.peek_new_tabs`, `settings.peek_from_pinned`, both on by default).
- A Peek uses the profile of the tab it came from. It belongs to that page: switching tabs or spaces closes it. A popup from inside a Peek becomes a tab.
- A Peek is a CEF **overlay** of the one window (`Window::add_overlay_view`), never a second window. Notes: CEF resets the colors of views placed in an overlay, so they are set again after attaching. An overlay can't take keyboard focus until it is clicked, and requesting focus into it before that loses the keyboard entirely; so the keyboard stays with the page behind (Esc and shortcuts work at once), and clicking the Peek moves typing into it.

**Verified** on Linux/Xvfb with real clicks: target=_blank → Peek; Esc closes; Open as Tab moves the page into a tab without reloading; `window.open` → Peek, and `window.close()` from it closes the Peek; a pinned tab's link to another site → Peek while the tab stays put; closing the window with a Peek open exits cleanly. Not yet checked on Windows.

## D-023 · History in its own file, searched from the address box (2026-10-22)

**Decision.** Visits are kept in `<data>/history.json`, one entry per URL (title, last visit, count), newest first, at most 20,000, saved at most every 10 s. Only http(s) pages are recorded, including pages seen in a Peek. The address box shows matching open tabs first, then history (every typed word must match the title or URL; ranked by visit count, recency and host-prefix matches). **Ctrl+H** opens a searchable History list in the sidebar with per-entry delete and Clear History.

**Why.** History changes on every page load; keeping it out of `state.json` keeps that file small and its saves cheap. JSON is enough at this size (a SQLite move stays open, as for D-005). The sidebar asks Rust for matches (`suggest`) instead of receiving the whole history on every render.

## D-024 · Burrow as the default browser; links from other programs (2026-10-22)

**Decision.**
- `burrow.exe <url-or-file>` opens it. If Burrow is already running, the new process hands the link to it over the local API (`POST /v1/external`, same token as D-018) and exits, so a link never starts a second window or fails on the profile lock. Chromium's helper processes (`--type=…`) are ignored.
- The link opens as a foreground Today tab in the space Air Traffic Control picks (else the active space), and the window is restored and brought to the front.
- **+ → Make Burrow Your Default Browser…** (Windows only) writes the per-user registry entries Windows needs to list Burrow as a browser (`HKCU\Software\Clients\StartMenuInternet\Burrow`, `RegisteredApplications`, `BurrowURL`/`BurrowHTML` classes for http, https, .htm, .html, .pdf) with `reg.exe`, then opens Settings → Default apps, where the user picks Burrow. Windows doesn't let programs make themselves the default.

**Not verified on Windows yet** (the registry was not written on the development machine). Unit tests check the registry values and argument parsing; the handoff to a running Burrow was verified on Linux.

## D-025 · Site icons cached on disk (2026-10-22)

**Problem (reported with a screenshot).** Favorites showed a globe instead of their icon. Burrow only knew a tab's icon once its page had loaded in this run; otherwise it guessed `<site>/favicon.ico`, which many sites (Notion, Gmail, Asana, rust-lang.org, …) don't serve.

**Decision.** Icons are saved as PNGs in `<data>/favicons/<host>.png` (up to 128 px), and the sidebar shows them by file URL.
- When a page reports its icon, Burrow downloads it (`BrowserHost::download_image` on the sidebar's browser) and saves it, once per site per run.
- For any tab whose site has no saved icon (e.g. favorites right after an Arc import), Burrow fetches the tab's home page in its space's profile (a `URLRequest`, so signed-in apps answer as usual; 4 at a time), reads its `<link rel=icon|apple-touch-icon>` tags (`burrow_core::favicon::candidates`, largest first), and saves the first one that downloads, falling back to `/favicon.ico`.
- The sidebar tries, in order: the saved icon for the tab's current site, for its home site, the page's reported icon, `/favicon.ico`, then a globe.

**Gotcha.** `Image::as_png` returns nothing unless it is given places to write the pixel size (cef-rs passes null for `None`, which CEF rejects).

**Verified** on Linux/Xvfb: with the icon folder deleted and no tab loaded, all 11 test sites got their real icons within seconds, including the ones that showed globes before.

## D-026 · Split View, fullscreen, zoom memory, Boosts and small tab actions (2026-10-22)

- **Split View.** `Space.split = {left, right}`: while either tab is active, both show side by side (the content panel is now a horizontal box layout; hidden tabs are skipped by it). Clicking into a half makes it the active tab, so the address box and buttons follow. Closing a half shows the other; moving a half to another space, clearing or archiving it ends the split. Auto-archive keeps both halves of a shown split. Right-click a tab → Open in Split View / Close Split View.
- **Fullscreen.** A page going fullscreen (videos) makes the window fullscreen and hides the sidebar, the card margins and the other half of a split. Escape leaves it: Chrome does that itself, CEF's Alloy style doesn't. F11 toggles Burrow's own fullscreen.
- **Zoom** is remembered per site (`State.zoom`, by host) and reapplied when a page of that site loads. The address box shows a zoom chip (e.g. 120%) when it isn't 100%; clicking it resets.
- **Boosts.** Right-click a tab → Boost This Site… adds the user's CSS to every page of that host (`State.boosts`), applied on each load and immediately to open pages.
- **Tab menu:** Duplicate Tab, Mute/Unmute Tab (a muted tab shows an icon). **Shortcuts:** Ctrl+Shift+C copies the address, Ctrl+P prints, Ctrl+H history, F11 fullscreen.
- Agents got an `air_traffic_control` tool (list/add/remove rules). Page tools on a split tab no longer emulate a viewport size, since that tab is shown.

## D-027 · Fixes found while testing (2026-10-22)

- **Pages that close themselves now close.** `do_close` returns 1 so closing one tab doesn't close the window (D-010 #5), but that means Burrow must remove the view itself. It never did for `window.close()`, so self-closing tabs and sign-in popups stayed open. `do_close` now asks the controller to close that tab or Peek.
- **Pages without a background are white.** Tab browsers had a transparent default background, so such pages showed the sidebar's tint behind them. Tabs and Peeks now default to white, as in Chrome.
- On Xvfb, the window's first frame can show without the content card until something changes. The build before these changes does the same, so it isn't a regression; it hasn't been seen on Windows.

## D-028 · Agents can use every new feature (2026-10-22)

**Context.** Agents should be able to do everything the new features do, Boosts above all.

**Decision.** New agent tools (MCP, CLI and API, D-018):
- In `burrow_core::agent` (model only, unit-tested): `split_view` (open/close), `duplicate_tab`, `link_settings` (the two Peek switches), plus `air_traffic_control` from D-026.
- In the app (`tools.rs`, `app_tool`): `boost` (list/get/set/remove, by host or tab), `search_history`, `delete_history`, `peek` (status/open/close/expand), `set_zoom` (percent, remembered per site) and `mute_tab`.

**Agents still don't disturb the user (D-018).** `split_view` never changes the active tab: the split appears only when the user is on one of its tabs. `duplicate_tab` stays in the background unless `focus` is set. `peek open` is the exception, since a Peek only exists over the tab the user is looking at; its description tells agents to use it only when asked. None of these are page tools (D-019): they don't read or act on a page's content, so they work when pages are blocked. Boosts do change how pages look; that is the user's own choice of CSS, set on request.

**Verified** on Linux/Xvfb through `burrow --agent`: a Boost set by an agent restyled an open page; history search and delete; zoom 125% saved for the site; mute; duplicate; split open/close; Peek open/status/close; the shown tab stayed the same throughout.

## D-029 · Windows package runs on a clean PC; a second launch focuses the window (2026-10-22)

**Context.** The 0.0.1 Windows archive had never been launched. Checking it as a downloaded package found two problems:
1. `burrow.exe` imported `VCRUNTIME140.dll` (the Visual C++ Redistributable), which the archive doesn't include. Most PCs have it from other apps, but a clean Windows install may not, and Burrow wouldn't start there.
2. Starting Burrow while it was already running (a second click on a shortcut) made CEF open a separate plain Chromium window ("New Tab - Chromium", with Google's new-tab page). That breaks the one-window rule.

**Decision.**
- Link the C runtime statically on Windows (`.cargo/config.toml`: `+crt-static`). `burrow.exe` now imports only Windows system DLLs and `libcef.dll`. None of the bundled CEF DLLs need the VC runtime. CEF's wrapper library was already built with the static runtime (`/MT`, by cef-dll-sys), so this also stops mixing two runtimes in one exe.
- Implement `on_already_running_app_relaunch`: return 1 and bring the existing window to the front (restoring it if minimized). The second process exits.
- Embed Windows version info in `burrow.exe` (product name "Burrow" and the Cargo version), so Explorer's file properties, Task Manager and SmartScreen show "Burrow". `build.rs` fills the version into `resources/burrow.rc`.
- `package.sh` writes `dist/SHA256SUMS.txt` for the release page.
- The README says to keep the `Burrow` folder together, where the data lives (outside the app folder, so updating means replacing the folder), and how to uninstall.

**Verified.** On Windows: the archive was extracted with `Expand-Archive` into a new folder and run with a throwaway data folder, alongside an installed Burrow, which was left running. Checked: the sidebar loaded from the app folder; a real HTTPS page loaded; the agent CLI could list, read and screenshot a tab; a graceful close saved `state.json` and the tab came back after a restart; and a second launch exited and restored the minimized window, with no extra window. Logs and caches go to the data folder, so the app folder can be read-only. The Linux archive was checked the same way on its own Xvfb display: a page loaded, the agent CLI worked, and a second launch exited with no extra window.

**Still open.** The exe isn't code-signed, so SmartScreen warns on first run (needs a certificate). There's no installer, Start menu entry or auto-update for zip users; `scripts/install-win.sh` covers the development machine.

## D-030 · A regular Windows installer (NSIS, per-user); signing plugs in later (2026-10-22)

**Context.** Burrow needed "a regular install" and a signed app.

**Decision.** `scripts/package.sh` also builds `burrow-<version>-windows-x64-setup.exe` from `installer/burrow.nsi` with NSIS, next to the zip. NSIS was chosen because its compiler runs on Linux, so the whole release still builds from WSL (D-002). Inno Setup and WiX need Windows to build.
- **Per-user, no admin**, into `%LOCALAPPDATA%\Programs\Burrow`. That is the same folder `install-win.sh` uses, so an existing install is upgraded in place. Chrome's and VS Code's user installers work the same way, and a per-user install doesn't need UAC.
- The installer adds a Start menu shortcut and an entry in *Settings → Apps* (HKCU uninstall key with version, size, publisher and icon). The finish page has "Start Burrow" and an "Add a desktop shortcut" checkbox.
- **Updates and uninstalls handle a running Burrow:** they ask, then close it gracefully (it saves), wait up to 20 s, and offer Retry. The installer is 32-bit and can't see a 64-bit process's path through a 32-bit PowerShell, so it calls the 64-bit PowerShell via `Sysnative`.
- **The uninstaller deletes only the files it installed** (a list generated at build time), never the whole folder, in case someone installed into a folder that holds other things. It asks whether to delete the data in `%LOCALAPPDATA%\Burrow`; the default, and the answer in silent mode, is No.
- Silent install and uninstall work: `setup.exe /S [/D=folder]` and `uninstall.exe /S`.
- The zip stays available for people who don't want an installer.

**Signing.** Signing needs a certificate tied to a verified identity, which only the project's maintainer can get. The build is ready for it: set `BURROW_SIGN` to a command that signs a file in place. `package.sh` then signs `burrow.exe`, and NSIS signs the setup and the uninstaller it embeds (`!finalize` / `!uninstfinalize`).

**Verified on Windows** (silent mode, installed to a temp folder, with the existing Start menu shortcut backed up and restored): a fresh install put the files, registry entry and shortcut in place, and Burrow ran from it. An upgrade while Burrow ran closed it, it saved, and the files were replaced. An uninstall while Burrow ran removed the files, the folder, the registry entries and the shortcut, and kept the data. The existing `%LOCALAPPDATA%\Burrow` data was checked before and after and was untouched. The welcome page was captured to check the artwork. The other pages and their buttons weren't clicked through by hand.

## D-031 · Release 0.0.2 (2026-10-22)

**Contents.** Everything in D-021–D-028 (Air Traffic Control, Peek, history, default browser and links from other programs, cached site icons, Split View, fullscreen, zoom memory, Boosts, fixes, agent tools), plus the packaging work in D-029–D-030. Downloads: the Windows setup .exe (the recommended download now), the Windows zip, the Linux tarball, and `SHA256SUMS.txt`. Not signed (D-030).

**Verified before publishing.** On Windows, the 0.0.2 setup was installed silently into a temp folder with a throwaway data folder. Burrow ran and the agent CLI worked. A link passed to `burrow.exe` while it ran opened as a tab in the running window (D-024's handoff, until now only verified on Linux). A plain second launch exited with one window still open. The uninstaller removed everything and kept the data. The existing Start menu shortcut and data were untouched. The Linux tarball was run on its own Xvfb display: a page, the agent CLI and the link handoff worked, with one window.

## D-032 · Chrome extensions: installed by Burrow, loaded at startup, popups in a card (2026-10-22)

**Context.** Extension support was requested. Burrow's tabs are Alloy-style (D-010). Tested in CEF 154 on Xvfb: CEF's old extension API is gone, and `chrome://extensions` is blocked in Alloy-style browsers, so there is no built-in way to install or manage extensions, and the Web Store's "Add to Chrome" shows "Switch to Chrome". But extensions loaded with Chromium's `--load-extension` switch **do run** with Alloy-style tabs: background service workers, content scripts, `chrome.runtime` messaging, storage, `chrome.tabs.get/update/sendMessage` by tab id, and their own pages (`chrome-extension://…`).

**Decision.**
- **Installing.** Burrow downloads the extension's `.crx` from the Chrome Web Store's update server (the same URL Chrome uses, asking for Chromium 154), or takes a folder or a `.crx`/`.zip` from disk. `burrow_core::extensions` unpacks it into `<data>/extensions/<id>/<version>_<n>/`, deletes `_metadata` (Chromium refuses unpacked extensions with reserved `_` names), and writes a `key` into its manifest: the store's own public key from the CRX3 header, so the id matches the store's; a made-up one for files without a key, so the id doesn't depend on the folder path. The list lives in `State.extensions`.
- **Loading.** At startup Burrow passes the enabled extensions' folders in `--load-extension`. They load in **every space's profile**, each with its own storage, so an extension's sign-ins are per space like the sites' (D-004). Spaces created later get them too (verified).
- **Changes need a restart.** Chromium reads the switch only at startup, so installing, updating, removing and turning on/off take effect after a restart. The sidebar shows a "Restart Burrow" card (with Later) and the Extensions dialog a Restart Now button. Restarting closes the window gracefully (state saved) and `main` starts the exe again once CEF has shut down and released the profile; command-line links aren't reopened. Updates go into a new version folder and removed extensions stay on disk until the next start, so the files of the running version are never touched (Windows locks them); `extensions::clean_up` deletes unused folders before Chromium starts.
- **UI.** A puzzle button in the sidebar's toolbar lists the enabled extensions (with their icons) and opens a clicked one's popup; it also has Manage Extensions… and a Chrome Web Store link. On a Web Store extension page the sidebar offers **Add to Burrow**. **Manage Extensions…** (also in the + menu) has on/off switches, Remove, Options, Check for Updates, and installing from a link, a folder or a `.crx` file. No auto-update yet.
- **Popups** open in a small card over the top left of the page (a window overlay, never a window), in the active space's profile, sized by the page itself (CEF auto-resize, Chrome's limits 25×25 to 800×600). Clicking the page or the sidebar, Esc, switching tabs or `window.close()` closes it, as in Chrome.
- **Popups and `chrome.tabs`.** Alloy tabs aren't Chrome tabs: in a popup, `chrome.tabs.query({active: true})` finds nothing, `tabs.create` fails ("No current window"), and `tabs.getCurrent` returns the popup itself (Bitwarden then thinks it's in a tab and lays out at 25 px wide). Before a popup's page loads, Burrow adds a script through DevTools (`Page.addScriptToEvaluateOnNewDocument`; it needs `Page.enable` first) that answers `tabs.query` with the user's tab(s), opens `tabs.create`/`openOptionsPage` through Burrow (the popup's console, read only for the popup's own browser), and makes `getCurrent` return nothing.
- **Tab ids.** CEF's browser id is *not* the extension APIs' tab id in Alloy style (CEF's header says it is; it isn't: 2 vs 1238539899). CEF has no way to ask. Burrow therefore loads a tiny **helper extension** of its own (written to `<data>/extension-helper/` at startup, only when other extensions are on): its content script asks its service worker for `sender.tab.id` and logs it with a secret made fresh each run, which pages can't read, so they can't fake it. Burrow keeps the id with the tab (`LiveTab::ext_tab`).
- **The sidebar moved from `file://` to `burrow://ui/`.** Chromium gives unpacked extensions access to file URLs, and the first test showed a test extension's content script running *inside the sidebar*. No extension can match a custom scheme. `scheme.rs` serves `burrow://ui/…` (the sidebar), `burrow://icons/…` (D-025's saved icons, which were file URLs too) and `burrow://ext/…` (extension icons) from their folders, only plain relative paths, on the global request context only (the sidebar's; web pages live in the spaces' profiles).
- Agents: the `extensions` tool (list, install from the store or a path, update, enable, disable, remove, open/close a popup, restart).

**Verified.** On Linux/Xvfb with real clicks: Web Store installs of Bitwarden, Dark Reader and uBlock Origin Lite (the last from the "Add to Burrow" card) with their store ids; Chromium's ids equal the ones Burrow computed; the restart card and its button; Dark Reader darkens pages, also in a space made after startup; a test extension's content script runs in pages and not in the sidebar; the Bitwarden popup lays out at its normal size; a test popup's `tabs.query` returns the user's tab with its real id and `tabs.create` opens a tab; clicking the page closes the popup. On Windows (a separate copy with a throwaway data folder, next to an installed Burrow that kept running): sidebar over `burrow://`, folder and store installs, restart, extensions loaded, content scripts in pages only, popup open.

**Limits.** `chrome.tabs`/`chrome.windows` calls from an extension's *background* (e.g. opening a welcome tab after install, `windows.create` for Bitwarden's "pop out") still fail: only popups get the fix. No `activeTab` grant on click, so extensions that rely on it alone can't act on the page. No extension badges or icons in the toolbar, no context-menu items, no keyboard commands, no side panels. Extensions are "unpacked" to Chromium, so Web Store content verification doesn't apply.

## D-033 · Resizable sidebar (2026-10-22)

**Decision.** Drag the sidebar's right edge (a 6 px handle that shows a line on hover) to set its width, 200–600 px, also never leaving the page less than 320 px. Double-click the edge for the default 264 px. The width is saved in `settings.sidebar_width` and used when the hidden sidebar peeks out. While dragging, the sidebar sends one width per animation frame and Rust lays the window out; it's saved when the mouse is released. The favorites grid now fills the width with as many 68 px+ columns as fit (3 at the default width). Agents: the `sidebar` tool (width, visible).

**Verified** on Linux/Xvfb with a real mouse drag (264 → 403 px, saved) and double-click reset; on Windows through the agent tool.

## D-034 · Performance: saves off the UI thread, fewer sidebar redraws (2026-10-22)

**Context.** A look at where Burrow spends time. Every page event (address, title, icon, loading) re-sends the whole view and the sidebar rebuilds its rows; `state.json` was written on the UI thread 1.5 s after each change and `history.json` (up to 20,000 visits, several MB) every 10 s while browsing.

**Decision.**
- **Saves** are still serialized on the UI thread (a consistent snapshot, a few ms at most) but written on a background thread (`writer.rs`). Each write gets a version number and holds one lock, so writes never overlap and an older snapshot never replaces a newer one. On shutdown the write happens before returning, as before. `store::write_atomic` is the shared temp-file-then-rename write.
- **Sidebar.** Renders are drawn once per animation frame, so a burst of views draws only the last. Icons that fail to load are remembered for the session: before, every redraw rebuilt the rows and downloaded the same missing icons (12 broken icon URLs in a real sidebar of about 85 tabs and folders) again. The toolbar's SVG icons are reparsed only when they change.

**Measured** on Linux/Xvfb with that sidebar: one full render takes ~1.6 ms, and loading a page sent 18 views and drew 16, so the sidebar itself was never the bottleneck; the repeated icon downloads and disk writes on the UI thread were the waste.

**Not done.** Idle tabs are never unloaded automatically, so memory grows with every tab opened in a session. Arc-style auto-unloading would change behavior (unsaved form text is lost) and needs a decision first.

## D-035 · Window buttons in the corner, shown on hover; a cleaner sidebar top (2026-10-22)

**Context.** Minimize, maximize and close sat at the right end of the sidebar's toolbar, next to back/forward/reload and the extensions button, which crowded it. A redesign was requested: the buttons on the right, out of sight until the mouse goes there, as in Arc on Windows.

**Decision.**
- The buttons are native CEF buttons in a window **overlay** (never a window) in the window's **top-right corner**. At rest the overlay is only the 8 px strip above the content card, 138 px wide, painted the space color (the window's own color there) and with no glyphs, so it is invisible and never covers the page. Hovering that strip (CEF's button hover state, `on_button_state_changed`) slides out 46×32 px buttons over the page's corner, Windows-style: Segoe MDL2 Assets glyphs on Windows, a red close on hover. They go back half a second after the mouse leaves them. Not shown in fullscreen, nor with the system title bar.
- The whole strip above the card is a drag area for moving the window, so it can be moved with the sidebar hidden too.
- The sidebar's toolbar is now Arc's: the sidebar toggle and extensions (and the Agent badge) on the left, back/forward/reload on the right.
- Agents: the `window` tool (status, minimize, maximize, restore, fullscreen on/off, show_buttons for two seconds).

**Verified** on Linux/Xvfb with real mouse moves: nothing visible at rest, the buttons appear when the corner is hovered and hide after leaving; the `window` tool shows them.

## D-036 · Picture in picture, Arc-style (2026-10-22)

**Context.** Picture in picture for all videos, like Arc, was requested: leave a tab while its video plays and the video keeps playing in a small floating window.

**Finding.** Chromium's own picture-in-picture works in CEF 154 with Alloy-style tabs: `document.pictureInPictureEnabled` is true and `requestPictureInPicture()` opens Chromium's always-on-top video window, with its play/pause, "back to tab" and close buttons. This is the same window Arc uses. It is the one exception to "one window" (D-003): it holds only a video, not a page, and closes when the video goes back.

**Decision.**
- **Auto picture in picture** (on by default; + menu to turn off): when a tab leaves the content area (another tab, another space, or out of split view), Burrow asks its page through DevTools for a video that is playing **with sound** and floats the largest one. Muted loops (background videos on sites) never pop out. Pages may only start picture in picture after a user gesture; `Runtime.evaluate` with `userGesture` gives one. Coming back to the tab puts the video back. Not while a page is fullscreen.
- **By hand**: right-click a tab → Picture in Picture (any playing video, else any loaded one), or Ctrl+Shift+P for the shown tab. A small icon on the tab row shows the floating video; clicking it puts it back.
- **Back to tab.** CEF reports nothing when the window's "back to tab" button is pressed (no focus callback, checked). Pages learn through `leavepictureinpicture`, reported to Burrow with a DevTools binding (`Runtime.addBinding`, `__burrowPip`). Chromium pauses the video when the window is *closed* but not on "back to tab", so a video still playing means "back to tab": Burrow selects the tab, in its space, and brings the window to the front. A live stream (a call, `srcObject`) can't pause, so there the two look the same and Burrow does nothing. Pages can call the binding themselves; Burrow believes "entered" only after checking `document.pictureInPictureElement`, and "back" only for the tab it knows is floating.
- Agents: the `picture_in_picture` tool (status, open, close, auto on/off).

**Verified** on Linux/Xvfb: switching tabs floated a playing video (the floating window appeared, the tab got its icon); clicking the tab again closed the window with the video still playing in the page; "back to tab" in the floating window showed the tab; its close button paused a file video and stayed on the current tab, and for a live stream also stayed; Ctrl+Shift+P toggled it; the agent tool reported and refused correctly.

**Limits.** Videos inside iframes (e.g. a YouTube embed in an article) aren't found: the script runs in the top page only. Chrome's "auto picture in picture" for video calls when the whole app loses focus isn't done.

## D-037 · Auto-update from GitHub releases (2026-10-22)

**Decision.**
- **Checking.** Burrow asks GitHub's API for the latest release (`/repos/moishebin/burrow/releases/latest`, which skips drafts and pre-releases) 20 s after starting and every six hours, through CEF's URL requests (no new HTTP library). It compares the tag with its own version (`burrow_core::update`, unit-tested). Development builds don't check by themselves. **+ → About and Updates…** shows the version, checks now, and turns automatic checks off (`settings.auto_update`).
- **Installing (Windows, setup installs only).** A copy the setup installed (its `uninstall.exe` is next to `burrow.exe`) downloads the release's `burrow-<v>-windows-x64-setup.exe` into `<data>/updates/` in the background, then checks it against the release's `SHA256SUMS.txt`. A file that doesn't match (or has no line) is deleted and never run. Once it checks out, the sidebar offers **Restart** (or Later). Restart closes the window gracefully; after CEF shuts down, `main` starts `setup.exe /S /RELAUNCH /D=<this copy's folder>`. The installer already waits for a running Burrow to exit (D-030). The new `/RELAUNCH` option starts Burrow when it's done. If the user doesn't restart, the update installs silently when Burrow quits, and the next start is the new version, as in Chrome. Files Burrow downloads itself carry no "downloaded from the internet" mark, so Windows doesn't warn. The checksum file comes from the same release, so it catches broken downloads, not a compromised release. Signing (D-030) is what would cover that.
- **Other copies** (the zip, Linux, development builds) only say "Burrow X is out", with **Download** opening the release page.
- `/D=` is passed so an update always replaces the copy that is running, never whatever folder the registry remembers.
- `BURROW_UPDATE_URL` points the check at another server answering the same JSON (for testing).
- Agents: the `update` tool (status, check, install, auto on/off).

**Verified** on Linux/Xvfb against a local fake release (0.0.99): the automatic check found it; with the install path forced on for the test, the setup downloaded, passed the checksum and the Restart card appeared; a wrong checksum deleted the file and reported it; without the install path, the "is out" card with Download appeared. A real check against GitHub reported 0.0.2 as current. The Windows install step (`/S /RELAUNCH`) is verified on Windows in D-043.

## D-038 · Resizable split view (2026-10-22)

**Decision.** A split has a `ratio`, the left tab's share of the width (0.15–0.85, default 0.5, saved; `Command::SetSplitRatio`). Between the two tabs sits a **divider** as wide as the gap between cards (8 px), painted in the content card's tint. Dragging it moves the split live, and it is saved on release; double-clicking gives equal halves. CEF's native views have no mouse-move events, so the divider is a tiny page (`ui/divider.html`, on `burrow://`), the same trick as the sidebar's resizable edge (D-033): pointer capture keeps the drag going outside its 8 px, and it reports one movement per frame through the sidebar's message channel. It is created the first time a split shows and closes with the window.

The content area is a box layout. Its flex weights share out the *difference* from each view's preferred width, and a BrowserView's preferred width is large, so the weights worked backwards at first (a 0.81 split showed as 0.18). Tabs and Peeks now report a 1×1 preferred size and take what the layout gives them. Extension popups keep their natural, auto-resized size (`TabViewDelegate { fills }`).

Agents: `split_view` takes `ratio` when opening and has a `resize` action.

**Verified** on Linux/Xvfb: the divider sits exactly at the saved ratio (0.8125 → x 1174 of a 1120 px area starting at 264); a real mouse drag of −400 px landed where computed; double-click reset to halves and saved; the agent's resize to 0.25 applied at once; a Peek still lays out as before.

## D-039 · Dialogs open over the whole window, not inside the sidebar (2026-10-22)

**Context.** Every dialog (History, Extensions, Air Traffic Control, Links and Peek, Boosts, spaces, settings, About and Updates) was drawn inside the sidebar's own page, squeezed to its width. Reported: they should show over the main browser.

**Decision.**
- A dialog opens **centered over the window** on a dimmed backdrop, using two overlays of the one window (never a second window). The backdrop is a native button covering the window, so clicking outside the dialog closes it. Above it sits a card with a page that **sizes itself to the dialog** (CEF auto-resize, up to the window minus a margin; long lists scroll). CEF paints pages in overlays opaque (tested: a transparent page showed a flat color), so the page can't draw the backdrop itself.
- The card runs the sidebar's own page with `?modal`. In that mode it shows only the dialog, so every dialog's code stays as it was. In the sidebar, each dialog function (`DIALOGS` in sidebar.js) is replaced by one that asks Rust to open it by name (`open_dialog`); a space travels as its id. The card gets the same state pushes as the sidebar while it is open, and History's search answers go to both.
- **Keyboard.** The keyboard stays with the page when a dialog opens, so Esc closes it from anywhere (the key handler knows a dialog is open). Clicking a field in the dialog moves typing there. This is the Peek approach (D-022): focusing into an overlay before it is clicked loses the keyboard altogether. Like a Peek, every dialog gets **new overlays** and they are destroyed on close. Reusing hidden overlays was tried first: an overlay that is hidden and shown again took the keyboard away from every page on Linux, so Esc stopped working after the first dialog.
- **Fixed on the way:** typing into a dialog after another dialog had been open did nothing. `d.onkeydown = (e) => e.key === "Enter" && …` returns `false` for every other key, which cancels it, and `openDialog` never cleared the previous dialog's handler. The handlers now have block bodies, and `openDialog` resets `onkeydown`. The bug was there in the sidebar too.
- Menus (right-click, +, the puzzle button) stay in the sidebar, as in Arc. Peeks and extension popups were already over the page.

**Verified** on Linux/Xvfb with real input (on a display of its own: another checkout's test Burrow shared :99, so `run-xvfb.sh` now takes `BURROW_XVFB_DISPLAY`): History (Ctrl+H) opens centered with the page visible and dimmed behind; searching filters it; Esc from the page closes it, three times running; clicking the backdrop closes it; Extensions and Air Traffic Control from the + menu; Rename Space from the space menu gets its space, takes typing and saves with Enter, twice in a row.

## D-040 · Sleeping tabs (2026-10-22)

**Context.** A tab's page kept running until the tab was closed or unloaded by hand, so memory grew with every tab opened in a session (D-034).

**Decision.** A tab out of sight for **10 minutes** (the default, `settings.sleep_after_minutes`) is put to sleep: its browser is closed, the tab stays where it is, and the page loads again at its current address when shown. Scroll position and the page's back/forward list are lost, as after a restart.
- **Per tab.** Right-click → *Sleep After* gives a tab its own time (5 min … 4 hours, or Never; `Tab::sleep_after_minutes`, `None` = the default). *+ → Sleeping Tabs…* sets the default (0 = never). *Sleep Now* unloads a tab that isn't on screen.
- **What stays awake.** Tabs on screen (both halves of a shown split), tabs using the camera or microphone (CEF's `on_media_access_change`: calls), and tabs waiting on a permission prompt. Before sleeping a tab, Burrow asks the page (`Runtime.evaluate` over the tab's DevTools connection, no user gesture) whether it has a picture-in-picture window (D-036), audible media playing, a text field holding typed text that differs from its default, or the cursor in a rich editor (`contenteditable`) that holds text. Frames from other sites (an embedded YouTube player) run in their own process, so the page can't see into them: Burrow turns on `Target.setAutoAttach` (flattened, without pausing frames) the first time, keeps each frame's DevTools session, and asks every frame the same question. If anything is in use the tab stays awake and is asked again after another full period. A page or frame that doesn't answer counts as not in use.
- **When.** A check runs every minute; a tab counts from when it was last on screen (or created, for background tabs).
- **Model/app split.** `State::tabs_to_sleep` and `sleep_after_ms` decide who is due (unit-tested); `browser/sleep.rs` tracks when tabs were last seen, asks the page, and unloads.
- Agents: the `tab_sleep` tool (default, a tab's own time, sleep now).

**Limits.** CEF reports no "tab is playing sound" signal (its audio handler would take the sound away from the speakers), so sound is detected from `<audio>`/`<video>` elements: sound made only with Web Audio isn't seen. Frames nested inside a frame from another site aren't asked. A rich editor counts only while the cursor is in it.

## D-041 · Extensions: badges, pinned buttons, and tabs from their backgrounds (2026-10-22)

(D-040 is taken by sleeping tabs, built in parallel on another branch.)

**Context.** D-032's limits: Burrow's tabs aren't Chrome's, so an extension's background can't open a tab (`tabs.create`: "No current window") or a window, `tabs.query` finds nothing, and badges (like an unread count) show nowhere. Chromium does keep the badge, but nothing outside the extension can read it.

**Decision: a bridge script in each extension's background.**
- At every start, before Chromium loads them, Burrow writes `burrow_hook.js` into each enabled extension's folder and has it run first in the background. A service worker gets `burrow_bg.js`, which loads the hook and then the extension's own worker (`importScripts`, or `import` for module workers). The original's name is kept in `burrow_bg_original.txt`, so this can be redone at every start. MV2 background scripts get the hook prepended. Extensions with a background *page*, or no background, are left alone (`burrow_core::extensions::bridge_manifest`, unit-tested).
- The hook wraps `action.setBadgeText`/`setBadgeBackgroundColor` (also `browserAction`) to report to Burrow; `tabs.create` and `windows.create` (one window, always: a "window" opens as a tab); `tabs.query` when Chromium finds nothing, answered with the user's tab and the space's tabs, as D-032 does for popups; `windows.getCurrent`/`getLastFocused` when they fail.
- It talks to Burrow over HTTP on 127.0.0.1, on a port of its own, with a secret. Both are chosen once and kept in `<data>/bridge.json`, because a second `burrow.exe` runs the same startup code before handing over to the running one and must write the very same hook. Only `POST /ext` from an `Origin: chrome-extension://<id>` with the secret is served. The id comes from the Origin, which pages can't set; the secret is only in the extensions' files. Requests are "simple" (text body, no custom headers), so there is no CORS preflight. If the saved port is taken at start, a new one is saved for the next start.
- **Badges and pins.** A badge is kept per extension, and per tab when the extension set one for a tab. **Pinning** an extension (Manage Extensions → Pin, `Extension.pinned`) gives it a button in the sidebar's toolbar with its badge; clicking opens its popup, right-click unpins. Badges of unpinned extensions show as a dot on the puzzle button and next to their names in its menu.
- Agents: `extensions` gets `pin` / `unpin`, and `list` shows `pinned` and `badge`.

**Verified** on Linux/Xvfb with a test extension (MV3 service worker): the manifest was rewritten and the hook loaded; its badge "7" in its green appeared on its pinned toolbar button; `tabs.create` and `windows.create` from the background opened tabs; `tabs.query({active, currentWindow})` returned the tab the user was on. Called directly, the bridge refused a wrong secret and a web page's Origin.

**Limits.** Extensions run in every space's profile (D-032), and each copy's background can set the badge; a badge for all tabs shows what the last copy set. Burrow's tabs report window id -1 to extensions. Extension context menus, keyboard commands and side panels are still missing.

## D-042 · Website notifications as system notifications (2026-10-22)

**Finding.** In Burrow's Alloy-style tabs, Chromium accepts `new Notification(…)` once permission is granted (Burrow's prompt works, D-014) and fires `show`, but nothing reaches the screen. On Windows no toast arrived and Windows' notification database had no handler for Burrow or Chromium; on Linux no window appeared.

**Decision.**
- **Catching them.** As each page (and frame) starts loading (`LoadHandler::on_load_start`), Burrow runs a small script (`burrow_core::notify::page_script`) that replaces `window.Notification`, and `showNotification` on service worker registrations. Pages using those get standard behavior: `permission`, `requestPermission`, `show`/`error`/`click`/`close` events, `close()`. If the page's real permission is "granted", the notification goes to Burrow as a console message with a secret made fresh each run. The secret lives only inside the script's closure, so pages can't send fake ones. Permission stays Chromium's, through Burrow's prompts.
- **Showing them.** On Windows, as a **toast** through WinRT (the `windows` crate, Windows builds only), with the site as its attribution line. Burrow registers its own toast identity at every start (`HKCU\Software\Classes\AppUserModelId\Burrow.Browser`: name and icon), so toasts work without a Store package or a special shortcut. On Linux, `notify-send` if it is there.
- **Clicking one** shows its tab (in its space), brings the window to the front, and fires the page's `click` event, so a mail site can open the message.
- **+ → Turn Off Website Notifications** (`settings.notifications`). Agents: the `notifications` tool (list the latest, open one as a click would, enable/disable).

**Verified.** Linux/Xvfb: the page's `Notification` is Burrow's and permission is still Chromium's; a notification reached Burrow; `open` showed its tab and fired the page's `onclick` once; a fake message with the wrong secret was ignored. Windows (a test copy next to an installed Burrow): the toast reached Windows: its database recorded it under `Burrow.Browser`. The screen was locked, so the toast wasn't seen or clicked there.

**Limits.** Notifications a service worker shows on its own (web push while no page is open) don't go through the page script, so they still aren't shown. Notification icons and action buttons aren't shown. A toast clicked after Burrow was restarted does nothing.

## D-043 · Windows checks of this round (2026-10-22)

Done on the development machine while its screen was locked: no mouse, keyboard or screen capture, only DevTools and the agent API. The installed Burrow kept running throughout (its processes were counted before and after). Its registry entries and Start-menu shortcut were exported before and put back after; the installer of a test copy writes the same ones.

- **Auto-update (D-037), end to end.** A setup built from this code as 0.0.2 was installed silently into a test folder and run with its own data folder and `BURROW_UPDATE_URL` pointing at a local fake release (a second setup built as 0.0.99 and its `SHA256SUMS.txt`). The automatic check found 0.0.99 20 s after start, downloaded the 138 MB setup and checked it; `update install` closed Burrow, the setup replaced it (the exe now reports 0.0.99) and Burrow ran again. To rule out the agent client starting it, `setup /S /RELAUNCH` was also run with the test copy fully stopped and no agent calls: it started Burrow. The test copy's uninstaller then removed it.
- **Default-browser registration (D-024).** Not run for real: it would point the machine's settings at a test copy. The risky part, the quoting of `reg add` values, was checked by running `reg.exe` with exactly the command lines Rust builds, under a throwaway key: `"C:\Users\Some One\…\burrow.exe" "%1"` and the icon value read back exactly. (A small test program was blocked by Windows Defender, which flags unsigned programs that write the registry; it was deleted.)
- **Picture in picture (D-036).** The "Picture in picture" window opened next to Burrow's window.
- **Notifications (D-042).** Chromium alone dropped them; with D-042 the toast reached Windows.
- **Not checked here:** hovering the window buttons (D-035), dragging and resizing the frameless window, the split view divider and dialogs with real input; they were verified on Linux/Xvfb.

## D-044 · Release 0.0.3 (2026-10-22)

**Contents.** D-032–D-043 (Chrome extensions and their badges, resizable sidebar and split view, performance, the new window corner, picture in picture, auto-update, dialogs over the window, sleeping tabs, website notifications). The same downloads as 0.0.2 (D-031). Not signed (D-030).

**Upgrading.** 0.0.2 has no update check (D-037 came later), so 0.0.2 installs need the setup once; from 0.0.3 on, updates come by themselves. On the development machine, the installed 0.0.2 was upgraded with `setup /S /RELAUNCH /D=<its folder>`: it closed gracefully, the exe reports 0.0.3, and Burrow started again. Note for scripts: PowerShell's `Start-Process -Wait` on the setup doesn't return after `/RELAUNCH`, because it also waits for the Burrow the setup starts.

## D-045 · A top bar with the address, replacing the corner buttons (2026-10-22)

**Context.** On Windows the corner buttons of D-035 showed as missing-character boxes ("?" in a diamond): the Segoe MDL2 Assets glyphs didn't render in CEF's buttons. A change was also requested: a full bar over the top that pushes the page down instead of covering its corner, opens when the mouse goes anywhere along the top, and shows the page's address.

**Decision.**
- The overlay now spans the whole content card. At rest it is still the 8 px strip above the card in the space color. Hovering anywhere on it opens a 32 px bar: the active tab's URL on the left (clicking it copies the link, like Ctrl+Shift+C), minimize/maximize/close on the right (46 px each, red close on hover). While open, the content card gets a top inset of 24 px, so the page moves down below the bar rather than under it; the page relayouts on open and close. It hides half a second after the mouse leaves; the URL follows navigations while open.
- The glyphs are drawn in code (10×10 logical px with a 1 px stroke, 4×4 supersampled, at 1x and 2x) and set as button images, so no font is needed.
- The strip above the card is no longer a drag area: hover is what opens the bar, and a drag area takes the mouse away from the views (on Windows it becomes title-bar hit testing). The strip above the sidebar and the sidebar's own toolbar still move the window. The bar itself is not draggable, for the same reason.
- Agents: the `window` tool's `show_bar` action (old name `show_buttons` still works) opens the bar for two seconds; `status` reports `top_bar_open`.

**Verified** on Linux/Xvfb with real mouse moves: hovering the top of the card opens the bar with the URL and drawn icons and moves the page down; the close button turns red; the bar hides after leaving; clicking the URL shows "Link copied"; `show_bar` opens it. Not yet checked on Windows.

## D-047 · The top bar moves the window (2026-10-22)

(D-046 is taken by the floating sidebar, built in parallel on another branch.)

**Context.** D-045 made the strip above the card and the bar itself non-draggable, and people want to move the window by the top bar, as with any title bar.

**Decision.**
- The open bar is two window overlays: the address on the left (only as wide as its text) and the three buttons on the right. Between them, the window itself shows through (the card's background turns the space color while the bar is open), and that part is a drag area. One overlay across the whole bar can't work: on Windows an overlay is its own native child window, and the main window's drag areas (title-bar hit testing) don't reach under it. On Windows, dragging the bar of a maximized window restores it and moves it, as with a native title bar.
- Over a drag area the views never see the mouse, and on Windows hover also failed in the top pixels of a restored window. So on Windows, Burrow follows the cursor itself: every 100 ms it reads the cursor (`GetCursorPos`) and opens the bar when the cursor is over the strip above the card, and closes it half a second after the cursor leaves the bar. It ignores the cursor when another window is in front of Burrow's at that point (`WindowFromPoint`). The buttons' hover now only colors them there.
- Elsewhere (the Linux test build) Burrow can't read the cursor, so it keeps D-045's hover and the bar has no drag area.
- An agent's `show_bar` keeps the bar open for two seconds in both cases.

**Verified on Windows** with a separate test copy and real cursor input (`SetCursorPos`/`mouse_event`): at rest the page shows; hovering the strip 1, 4 and 7 px below the window's top opens the bar; the bar stays open with the cursor on the drag area; dragging there moved a restored window, and pulled a maximized one out of maximized; moving into the page closes the bar. The drawn button glyphs of D-045 render on Windows. On Linux/Xvfb the bar still opens on hover; Xvfb has no window manager, so nothing moves windows there.

## D-048 · A more compact sidebar, closer to Arc (2026-10-22)

**Context.** Compared with Arc side by side, Burrow's sidebar was too wide, its favorite tiles too big, and the address box above the favorites took too much room. Arc's sidebar is about 200 px wide, with favorites three to a row as roughly 54×42 px tiles.

**Decision.**
- Sidebar: 220 px by default (was 264). The minimum is now 180 px (was 200), so it can be made narrower than Arc's. A width someone already chose stays as it is; double-clicking the edge resets it to the new default.
- Favorites: tiles 40 px tall (was 46) with 18 px icons (was 20), at least 50 px wide (was 68), with 6 px gaps (was 7). At 220 px that is three to a row, about 63×40 px each.
- Address box: 28 px tall (was 34) with 12 px text, and 10 px less space around it.
- Agents: the `sidebar` tool takes 180–600 px.

**Verified** on Linux/Xvfb with eight favorites: three to a row at 220 px, with the address box and space header fitting above the folders.

## D-049 · The right icons for Google's apps (2026-10-22)

**Problem (reported).** Gmail, Google Voice, Sheets and other Google favorites showed the generic Google "G". There were three causes:
1. D-025's lookup fetched a favorite's page without the profile's cookies (a `URLRequest` sends none unless `UR_FLAG_ALLOW_STORED_CREDENTIALS` is set). Google answered with its sign-in or marketing page, and Burrow saved that page's icon. `mail.google.com`, `docs.google.com`, `voice.google.com` and `google.com` all had the same 1336-byte "G".
2. Icons were kept per host, but Gmail and Google Chat share `mail.google.com`, and Docs, Sheets, Slides and Forms share `docs.google.com`.
3. Even signed in, Google's pages aren't a reliable source. Signed out, they give no app icon at all.

**Decision.**
- Icons are saved under an *icon key* (`burrow_core::favicon::icon_key`). That is the host, plus the first part of the path on hosts with several apps (`SPLIT_HOSTS`: `mail.google.com`, `docs.google.com`), e.g. `docs.google.com/spreadsheets` → `docs.google.com_spreadsheets.png`. The sidebar computes the same key; the list of split hosts comes from Rust in the state.
- Google's own apps (Gmail, Chat, Sheets, Docs, Slides, Forms, Voice, Calendar, Drive, Meet, Keep) use Google's public product icons (`gstatic.com/images/branding/product/2x/<app>_2020q4_48dp.png`). They are used both for the lookup and when a loaded page reports its icon.
- The page lookup sends the profile's cookies. If the page redirects to another site (a sign-in page), only the asked site's `/favicon.ico` is tried, never the other site's icons.
- A pinned tab or favorite that is on another site than its home (e.g. a Google sign-in page) shows its home site's icon first.
- Icons saved before this (cache version 2, a `version` file in the icon folder) are looked up once more in the background and replaced when that succeeds; until then the old ones show.

**Verified** on Linux/Xvfb (signed out of Google): favorites for Gmail, Chat, Sheets, Docs, Voice, Calendar and GitHub got their own icons from an empty cache. A deliberately wrong saved icon was replaced on the next start, and the version marker was written.

## D-046 · The hidden sidebar slides out over the page (2026-10-22)

**Context.** Hovering the hidden sidebar's edge (D-016) widened the sidebar in the window's layout, so the page was squeezed and relaid out every time. Requested: as in Arc, it should slide out *over* the page with an animation, and the page should stay where it is.

**Decision.**
- While the sidebar is hidden, its view leaves the window's layout and lives in an **overlay** of the window (never a window). At rest the overlay is the 10 px edge strip and the card keeps a 10 px left margin for it. Hovering the strip slides the sidebar out over the page in 180 ms (ease-out); leaving slides it back after 300 ms, unless a menu, the address box, a rename or a drag still needs it. Hiding with Ctrl+S or the button also slides it away over the page, which takes the full width at once. Showing it again puts the view back into the layout at once (shortcuts like Ctrl+L focus it right away, which an overlay can't do before it is clicked, D-022). In fullscreen the view goes back into the layout, which hides it.
- **How it slides.** Burrow grows the overlay from the edge, one step per frame; the sidebar page keeps its contents at full width and slides them along with a CSS transform of the same duration and easing, starting as soon as the mouse enters or leaves. Moving the view itself was tried first: a view moving under a mouse that stands still makes Chromium report the mouse leaving, so the peek kept closing and reopening.
- **Keyboard.** Moving the live sidebar into an overlay takes the keyboard away while the window still believes the page has it, so focusing the page did nothing and shortcuts stopped. 50 ms later Burrow focuses the content panel (back in the window; it stays focusable) and then the page's browser directly. Focusing the page's view from the panel, or making the focused panel unfocusable, moves focus into the page as if by Tab and puts a focus ring on its first link. An overlay that can't be activated keeps the keyboard, but then nothing can be typed in the floating sidebar.
- Moving the sidebar resets the window's color (the margins around the card); it is painted again on the next task.
- No drag areas while the sidebar floats: they take the mouse away from the views (D-045), which would end a peek.
- Agents: no new tool. The `sidebar` tool already hides and shows the sidebar; peeking is a hover preview of the same sidebar.

**Verified** on Linux/Xvfb with real mouse and keys: the page doesn't move or resize while the sidebar peeks; frames of the slide show the contents moving with the edge, in and out; hovering keeps it open; typing in its address box works and keeps it open after the mouse leaves; clicking the page closes it; Ctrl+S hides and shows it again with the keyboard staying in a page's text field throughout; the toggle button while peeking brings it back; F11 in and out; no tabs open; the margins keep the space color. Not checked: resizing the window (Xvfb has no window manager), and anything on Windows.

## D-050 · A quieter top bar (2026-10-22)

**Context.** Reported on Windows: the top bar of D-045/D-047 looks ugly. It showed the raw URL (`https://…/`) in the default font right against the card's edge, with dark glyphs on a flat strip.

**Decision.** Only the look changes. The behavior is still D-047's: two overlays with the window showing through between them as the drag area, the hover and cursor rules, click to copy, and the page pushed down.
- The address is shown without the scheme, `www.` or a lone trailing slash, in a 12 px font (Segoe UI on Windows), in a soft gray (`INK_SOFT`) that turns dark when hovered. It starts 10 px in from the card's edge (`URL_PAD`, also counted in the drag-area math).
- The minimize, maximize and close glyphs are soft gray at rest and dark on hover, over a lighter hover shade (8 % instead of 12 %). Close still turns red.

**Verified** on Linux/Xvfb with `window show_bar`: the bar shows `example.com` in the smaller gray font with space before it, and the buttons are quieter. The Windows look is not yet checked by hand.

## D-051 · The site in the middle of the top bar, with site controls (2026-10-22)

**Context.** Even after D-050, the bar should look like Arc's: the site centered, with a link icon on its left and a sliders icon on its right (a screenshot of Arc's bar was given).

**Decision.**
- The open bar shows a centered group: a **link icon** (copies the link, like Ctrl+Shift+C), the **site** (the host without `www.`, or the whole address for pages without a host; clicking it opens the address box, like Ctrl+L), and a **sliders icon** (site controls). Both icons are drawn in code like the caption glyphs (14 px line icons), soft gray, dark on hover. The site's text only darkens on hover, with no box behind it.
- The window is dragged by the bar on **both sides** of the group (`top_bar_drag_rects`), where Burrow follows the cursor (Windows). Still two overlays, the site group and the window buttons, with the window showing through around them (D-047). On Linux the group's overlay spans the bar and centers its contents, and at rest the empty site button fills the strip so hovering it opens the bar.
- **Site controls** is a dialog over the window (D-039) for the shown page: zoom −, the level (click to reset to 100%), +; Copy Link; Boost This Site… (Edit Boost… when there is one); Send This Site to a Space…. Zoom goes through a new sidebar act `zoom` (`dir` 1, -1, or 0 to reset), the same as Ctrl+Plus/Minus/0, which also remembers the level for the site.
- Agents: no new tool. Everything in site controls already has one (`set_zoom`, `boost`, `air_traffic_control`), and copying the link is the agent reading the tab's URL.

**Verified** on Linux/Xvfb with real mouse moves: hovering the top opens the bar with link icon, site and sliders centered over the card; the sliders open the site controls; + twice showed 120%, and clicking the level reset it to 100%. Dragging by both sides of the group is Windows-only and not yet checked there.

## D-052 · Google's icons come from the sites, not a built-in table (2026-10-22)

**Problem (reported).** After D-049, Gmail's favorite showed Google's old 2020 "M". D-049 had built in a table of Google product icon URLs (`gstatic.com/images/branding/product/2x/<app>_2020q4_48dp.png`), and the newest icons at a fixed address were the 2020 set. Google's apps now name 2026 icons in their pages, e.g. Gmail `favicon_gmail_2026_v2.ico` and Voice `voice_favicon_2026_v2.png`. Any such table goes stale.

**Decision.**
- The built-in table is gone; icons come only from the sites. A loaded page's own icon is saved as before (D-025). The lookup for tabs that haven't loaded sends the profile's cookies (D-049), so signed-in apps answer with their real page and icon.
- D-049 skipped every lookup that ended on another site. That also skipped Google Chat, which moves from `mail.google.com/chat` to `chat.google.com`. Now only a lookup that ends on a sign-in page (`favicon::is_sign_in`: hosts like `accounts.`/`login.`, paths with `signin`/`login`/`oauth`) falls back to the asked site's `/favicon.ico`; other redirects are followed.
- Cache version 4: icons saved by D-049's builds are looked up once more.
- D-049's per-app keys (`SPLIT_HOSTS`) stay: they aren't icons, only which addresses share an icon.

## D-053 · The top bar opens on every monitor (2026-10-22)

**Problem (reported).** The top bar stopped opening on hover. The owner's Burrow is maximized on a second monitor, left of the main one (X=-1920, Y=7).

**Cause.** Burrow follows the cursor itself on Windows (D-047) and converted the cursor's screen point into window coordinates with `View::convert_point_from_screen`. For a window on that monitor the call left the point unchanged, (-958, 9) in and out, so the cursor never fell inside the card's range. On the main monitor the window's origin is about (0, 0), so the unconverted point happened to work, which is why D-047's checks passed.

**Decision.** Compare the cursor with the card's bounds in screen coordinates (`content.bounds_in_screen()`). The strip is the 8 px above the card's top, and the open bar the 32 px from there. Nothing else in Burrow uses `convert_point_from_screen`.

An 8 px "overhang" of maximized windows was suspected first and ruled out. `GetWindowRect` counts Windows' invisible resize border, and the window's visible top is the monitor's top.

**Verified on Windows** with a separate test copy, real cursor moves and screenshots: maximized on the left monitor (the owner's setup), the bar did not open before the fix and opens after it; maximized on the main monitor it opens before and after.

## D-054 · Tab icons are what the page shows, as in Arc (2026-10-22)

**Problem.** The owner asked for no special treatment: every tab should show the icon its page shows right now (a sign-in page's icon while on the sign-in page, Gmail's once the page is Gmail), changing as the page changes, as Arc does. Looking into why Burrow didn't already do that turned up a real bug: the icon a page reports was **never recorded**. `on_favicon_urlchange` read CEF's list of icon URLs with `list.clone().into_iter()`, and cef-rs's clone of a borrowed string list copies the opaque list handle by value, so the copy is always empty. Every tab's `favicon` stayed `None`, and the sidebar fell back to D-025's per-site cache for every tab. That cache is what D-049–D-052 kept patching for Google's apps. The same bug made the extensions "install from folder" picker return nothing.

**Decision.**
- `clients::strings` reads a list CEF lends a callback by wrapping the same pointer again instead of cloning it. The icon callback and the extensions folder picker use it.
- The sidebar shows, in order: the icon the tab's page reported (kept in `state.json`, so a tab that hasn't loaded since shows its page's last icon), then D-025's saved icon of its site or home site (for tabs that never loaded, e.g. after an Arc import), the site's `/favicon.ico`, a globe. When a tab moves to another site its recorded icon is cleared until the new page names its own.
- The special cases of D-049–D-052 are gone: no per-app icon keys for Google's hosts, no sign-in-page rule, no "home site's icon first" rule. D-049's cookies on the lookup for never-loaded tabs stay. That isn't a special case; it makes the lookup see the page the user would see.
- Cache version 5: saved site icons are looked up once more, and D-049's per-app files (`docs.google.com_spreadsheets.png`) are deleted.

**Verified** on Linux/Xvfb: a tab's `favicon` is recorded when its page loads, changes when the tab navigates to another site (GitHub → rust-lang.org's `favicon.svg`), shows in the sidebar, and is still there after a restart. A page that names no usable icon (example.org's `data:,`) shows the globe.
