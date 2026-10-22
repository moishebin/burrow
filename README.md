# Burrow

An open-source browser in the spirit of Arc, built with Rust and Chromium (via CEF), with no Google services.

*Every tab has its place.*

> **⚠️ Work in progress.** Burrow is an early, personal project, shared as open source in the hope that it's useful.
> There is **no commitment** to respond to issues or review pull requests, though both are welcome.
> I'll try to maintain it for as long as I can.

- **Spaces in one window.** Each space has its own pinned tabs, folders and favorites, and can have its **own logins** (a separate browser profile). Switch spaces from the bottom of the sidebar or with Ctrl+1…9.
- **Pinned tabs that stay put.** A pinned tab keeps its home URL. Browse anywhere inside it, then click ↺ (or close it) and it goes back home, in the same spot.
- **Folders** for pinned tabs, nested as deep as you like. Drag and drop to organise.
- **Favorites**: the app tiles at the top, separate for each space.
- **Today**: unpinned tabs live below the line and are auto-archived after 12 hours of disuse (configurable). Ctrl+Shift+T brings them back.
- **Import from Arc**: spaces, folders, pinned tabs and favorites from Arc's local data (Windows).
- **One window, always.** Popups and "open in new window" become tabs or Peeks.
- **Peek**: links that would open a new tab, and links in pinned tabs that leave their site, open in a card over the page. Esc closes it; "Open as Tab" keeps it.
- **Air Traffic Control**: rules like "figma.com → Work" send links to the right space, including links from other apps when Burrow is your default browser.
- **Split View**: two tabs side by side (right-click a tab → Open in Split View). Drag the divider between them to resize.
- **History** (Ctrl+H), searched as you type in the address box.
- **Boosts**: your own CSS for a site. Zoom is remembered per site. Videos go fullscreen properly.
- **Chrome extensions** from the Chrome Web Store (password managers, dark mode, ad blockers, …): open an extension's page in the store and click **Add to Burrow** in the sidebar, then restart Burrow when it asks. The puzzle button at the top of the sidebar opens their popups; **Manage Extensions…** turns them on or off, removes or updates them, or installs one from a folder or `.crx` file.
- **Resizable sidebar**: drag its right edge; double-click the edge for the default width.
- **Picture in picture** for any video, like Arc: switch away from a tab playing a video and it keeps playing in a small floating window; come back and it returns to the page. Or right-click a tab → Picture in Picture, or Ctrl+Shift+P.
- **Updates itself** (Windows installer): new versions download in the background and install when you restart or quit. **+ → About and Updates…**
- **Clean window**: minimize, maximize and close hide in the top-right corner until you point there.

Status: early, but usable day to day on Windows. See [docs/FEATURES.md](docs/FEATURES.md) for what works and what doesn't yet, and [docs/DECISIONS.md](docs/DECISIONS.md) for the reasoning behind each choice.

## Download

