#!/usr/bin/env bash
# Restarts the Windows build of Burrow with DevTools on port 9222 (dev only).
#   scripts/run-win.sh            uses a throwaway data dir (…\Burrow-dev\data)
#   scripts/run-win.sh --real     uses the normal data dir (%LOCALAPPDATA%\Burrow)
set -euo pipefail
win_user="$(cmd.exe /c 'echo %USERNAME%' 2>/dev/null | tr -d '\r')"
app="C:\\Users\\$win_user\\AppData\\Local\\Burrow-dev\\app"
data_env="\$env:BURROW_DATA_DIR='C:\\Users\\$win_user\\AppData\\Local\\Burrow-dev\\data';"
[[ "${1:-}" == "--real" ]] && data_env="Remove-Item Env:BURROW_DATA_DIR -ErrorAction SilentlyContinue;"
powershell.exe -NoProfile -Command "
  Get-Process burrow -ErrorAction SilentlyContinue | Stop-Process -Force; Start-Sleep -Milliseconds 800;
  $data_env
  Start-Process -FilePath '$app\\burrow.exe' -ArgumentList '--remote-debugging-port=9222' -WorkingDirectory '$app'"
echo "Started $app\\burrow.exe"
