# Features: needed vs implemented

Legend: ✅ done · 🟡 partial · ⬜ not started · 💤 deferred
Priority: **P0** = needed before the owner can switch from Arc · P1 = soon after · P2 = later
"Verified" means checked in the running Windows build via DevTools (D-012), not only unit-tested.

## P0: must have to replace Arc daily

| Feature | Status | Notes |
|---|---|---|
| Chromium engine without Google services (CEF 154) | ✅ | D-001, D-006. Verified. |
| Builds for Windows from WSL | ✅ | `scripts/build-win.sh` (D-002) |
| Single window; popups and "new window" become tabs | ✅ | Verified: `target=_blank` → tab; `window.open` → tab with `opener` kept (OAuth works) |
| Left sidebar (Arc layout) | ✅ | Verified with screenshots |
| **Spaces**: several in one window, switcher, Ctrl+1…9 | ✅ | Verified |
| **Separate logins per space** (profiles) | ✅ | Verified: a cookie set in one space isn't visible in another |
| **Favorites** grid per space | ✅ | |
| **Pinned tabs with a fixed home** (↺ back home, close = unload) | ✅ | Unit-tested and verified |
| **Folders** (nested, collapse, rename, delete keeps tabs) | ✅ | Unit-tested |
| Today / unpinned tabs | ✅ | |
| New tab via address box (Ctrl+T), URL-or-search | ✅ | Verified |
| Back / forward / reload / stop, address display | ✅ | |
| Close tab (Ctrl+W); pinned → unload | ✅ | |
| Persist everything across restarts | ✅ | `state.json`, atomic, debounced. Verified after a graceful close. |
| Graceful window close (all browsers closed first) | ✅ | Verified |
| Drag and drop: reorder, into folders, to favorites/today, onto another space | ✅ | Verified with a real mouse on Xvfb (D-015): self-drop, empty folder, favorites, above folder, folder onto own child, onto a space dot |
| Import from Arc (spaces, folders, pinned, favorites) | ✅ | Verified on a real Arc profile |
| Downloads (to ~/Downloads, list in sidebar, click to open) | ✅ | Verified |
| Look and feel: space-tinted sidebar, content card, frameless window | ✅ | D-013. Dragging and resizing the frameless window not yet checked by hand. |
| Camera / mic / notification permission prompts | ✅ | D-014. Verified the allow-and-remember flow. |
| Page context menu: open link in new tab, copy link, search | ✅ | D-014 |
| App icon (window/taskbar) | ✅ | `scripts/make-icon.py` |

## P1

| Feature | Status | Notes |
|---|---|---|
| Search open tabs from the address box | ✅ | ↑↓ + Enter picks a tab, in any space |
| Find in page (Ctrl+F) | ✅ | D-014 |
| Auto-archive Today tabs (default 12h) + Recently Closed + Ctrl+Shift+T | ✅ | Unit-tested |
| Keyboard shortcuts (see README) | ✅ | Verified with real keys on Xvfb: Ctrl+T/W/D/F, Ctrl+Shift+T, Esc |
| Sidebar collapse (Ctrl+S) + hover the left edge to peek | ✅ | Verified with real input on Xvfb. A hidden sidebar used to leave no way back (reported by the owner, D-016). |
| Space colors, icons, rename, delete | ✅ | |
| Search engine setting | ✅ | Default Google (D-011) |
| Favicon cache on disk (show icons for never-loaded tabs offline) | 🟡 | Falls back to `<origin>/favicon.ico` |
| Command bar with history search | ⬜ | Open tabs only so far |
| Peek / little browser for links from pinned tabs | ⬜ | |
| Split view | ⬜ | |
| Password saving / autofill | ⬜ | Alloy has no password UI (D-010). Likely a built-in manager or an extension later. |
| Default-browser registration on Windows | ⬜ | The installer exists now (D-030); it doesn't register Burrow as a browser yet |
| Installer + auto-update | 🟡 | Windows setup .exe: per-user, Start menu, *Settings → Apps* uninstall, updates in place (D-030). Verified install/upgrade/uninstall on Windows. Zip and Linux archive too (D-020); the zip runs on a clean PC (D-029). Not code-signed yet (ready via `BURROW_SIGN`); no auto-update yet. |
| Launching Burrow again focuses the running window | ✅ | D-029. Verified on Windows: restores a minimized window, no second window. |
| Exe file icon (Explorer) | ✅ | `build.rs` + embed-resource (llvm-rc) |
| Zoom indicator, per-site zoom memory | 🟡 | Ctrl+=/−/0 work; no indicator |
| History page | ⬜ | |
| Notifications shown as Windows toasts | ⬜ | Permission prompt exists; Alloy may not deliver toasts |
| **Agent control** (MCP server, CLI, local API): tabs, spaces, folders, read/screenshot/click/type in any tab | ✅ | D-018. Verified on Linux/Xvfb: background tabs read, screenshotted, clicked and typed into while the user's tab stays shown; MCP over stdio; auth/Origin/Host checks. Page tools can be switched off; tab and space tools always work (D-019). Windows exe verified as a client from WSL; the server side on Windows is not yet checked by hand. |

## P2

| Feature | Status | Notes |
|---|---|---|
| Chrome extensions (password managers first) | 💤 | Owner said "not now" (2026-10-22). Alloy has limited extension support (D-010). |
| Built-in ad blocker | 💤 | |
| macOS / Linux packages | 🟡 | Linux `.tar.gz` since 0.0.1 (D-020). macOS needs a Mac to build (app bundle, signing). |
| Sync between machines | 💤 | |
| Boosts / custom CSS | 💤 | |
