# Captures the DISPLAY1 rect (the test panel) into a PNG. No input, no focus change.
param([string]$out)
Add-Type -AssemblyName System.Drawing, System.Windows.Forms
Add-Type 'using System.Runtime.InteropServices; public static class D { [DllImport("user32.dll")] public static extern bool SetProcessDPIAware(); }'
[D]::SetProcessDPIAware() | Out-Null
$b = ([System.Windows.Forms.Screen]::AllScreens | ? { $_.DeviceName -eq '\\.\DISPLAY1' }).Bounds
$bmp = New-Object System.Drawing.Bitmap $b.Width, $b.Height
[System.Drawing.Graphics]::FromImage($bmp).CopyFromScreen($b.X, $b.Y, 0, 0, $bmp.Size)
$bmp.Save($out, [System.Drawing.Imaging.ImageFormat]::Png)
"$($b.X),$($b.Y) $($b.Width)x$($b.Height) -> $out"
