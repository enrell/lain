# Sends keystrokes (SendKeys syntax) to a process's main window.
# usage: powershell -File keys.ps1 <process-name> "<keys>" [delay-ms-between]
param([string]$Name, [string]$Keys, [int]$Delay = 150)
Add-Type -AssemblyName System.Windows.Forms
Add-Type @"
using System; using System.Runtime.InteropServices;
public class K { [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h); }
"@
$p = Get-Process -Name $Name | Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
[K]::SetForegroundWindow($p.MainWindowHandle) | Out-Null
Start-Sleep -Milliseconds 300
foreach ($k in $Keys.Split(' ')) { [System.Windows.Forms.SendKeys]::SendWait($k); Start-Sleep -Milliseconds $Delay }
