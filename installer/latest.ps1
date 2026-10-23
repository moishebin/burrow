# Run by the setup (D-056): asks GitHub for the latest Burrow release, and downloads its setup.
#
#   latest.ps1 -Mode check                          prints the latest version, e.g. 0.0.4
#   latest.ps1 -Mode download -Version 0.0.4 -Out x saves that version's setup to x, checked
#                                                   against the release's SHA256SUMS.txt
#
# Exit codes: 0 done, 1 no answer (offline, rate limit, …), 2 no such file in the release,
# 3 checksum mismatch (the file is deleted).
param([string]$Mode = 'check', [string]$Version, [string]$Out)
$ErrorActionPreference = 'Stop'
# Windows PowerShell 5 may default to TLS 1.0, which GitHub refuses.
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
# Invoke-WebRequest's progress display slows big downloads down a lot.
$ProgressPreference = 'SilentlyContinue'
$headers = @{ 'User-Agent' = 'Burrow-Setup' }
try {
  $release = Invoke-RestMethod -Uri 'https://api.github.com/repos/moishebin/burrow/releases/latest' -Headers $headers -TimeoutSec 10
  if ($Mode -eq 'check') {
    [Console]::Out.Write($release.tag_name.TrimStart('v'))
    exit 0
  }
  $name = "burrow-$Version-windows-x64-setup.exe"
  $asset = $release.assets | Where-Object { $_.name -eq $name }
  $sums = $release.assets | Where-Object { $_.name -eq 'SHA256SUMS.txt' }
  if (-not $asset -or -not $sums) { exit 2 }
  $text = (Invoke-WebRequest -Uri $sums.browser_download_url -Headers $headers -UseBasicParsing -TimeoutSec 30).Content
  if ($text -is [byte[]]) { $text = [Text.Encoding]::UTF8.GetString($text) }
  $line = $text -split "`n" | Where-Object { $_ -match [regex]::Escape($name) } | Select-Object -First 1
  if (-not $line) { exit 2 }
  $want = ($line -split '\s+')[0]
  Invoke-WebRequest -Uri $asset.browser_download_url -Headers $headers -UseBasicParsing -OutFile $Out
  $have = (Get-FileHash -Path $Out -Algorithm SHA256).Hash
  if ($have -ne $want.ToUpper()) {
    Remove-Item -Path $Out -Force
    exit 3
  }
  exit 0
} catch {
  exit 1
}
