# Burrow: notes for working in this repo

Open-source Arc-style browser: Rust + CEF (cef-rs), Windows first, developed in WSL.

## Rules

- Record every non-obvious decision in `docs/DECISIONS.md` (append a new D-NNN; never rewrite history) and keep `docs/FEATURES.md` statuses current. The owner relies on both.
- The owner's must-haves: pinned tabs and folders with a fixed place, spaces with separate logins in **one window**, and per-space favorites. Never add anything that opens a second window.
- Model logic goes in `crates/burrow-core` with unit tests. The app crate only translates CEF events into commands and effects.
- CEF callbacks must not touch the controller directly: use `browser::later(...)` (D-009). The only synchronous `browser::with` calls are ones where CEF needs an answer now (`can_close`, `is_frameless`, popups, permissions).
- Text from web pages goes into the sidebar with `textContent`, never `innerHTML`.
- Agent tools (D-018): model-only tools go in `burrow_core::agent`; page tools in `crates/burrow/src/browser/tools.rs`. Agents must not disturb the user: open in the background unless asked to focus.

## Dev loop (D-012)

```sh
cargo test -p burrow-core                       # model tests
scripts/build-win.sh                            # cross-compile + copy to %LOCALAPPDATA%\Burrow-dev\app
scripts/run-win.sh                              # restart with a throwaway profile, DevTools on :9222
powershell.exe -File 'C:\Users\<user>\AppData\Local\Burrow-dev\tools\cdp.ps1' -List
powershell.exe -File '…\cdp.ps1' -Match sidebar -Shot C:\…\sb.png -Eval "JSON.stringify(view.state)"
```

`build-win.sh` and `run-win.sh` kill every running `burrow.exe`, including the owner's installed one; check `Get-Process burrow` first. `build-win.sh` doesn't copy `scripts/cdp.ps1`/`postkey.ps1`; copy them to `…\Burrow-dev\tools\` when they change. The DevTools port is only reachable from Windows, so use `powershell.exe`, not curl from WSL.

Real mouse/keyboard testing (Linux build on a virtual display, D-015):

```sh
CEF_PATH=~/.local/share/cef cargo build -p burrow   # Linux build
scripts/run-xvfb.sh                                 # Burrow on DISPLAY=:99, DevTools on :9222
DISPLAY=:99 xdotool mousemove 130 69 click 1        # real input
python3 scripts/xshot.py /tmp/shot.png              # screenshot of :99
node scripts/cdp.mjs sidebar eval "view.state.spaces.length"
target/debug/burrow --agent list_tabs --data-dir target/xvfb-data   # agent tools (D-018)
scripts/run-xvfb.sh stop
```

Don't `pkill -f` a pattern that also appears in your own command line: it kills your shell.

## CEF gotchas (D-010)

- Use the Alloy runtime style for the window and every BrowserView; a window holds only one Chrome-style BrowserView.
- Never pass `--disable-features` (it crashes browser creation).
- Unimplemented int-returning callbacks may read as "no": implement `can_close`, `can_download`, etc. explicitly.
- `do_close` returns 1 except during window shutdown; with 0, closing one tab closes the whole window.
- Request contexts initialize asynchronously; tabs wait in `Profile::pending`.
