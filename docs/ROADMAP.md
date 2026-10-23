# Burrow roadmap

Everything planned that Burrow does not have yet. What is already built is in [FEATURES.md](FEATURES.md); decisions in [DECISIONS.md](DECISIONS.md).

Not started unless marked. Roughly in order within each group. Anything an agent can't do yet gets its tool in the same change (D-028).

## Requested (2026-10-22)

| Feature | Notes |
|---|---|
| Fix wrong site icons | Bug: Gmail shows the Google "G", not its own icon. Likely the first icon a page offers is kept, while Gmail swaps its icon by script later; follow icon changes. |
| Tail command for agents | `burrow --tail` + an agent tool: a live stream of what happens (clicks, tabs opened/closed/loaded, sidebar actions, page errors, Burrow's log), so an agent can see a bug being reproduced. Optional screenshots per step. |
| Live folders | Folders that fill themselves: open GitHub pull requests of chosen repos (or awaiting review), issues, RSS. |
| Custom keyboard shortcuts | Rebind any shortcut in a settings dialog, with conflict warnings. |
| Where new tabs go | Top or bottom of Today, or next to the tab they came from. |
| Folder size | Compact or roomy folder rows, and how many items show before "show more" (meaning to confirm). |
| Import from any browser | Onboarding from Chrome, Edge, Brave, Firefox, Opera, Vivaldi, not only Arc: bookmarks, open tabs, history, passwords where possible. First-run screen. |

## Next up

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

## Arc features still missing

| Feature | Notes |
|---|---|
| Links from other apps in a card | Arc's Little Arc, but over the current window (no second window): a link from another app opens in a Peek-style card; keep it or send it to a space. |
| Capture | Select part of a page, copy or save it as an image. |
| Tab hover previews | A preview of a tab's page when hovering it in the sidebar. |
| Boosts: Zap and scripts | Click elements to hide them for good; optional per-site JavaScript (D-026 left it out). |
| Notes | Arc-style notes as pinned items in a space. |
| Themes | Gradient space colors, light/dark per space, a grain option. |
| AI helpers (optional, off by default) | Tidy Today tabs into folders, rename tabs to short titles, summarize a page or a link before opening it. Bring-your-own API key or a local model. |

## Later

| Feature | Notes |
|---|---|
| Reader mode | Clean text view of articles. |
| Translate pages | No Google services (D-006), so needs another translation engine. |
| Sidebar on the right | Option. |
| Built-in ad blocker | Was 💤; extensions (uBlock Origin Lite) cover it for now. |
| Sync between machines | Spaces, pinned tabs, favorites, Boosts; was 💤. |
| macOS build | Needs a Mac for the app bundle and signing. |
