# Tiny Chrome DevTools Protocol client for checking a running Burrow from WSL (dev only).
# Burrow must be started with --remote-debugging-port=9222.
#
#   cdp.ps1 -List
#   cdp.ps1 -Match sidebar -Shot C:\path\out.png        screenshot of the page whose URL contains "sidebar"
#   cdp.ps1 -Match sidebar -Eval "document.title"       evaluate JS, print the result as JSON
#   cdp.ps1 -Match example -Key T -Modifiers 2          press Ctrl+T in that page (modifiers: 1 Alt, 2 Ctrl, 4 Meta, 8 Shift)
param(
    [switch]$List,
    [string]$Match = "",
    [string]$Shot = "",
    [string]$Eval = "",
    [int]$Port = 9222,
    [int]$Index = 0,
    [string]$Key = "",
    [int]$Modifiers = 0
)
$ErrorActionPreference = "Stop"
$targets = (Invoke-WebRequest -UseBasicParsing "http://127.0.0.1:$Port/json").Content | ConvertFrom-Json
if ($List) { $targets | ForEach-Object { "{0}`t{1}`t{2}`t{3}" -f $_.id, $_.type, $_.title, $_.url }; exit 0 }
$t = @($targets | Where-Object { $_.type -eq "page" -and $_.url -like "*$Match*" })[$Index]
if (-not $t) { Write-Error "no page matching '$Match'" }

$ws = New-Object System.Net.WebSockets.ClientWebSocket
$ct = [Threading.CancellationToken]::None
$ws.ConnectAsync([Uri]$t.webSocketDebuggerUrl, $ct).Wait()

function Call($id, $method, $params) {
    $msg = @{ id = $id; method = $method; params = $params } | ConvertTo-Json -Depth 10 -Compress
    $bytes = [Text.Encoding]::UTF8.GetBytes($msg)
    $ws.SendAsync([ArraySegment[byte]]$bytes, "Text", $true, $ct).Wait()
    $buf = New-Object byte[] 1048576
    while ($true) {
        $sb = New-Object Text.StringBuilder
        do {
            $r = $ws.ReceiveAsync([ArraySegment[byte]]$buf, $ct).Result
            [void]$sb.Append([Text.Encoding]::UTF8.GetString($buf, 0, $r.Count))
        } while (-not $r.EndOfMessage)
        $resp = $sb.ToString() | ConvertFrom-Json
        if ($resp.id -eq $id) { return $resp }
    }
}

if ($Shot) {
    $r = Call 1 "Page.captureScreenshot" @{ format = "png" }
    [IO.File]::WriteAllBytes($Shot, [Convert]::FromBase64String($r.result.data))
    "saved $Shot"
}
if ($Key) {
    $code = [int][char]$Key.ToUpper()
    foreach ($type in "rawKeyDown", "keyUp") {
        $null = Call 3 "Input.dispatchKeyEvent" @{ type = $type; modifiers = $Modifiers; windowsVirtualKeyCode = $code; key = $Key.ToLower(); code = "Key$($Key.ToUpper())" }
    }
    "pressed $Key (modifiers $Modifiers)"
}
if ($Eval) {
    $r = Call 2 "Runtime.evaluate" @{ expression = $Eval; returnByValue = $true; awaitPromise = $true; userGesture = $true }
    if ($r.result.exceptionDetails) { $r.result.exceptionDetails | ConvertTo-Json -Depth 6 } else { $r.result.result.value | ConvertTo-Json -Depth 8 }
}
