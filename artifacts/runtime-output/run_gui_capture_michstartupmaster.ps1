$ErrorActionPreference='Stop'
$App='F:\study\Windows\Applications\StartupManager\MichStartupMaster\build\MichStartupMaster.exe'
# Stop only previous instances of this exact app to avoid duplicate tray icons during verification.
Get-CimInstance Win32_Process | Where-Object { $_.ExecutablePath -eq $App } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
$p=Start-Process -FilePath $App -PassThru
Start-Sleep -Seconds 3
$proc=Get-CimInstance Win32_Process | Where-Object { $_.ExecutablePath -eq $App } | Select-Object -First 1
if(-not $proc){ throw 'GUI process did not remain alive' }
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
$bounds=[System.Windows.Forms.SystemInformation]::VirtualScreen
$bmp=New-Object System.Drawing.Bitmap $bounds.Width,$bounds.Height
$g=[System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($bounds.Left,$bounds.Top,0,0,$bounds.Size)
$out='C:\Temp\hermes_snap\MichStartupMaster_running.png'
New-Item -ItemType Directory -Force -Path (Split-Path -Path $out -Parent) | Out-Null
$bmp.Save($out,[System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose(); $bmp.Dispose()
"GUI_PROCESS pid=$($proc.ProcessId) path=$($proc.ExecutablePath)"
"SCREENSHOT $out size=$((Get-Item $out).Length)"
