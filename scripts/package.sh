#!/usr/bin/env bash
# Builds release packages for Linux and Windows into dist/ (from WSL/Linux, D-002).
#
#   scripts/package.sh            both
#   scripts/package.sh linux      one of them
#   scripts/package.sh windows
#
# Each archive holds a Burrow/ folder: the executable, the CEF runtime, ui/, and the licenses.
# Windows also gets a setup .exe (per-user installer). macOS isn't built here: it needs a Mac.
#
# Code signing (D-030): set BURROW_SIGN to a command that signs the file given as its argument
# in place, e.g. BURROW_SIGN=scripts/sign.sh. It signs burrow.exe, the setup and the uninstaller.
set -euo pipefail

repo="$(cd "$(dirname "$0")/.." && pwd)"
export PATH="$HOME/.cargo/bin:$HOME/.local/llvm/bin:$PATH"
export CEF_PATH="${CEF_PATH:-$HOME/.local/share/cef}"
version="$(grep -m1 '^version' "$repo/Cargo.toml" | cut -d'"' -f2)"
dist="$repo/dist"
targets=("$@")
[[ $# -eq 0 ]] && targets=(linux windows)
mkdir -p "$dist"

# Runtime files cef-dll-sys copies next to the executable, minus build-only leftovers.
runtime=(--include='*.pak' --include='*.dat' --include='*.bin' --include='vk_swiftshader_icd.json'
  --include='locales/***' --include='CREDITS.html')

stage() { # <build dir> <exe name> <extra rsync includes…>
  local out="$1" exe="$2"; shift 2
  local dir="$dist/stage/Burrow"
  rm -rf "$dist/stage" && mkdir -p "$dir"
  rsync -a "${runtime[@]}" "$@" --include="$exe" --exclude='*' "$out/" "$dir/"
  rsync -a "$repo/ui/" "$dir/ui/"
  cp "$repo/LICENSE" "$dir/LICENSE.txt"
  cp "$repo/README.md" "$dir/README.md"
  mv "$dir/CREDITS.html" "$dir/CEF-CHROMIUM-CREDITS.html"
}

# The Windows setup .exe, from the staged folder (installer/burrow.nsi, D-030). NSIS is unpacked
# into ~/.local/nsis (`apt-get download nsis nsis-common` + `dpkg-deb -x`; no root needed).
installer() {
  local nsis="$HOME/.local/nsis/usr"
  # The uninstaller deletes exactly what was installed: one Delete per file, then the folders.
  (cd "$dist/stage/Burrow" &&
    find . -type f | sort | sed 's|^\./||; s|/|\\|g; s|.*|  Delete "$INSTDIR\\&"|' &&
    find . -mindepth 1 -type d | sort -r | sed 's|^\./||; s|/|\\|g; s|.*|  RMDir "$INSTDIR\\&"|'
  ) > "$dist/stage/uninstall-list.nsh"
  local sign=()
  [[ -n "${BURROW_SIGN:-}" ]] && sign=(-DSIGN="$BURROW_SIGN")
  NSISDIR="$nsis/share/nsis" "$nsis/bin/makensis" -V2 \
    -DVERSION="$version" -DSRCDIR="$dist/stage/Burrow" -DUNLIST="$dist/stage/uninstall-list.nsh" \
    -DICON="$repo/crates/burrow/resources/burrow.ico" \
    -DOUTFILE="$dist/burrow-$version-windows-x64-setup.exe" "${sign[@]}" "$repo/installer/burrow.nsi"
}

for t in "${targets[@]}"; do
  case "$t" in
    linux)
      cargo build --manifest-path "$repo/Cargo.toml" -p burrow --release
      stage "$repo/target/release" burrow --include='*.so' --include='*.so.*'
      # CEF ships libcef.so with debug symbols (~1.4 GB); users don't need them.
      strip "$dist/stage/Burrow/burrow" "$dist/stage/Burrow/libcef.so"
      name="burrow-$version-linux-x64.tar.gz"
      tar -C "$dist/stage" -czf "$dist/$name" Burrow
      ;;
    windows)
      cargo xwin build --manifest-path "$repo/Cargo.toml" -p burrow --target x86_64-pc-windows-msvc --release
      stage "$repo/target/x86_64-pc-windows-msvc/release" burrow.exe --include='*.dll'
      [[ -n "${BURROW_SIGN:-}" ]] && $BURROW_SIGN "$dist/stage/Burrow/burrow.exe"
      name="burrow-$version-windows-x64.zip"
      python3 -c 'import shutil, sys; shutil.make_archive(sys.argv[1], "zip", sys.argv[2], "Burrow")' \
        "$dist/${name%.zip}" "$dist/stage"
      echo "Packaged dist/$name ($(du -h "$dist/$name" | cut -f1))"
      installer
      name="burrow-$version-windows-x64-setup.exe"
      ;;
    *) echo "unknown target $t" >&2; exit 2 ;;
  esac
  echo "Packaged dist/$name ($(du -h "$dist/$name" | cut -f1))"
done
rm -rf "$dist/stage"
# Checksums for every archive of this version, for the release page.
(cd "$dist" && sha256sum burrow-"$version"-* > SHA256SUMS.txt)
echo "Checksums in dist/SHA256SUMS.txt"
