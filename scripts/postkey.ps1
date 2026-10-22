# Posts a key press to every Chrome render widget window of Burrow (dev testing only).
param([int]$Vk)
Add-Type @"
using System; using System.Text; using System.Collections.Generic; using System.Runtime.InteropServices;
public class PK {
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumChildWindows(IntPtr p, EnumProc f, IntPtr l);
  [DllImport("user32.dll")] public static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  public static List<IntPtr> Find(IntPtr top) {
    var r = new List<IntPtr>();
    EnumChildWindows(top, (h, l) => { var sb = new StringBuilder(256); GetClassName(h, sb, 256);
      if (sb.ToString() == "Chrome_RenderWidgetHostHWND" && IsWindowVisible(h)) r.Add(h); return true; }, IntPtr.Zero);
    return r;
  }
}
"@
$p = Get-Process burrow | Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
$hs = [PK]::Find($p.MainWindowHandle)
"render widgets: $($hs.Count)"
$h = $hs[$hs.Count - 1]
[PK]::PostMessage($h, 0x100, [IntPtr]$Vk, [IntPtr]1) | Out-Null
Start-Sleep -Milliseconds 100
[PK]::PostMessage($h, 0x101, [IntPtr]$Vk, [IntPtr]0xC0000001) | Out-Null
"posted vk=$Vk"
