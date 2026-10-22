#!/usr/bin/env bash
# Builds a release Burrow and installs it for the current Windows user (no admin needed):
#   %LOCALAPPDATA%\Programs\Burrow\burrow.exe  +  Start menu shortcut "Burrow".
# Your data stays in %LOCALAPPDATA%\Burrow. Uninstall: delete that Programs folder and the shortcut.
set -euo pipefail

repo="$(cd "$(dirname "$0")/.." && pwd)"
win_user="$(cmd.exe /c 'echo %USERNAME%' 2>/dev/null | tr -d '\r')"
dest="/mnt/c/Users/$win_user/AppData/Local/Programs/Burrow"

BURROW_WIN_DIR="$dest" "$repo/scripts/build-win.sh" --release

powershell.exe -NoProfile -Command "
  \$lnk = Join-Path ([Environment]::GetFolderPath('Programs')) 'Burrow.lnk';
  \$s = (New-Object -ComObject WScript.Shell).CreateShortcut(\$lnk);
  \$s.TargetPath = \"\$env:LOCALAPPDATA\\Programs\\Burrow\\burrow.exe\";
  \$s.WorkingDirectory = \"\$env:LOCALAPPDATA\\Programs\\Burrow\";
  \$s.Description = 'Burrow browser';
  \$s.Save();
  \"Start menu shortcut: \$lnk\""
echo "Installed to $dest"