Get the latest build from [Releases](https://github.com/moishebin/burrow/releases):

- **Windows (x64):** run `burrow-…-windows-x64-setup.exe`. It installs Burrow for your account (no admin rights needed), adds it to the Start menu, and lists it in *Settings → Apps* for uninstalling. Running a newer setup updates Burrow and keeps your spaces, tabs and logins. It isn't signed yet, so Windows SmartScreen may warn the first time ("More info → Run anyway").
  Prefer no installer? Unzip `burrow-…-windows-x64.zip` anywhere and run `Burrow\burrow.exe`; keep the folder together.
- **Linux (x64):** `tar xzf burrow-…-linux-x64.tar.gz` and run `Burrow/burrow`. It needs the usual desktop libraries Chrome needs (GTK 3, NSS). Keep the folder together.
- **macOS:** not built yet.

Your spaces, tabs, history and logins are saved in `%LOCALAPPDATA%\Burrow` (Windows) or `~/.local/share/Burrow` (Linux), separate from the app, so updating never touches them. Uninstalling asks whether to delete them too.
`SHA256SUMS.txt` on the release page lists each download's checksum.

## Building

Burrow is developed in WSL/Linux and cross-compiled for Windows.

```sh
rustup target add x86_64-pc-windows-msvc
cargo install cargo-xwin
# clang, lld and llvm-lib on PATH; scripts/build-win.sh looks in ~/.local/llvm/bin
scripts/build-win.sh            # builds and copies to %LOCALAPPDATA%\Burrow-dev\app
scripts/run-win.sh              # starts it with a throwaway profile and DevTools on :9222
scripts/run-win.sh --real       # starts it with your real profile (%LOCALAPPDATA%\Burrow)
scripts/package.sh              # release archives for Linux and Windows, plus the Windows setup .exe, in dist/
```

The Windows installer is built with NSIS (`installer/burrow.nsi`). `package.sh` expects it unpacked in `~/.local/nsis`:
`apt-get download nsis nsis-common`, then `dpkg-deb -x` each into `~/.local/nsis` (no root needed).

The first build downloads CEF (~400 MB) to `~/.local/share/cef`.

Tests for the model (no browser needed):

```sh
cargo test -p burrow-core
```

## Layout

| Path | What |
|---|---|
| `crates/burrow-core` | Model and logic: spaces, folders, tabs, commands, persistence, Arc import. Pure Rust, unit-tested. |
| `crates/burrow` | The app: CEF setup, window, tabs, profiles, handlers. |
| `ui/` | The sidebar (HTML/CSS/JS, no build step). |
| `scripts/` | Build, run and DevTools helpers. |
| `docs/` | Decisions and the feature tracker. |

## Agents

AI agents can drive Burrow: open, close, move and pin tabs, set up spaces and folders, and read, screenshot, click and type in **any** tab, even one you aren't looking at. They can also set Boosts, search history, set up split views, Air Traffic Control rules and Peek settings, zoom, mute and duplicate tabs, resize or hide the sidebar, minimize, maximize or fullscreen the window, float videos in picture in picture, and install, update, turn on or off and open Chrome extensions. They work in the background, so your own tab stays put. A pulsing **Agent** badge in the sidebar shows when one is working, and **+ → Block Agents from Pages** stops agents from reading or using pages (they can still manage tabs and spaces).

Add Burrow as an MCP server to your agent. For Claude Code:

```sh
claude mcp add burrow -- "C:\Users\<you>\AppData\Local\Programs\Burrow\burrow.exe" --mcp
# from WSL:
claude mcp add burrow -- /mnt/c/Users/<you>/AppData/Local/Programs/Burrow/burrow.exe --mcp
# (from the zip: use the burrow.exe in the folder you unzipped)
```

Or call tools from a shell (Burrow starts if it isn't running):

```sh
burrow --agent tools                                   # list the tools and their arguments
burrow --agent open_tab '{"url":"example.com"}'        # opens in the background, returns the tab
burrow --agent read_page '{"tab_id":"…","format":"elements"}'
burrow --agent screenshot '{"tab_id":"…"}' --out page.png
```

Tools: `list_spaces`, `get_space`, `list_tabs`, `open_tab`, `close_tab`, `focus_tab`, `navigate`, `move_tab`, `rename_tab`, `create_folder`, `update_folder`, `create_space`, `update_space`, `delete_space`, `switch_space`, `air_traffic_control`, `split_view`, `duplicate_tab`, `link_settings`, `boost`, `search_history`, `delete_history`, `peek`, `set_zoom`, `mute_tab`, `sidebar`, `window`, `picture_in_picture`, `update`, `extensions`, `read_page`, `screenshot`, `click`, `type_text`, `press_key`, `wait_for`, `eval_js`.

```sh
burrow --agent sidebar '{"width":320}'
burrow --agent extensions '{"action":"install","web_store":"https://chromewebstore.google.com/detail/…/<id>"}'
burrow --agent extensions '{"action":"restart"}'      # extensions load at startup
```
Under the hood it's a local HTTP API (`POST http://127.0.0.1:<port>/v1/<tool>`, port and token in `agent.json` in Burrow's data folder). See [D-018](docs/DECISIONS.md).

## Keyboard

| Keys | Action |
|---|---|
| Ctrl+T | New tab (also searches your open tabs and history) |
| Ctrl+L | Edit address |
| Ctrl+W | Close tab (pinned: unload) |
| Ctrl+D | Pin / unpin |
| Ctrl+1…9 | Switch space |
| Ctrl+Tab | Next tab |
| Ctrl+S | Hide / show sidebar |
| Ctrl+F | Find in page |
| Ctrl+Shift+T | Reopen closed tab |
| Alt+← / → | Back / forward |
| Ctrl+H | History |
| Ctrl+Shift+C | Copy the page's address |
| Ctrl+P | Print |
| Ctrl+= / − / 0 | Zoom (remembered per site) |
| F11 | Full screen |
| Ctrl+Shift+P | Picture in picture |
| Esc | Close a Peek, leave a fullscreen video |
| F12 | DevTools |

## License

[MPL-2.0](LICENSE). Burrow is not affiliated with The Browser Company; "Arc" is their trademark.
