# Features: needed vs implemented

Legend: ✅ done · 🟡 partial · ⬜ not started · 💤 deferred
Priority: **P0** = needed to switch from Arc · P1 = soon after · P2 = later
"Verified" means checked in the running Windows build via DevTools (D-012), not only unit-tested.

## P0: must have to replace Arc daily

| Feature | Status | Notes |
|---|---|---|
| Chromium engine without Google services (CEF 154) | ✅ | D-001, D-006. Verified. |
| Builds for Windows from WSL | ✅ | `scripts/build-win.sh` (D-002) |
| Single window; popups and "new window" become tabs | ✅ | Verified: `target=_blank` → Peek (or tab); `window.open` → Peek with `opener` kept (OAuth works) |
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
| Window buttons hidden in the top-right corner until hovered (Arc for Windows) | ✅ | D-035. Verified on Xvfb with real mouse moves. Agent tool `window`. |
| Dialogs (History, Extensions, Air Traffic Control, settings…) centered over the window | ✅ | D-039. Were squeezed into the sidebar. Verified on Xvfb with real input. |
| Picture in picture for every video; auto when leaving a playing tab (Arc) | ✅ | D-036. Chromium's own floating window. Verified on Xvfb: auto on tab switch, back on return, "back to tab", close, Ctrl+Shift+P; the floating window also opens on Windows (D-043). Videos inside iframes not yet. Agent tool `picture_in_picture`. |
| Camera / mic / notification permission prompts | ✅ | D-014. Verified the allow-and-remember flow. |
| Page context menu: open link in new tab, copy link, search | ✅ | D-014 |
| App icon (window/taskbar) | ✅ | `scripts/make-icon.py` |
| Site icons for every tab, also never-loaded ones | ✅ | D-025. Saved on disk; found from the page's `<link rel=icon>`. Verified on Xvfb (fixed a reported globe-icon bug). |

## P1

