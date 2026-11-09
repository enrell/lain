# Moves the mouse to window-relative (x, y) of a process's main window, optionally clicking.
# usage: powershell -File mouse.ps1 <process-name> <x> <y> [click]
param([string]$Name, [int]$X, [int]$Y, [string]$Click = "")
Add-Type @"
using System; using System.Runtime.InteropServices;
public class M {
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(int f, int x, int y, int d, int e);
  [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr h, int a, out RECT r, int s);
}
"@
[M]::SetProcessDPIAware() | Out-Null
$p = Get-Process -Name $Name | Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
$h = $p.MainWindowHandle
[M]::SetForegroundWindow($h) | Out-Null
$r = New-Object M+RECT
[M]::DwmGetWindowAttribute($h, 9, [ref]$r, 16) | Out-Null
[M]::SetCursorPos($r.L + $X - 10, $r.T + $Y - 10) | Out-Null
Start-Sleep -Milliseconds 60
[M]::SetCursorPos($r.L + $X, $r.T + $Y) | Out-Null
if ($Click -eq "click") { Start-Sleep -Milliseconds 60; [M]::mouse_event(2,0,0,0,0); Start-Sleep -Milliseconds 40; [M]::mouse_event(4,0,0,0,0) }
