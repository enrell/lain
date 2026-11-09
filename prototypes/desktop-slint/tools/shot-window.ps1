# Captures the lain-slint window (PrintWindow, PW_RENDERFULLCONTENT) into a PNG.
# Finds the visible top-level window of the process; no focus change, no input.
param([string]$out, [string]$name = "lain-slint")
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System; using System.Runtime.InteropServices;
public struct RECT { public int L, T, R, B; }
public static class W {
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc f, IntPtr l);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint flags);
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  public static IntPtr Find(uint pid) { IntPtr best = IntPtr.Zero; int area = 0;
    EnumWindows((h, l) => { uint p; GetWindowThreadProcessId(h, out p); RECT r;
      if (p == pid && IsWindowVisible(h) && GetWindowRect(h, out r) && (r.R-r.L)*(r.B-r.T) > area) { area = (r.R-r.L)*(r.B-r.T); best = h; }
      return true; }, IntPtr.Zero); return best; }
}
"@
[W]::SetProcessDPIAware() | Out-Null
$p = Get-Process $name -ErrorAction Stop | Select-Object -First 1
$h = [W]::Find([uint32]$p.Id)
$r = New-Object RECT
[W]::GetWindowRect($h, [ref]$r) | Out-Null
$bmp = New-Object System.Drawing.Bitmap ($r.R - $r.L), ($r.B - $r.T)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$dc = $g.GetHdc(); [W]::PrintWindow($h, $dc, 2) | Out-Null; $g.ReleaseHdc($dc)
$bmp.Save($out, [System.Drawing.Imaging.ImageFormat]::Png)
"$($r.L),$($r.T) $($r.R - $r.L)x$($r.B - $r.T) -> $out"
