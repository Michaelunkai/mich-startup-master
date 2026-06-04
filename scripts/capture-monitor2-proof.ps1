$ErrorActionPreference='Stop'
$Root = Split-Path -Path $PSScriptRoot -Parent
$App = Join-Path $Root 'build\MichStartupMaster.exe'
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
$target=[System.Windows.Forms.Screen]::AllScreens | Where-Object { -not $_.Primary } | Sort-Object { $_.Bounds.X } | Select-Object -First 1
if(-not $target){ throw 'No secondary monitor found' }
$p=Get-Process | Where-Object { $_.Path -eq $App } | Select-Object -First 1
if(-not $p){ throw 'Mich Startup Master is not running from this repo path' }
$bounds=$target.Bounds
$bmp=New-Object System.Drawing.Bitmap $bounds.Width,$bounds.Height
$g=[System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($bounds.Left,$bounds.Top,0,0,$bounds.Size)
$out=Join-Path $Root 'artifacts\proof\monitor2-current.png'
New-Item -ItemType Directory -Force -Path (Split-Path -Path $out -Parent) | Out-Null
$bmp.Save($out,[System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose(); $bmp.Dispose()
"SCREENSHOT=$out SIZE=$((Get-Item $out).Length) PID=$($p.Id) TITLE=$($p.MainWindowTitle)"
