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
- **One window, always.** Popups and "open in new window" become tabs.

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

AI agents can drive Burrow: open, close, move and pin tabs, set up spaces and folders, and read, screenshot, click and type in **any** tab, even one you aren't looking at. They work in the background, so your own tab stays put. A pulsing **Agent** badge in the sidebar shows when one is working, and **+ → Block Agents from Pages** stops agents from reading or using pages (they can still manage tabs and spaces).

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

Tools: `list_spaces`, `get_space`, `list_tabs`, `open_tab`, `close_tab`, `focus_tab`, `navigate`, `move_tab`, `rename_tab`, `create_folder`, `update_folder`, `create_space`, `update_space`, `delete_space`, `switch_space`, `read_page`, `screenshot`, `click`, `type_text`, `press_key`, `wait_for`, `eval_js`.
Under the hood it's a local HTTP API (`POST http://127.0.0.1:<port>/v1/<tool>`, port and token in `agent.json` in Burrow's data folder). See [D-018](docs/DECISIONS.md).

## Keyboard

| Keys | Action |
|---|---|
| Ctrl+T | New tab (also searches your open tabs) |
| Ctrl+L | Edit address |
| Ctrl+W | Close tab (pinned: unload) |
| Ctrl+D | Pin / unpin |
| Ctrl+1…9 | Switch space |
| Ctrl+Tab | Next tab |
| Ctrl+S | Hide / show sidebar |
| Ctrl+F | Find in page |
| Ctrl+Shift+T | Reopen closed tab |
| Alt+← / → | Back / forward |
| F12 | DevTools |

## License

[MPL-2.0](LICENSE). Burrow is not affiliated with The Browser Company; "Arc" is their trademark.
