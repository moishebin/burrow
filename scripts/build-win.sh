#!/usr/bin/env bash
# Cross-compiles Burrow for Windows from WSL/Linux and copies it to a Windows folder (D-002).
#
#   scripts/build-win.sh            debug build
#   scripts/build-win.sh --release  release build
#
# Needs: rustup target x86_64-pc-windows-msvc, cargo-xwin, LLVM in ~/.local/llvm, cmake, ninja.
# Output: $BURROW_WIN_DIR (default C:\Users\<you>\AppData\Local\Burrow-dev\app), run burrow.exe there.
set -euo pipefail

repo="$(cd "$(dirname "$0")/.." && pwd)"
export PATH="$HOME/.cargo/bin:$HOME/.local/llvm/bin:$PATH"
# Share one CEF download between Linux and Windows builds instead of one per target dir.
export CEF_PATH="${CEF_PATH:-$HOME/.local/share/cef}"

profile=debug
args=()
if [[ "${1:-}" == "--release" ]]; then
  profile=release
  args+=(--release)
fi

cargo xwin build --manifest-path "$repo/Cargo.toml" -p burrow --target x86_64-pc-windows-msvc "${args[@]}"

out="$repo/target/x86_64-pc-windows-msvc/$profile"
win_user="$(cmd.exe /c 'echo %USERNAME%' 2>/dev/null | tr -d '\r')"
dest="${BURROW_WIN_DIR:-/mnt/c/Users/$win_user/AppData/Local/Burrow-dev/app}"
mkdir -p "$dest/ui"

# A running Burrow locks its exe and DLLs.
powershell.exe -NoProfile -Command "Get-Process burrow -ErrorAction SilentlyContinue | Stop-Process -Force; Start-Sleep -Milliseconds 500" || true

# CEF runtime files sit next to the exe (copied there by cef-dll-sys).
rsync -a --delete-after \
  --include='*.dll' --include='*.pak' --include='*.dat' --include='*.bin' --include='*.json' \
  --include='locales/***' --include='burrow.exe' --include='burrow.pdb' --exclude='*' \
  "$out/" "$dest/"
rsync -a --delete "$repo/ui/" "$dest/ui/"
echo "Built $profile → $dest/burrow.exe"