| Feature | Status | Notes |
|---|---|---|
| Search open tabs from the address box | ✅ | ↑↓ + Enter picks a tab, in any space |
| **Air Traffic Control** (rules send links to a space) | ✅ | D-021. Verified on Xvfb: a matching link opened in the rule's space. Also an agent tool. |
| **Peek** (links open in a card over the page) | ✅ | D-022. Verified on Xvfb with real clicks; not yet on Windows. |
| History + history search in the address box | ✅ | D-023. Ctrl+H. Verified on Xvfb. |
| Split View | ✅ | D-026. Verified on Xvfb. |
| Fullscreen videos (and F11) | ✅ | D-026. Verified on Xvfb, Esc leaves. |
| Boosts (custom CSS per site) | ✅ | D-026. Verified on Xvfb. |
| Mute tab, duplicate tab, copy address, print | ✅ | D-026 |
| Pages that call `window.close()` close | ✅ | D-027 (was broken) |
| Find in page (Ctrl+F) | ✅ | D-014 |
| Auto-archive Today tabs (default 12h) + Recently Closed + Ctrl+Shift+T | ✅ | Unit-tested |
| Sleeping tabs: unload tabs out of sight (default 10 min, per-tab times, Sleep Now) | ✅ | D-040. Tabs playing media (also in embedded players from other sites), in picture in picture, in a call, or with typed text stay awake. Unit-tested; verified on Linux/Xvfb |
| Keyboard shortcuts (see README) | ✅ | Verified with real keys on Xvfb: Ctrl+T/W/D/F, Ctrl+Shift+T, Esc |
| Sidebar collapse (Ctrl+S) + hover the left edge to peek | ✅ | Verified with real input on Xvfb. A hidden sidebar used to leave no way back (reported in use, D-016). |
| Resizable sidebar (drag the right edge) | ✅ | D-033. 200–600 px, saved; double-click resets. Verified with a real drag on Xvfb. Agent tool `sidebar`. |
| Space colors, icons, rename, delete | ✅ | |
| Search engine setting | ✅ | Default Google (D-011) |
| Favicon cache on disk (show icons for never-loaded tabs offline) | ✅ | D-025 |
| Command bar with history search | ✅ | Address box: open tabs, then history (D-023) |
| Peek / little browser for links from pinned tabs | ✅ | D-022 |
| Split view | ✅ | Two tabs (D-026). Drag the divider to resize, double-click for halves (D-038). |
| Password saving / autofill | 🟡 | Alloy has no built-in password UI (D-010). A password manager extension works (Bitwarden's popup verified, D-032); no built-in manager. |
| Default-browser registration on Windows | 🟡 | D-024. The `reg add` quoting checked on Windows (D-043). + → Make Burrow Your Default Browser… writes per-user registry entries and opens Settings. Links hand over to a running Burrow (verified on Linux, and on Windows with the 0.0.2 installer, D-031). The registry part isn't checked by hand yet. |
| Installer + auto-update | 🟡 | Windows setup .exe: per-user, Start menu, *Settings → Apps* uninstall, updates in place (D-030). Verified install/upgrade/uninstall on Windows. Zip and Linux archive too (D-020); the zip runs on a clean PC (D-029). Not code-signed yet (ready via `BURROW_SIGN`). Auto-update (D-037): verified end to end on Windows (D-043). |
| Launching Burrow again focuses the running window | ✅ | D-029. Verified on Windows: restores a minimized window, no second window. |
| Exe file icon (Explorer) | ✅ | `build.rs` + embed-resource (llvm-rc) |
| Zoom indicator, per-site zoom memory | ✅ | D-026 |
| History page | ✅ | A History list in the sidebar (Ctrl+H), D-023 |
| Notifications shown as Windows toasts | ✅ | D-042. Chromium dropped them in Alloy tabs; Burrow catches them in the page and shows its own toasts. Verified on Windows (the toast reached Windows); clicking a toast verified on Linux only. Agent tool `notifications`. |
| **Agent control** (MCP server, CLI, local API): tabs, spaces, folders, read/screenshot/click/type in any tab | ✅ | D-018. Verified on Linux/Xvfb: background tabs read, screenshotted, clicked and typed into while the user's tab stays shown; MCP over stdio; auth/Origin/Host checks. Page tools can be switched off; tab and space tools always work (D-019). Boosts, history, split view, Peek, zoom, mute, duplicate, Air Traffic Control and link settings too (D-028). Windows exe verified as a client from WSL; the server side on Windows is not yet checked by hand. |

## P2

| Feature | Status | Notes |
|---|---|---|
| Chrome extensions | 🟡 | D-032. Install from the Chrome Web Store ("Add to Burrow" on a store page, or a link), a folder or a `.crx`; puzzle button opens popups; on/off, remove, update; changes need a restart (offered). Verified on Xvfb (Bitwarden, Dark Reader, uBlock Origin Lite) and on Windows. Pin an extension to the toolbar to see its badge (D-041); backgrounds can open tabs and find the active tab (D-041). Missing: extension context menus and shortcuts, auto-update. Agent tool `extensions`. |
| Built-in ad blocker | 💤 | |
| macOS / Linux packages | 🟡 | Linux `.tar.gz` since 0.0.1 (D-020). macOS needs a Mac to build (app bundle, signing). |
| Sync between machines | 💤 | |
| Boosts / custom CSS | ✅ | CSS only (D-026); no per-site JavaScript |

## Roadmap: planned next

Not started unless marked. Roughly in order within each group. Anything an agent can't do yet gets its tool in the same change (D-028).

### Next up

| Feature | Notes |
|---|---|
| Check by hand on Windows | Frameless window drag/resize, default-browser registry, typing in dialogs, clicking a toast, Peek, the agent server, hovering the window buttons. |
| Code-signed Windows builds | Stops SmartScreen warnings on install and auto-update. `BURROW_SIGN` is ready; needs a certificate. |
| Media controls in the sidebar | Arc's mini player: play/pause, next, mute for the tab playing sound, shown even when you're in another tab or space. |
| Clear browsing data, site settings | Per space: cookies and site data, cache, history; see and revoke camera/mic/notification permissions per site. |
| Built-in password saving | Save and fill logins per space, without an extension. Today only password-manager extensions (D-032). |
| Extension gaps | Context-menu items, keyboard shortcuts, auto-update of installed extensions, badges per space. |
| Picture in picture for videos in iframes | Embedded players (e.g. YouTube embeds on other sites). |
| Spellcheck suggestions | In the page context menu, with a language setting. |

### Arc features still missing

| Feature | Notes |
|---|---|
| Links from other apps in a card | Arc's Little Arc, but over the current window (no second window): a link from another app opens in a Peek-style card; keep it or send it to a space. |
| Capture | Select part of a page, copy or save it as an image. |
| Tab hover previews | A preview of a tab's page when hovering it in the sidebar. |
| Boosts: Zap and scripts | Click elements to hide them for good; optional per-site JavaScript (D-026 left it out). |
| Live folders | Folders that fill themselves, e.g. from an RSS feed or GitHub pull requests. |
| Notes | Arc-style notes as pinned items in a space. |
| Themes | Gradient space colors, light/dark per space, a grain option. |
| AI helpers (optional, off by default) | Tidy Today tabs into folders, rename tabs to short titles, summarize a page or a link before opening it. Bring-your-own API key or a local model. |

### Later

| Feature | Notes |
|---|---|
| Reader mode | Clean text view of articles. |
| Translate pages | No Google services (D-006), so needs another translation engine. |
| Custom keyboard shortcuts | Rebind any shortcut. |
| Sidebar on the right | Option. |
| Built-in ad blocker | Was 💤; extensions (uBlock Origin Lite) cover it for now. |
| Sync between machines | Spaces, pinned tabs, favorites, Boosts; was 💤. |
| macOS build | Needs a Mac for the app bundle and signing. |
