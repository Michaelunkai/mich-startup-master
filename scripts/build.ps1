$ErrorActionPreference='Stop'
$Root = Split-Path -Path $PSScriptRoot -Parent
$Src = Join-Path $Root 'src\MichStartupMaster.cs'
$Icon = Join-Path $Root 'assets\MichStartupMaster.ico'
$OutDir = Join-Path $Root 'build'
$Out = Join-Path $OutDir 'MichStartupMaster.exe'
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
Get-CimInstance Win32_Process | Where-Object { $_.ExecutablePath -eq $Out } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
if(-not (Test-Path -LiteralPath $Icon)){ throw "Application icon not found: $Icon" }
$candidates = @(
  "$env:WINDIR\Microsoft.NET\Framework64\v4.0.30319\csc.exe",
  "$env:WINDIR\Microsoft.NET\Framework\v4.0.30319\csc.exe"
)
$csc = $candidates | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
if(-not $csc){ throw 'C# compiler csc.exe was not found. Install/enable .NET Framework developer tools.' }
$cscArgs = @(
  '/nologo','/target:winexe','/platform:x64','/optimize+',
  "/out:$Out", "/win32icon:$Icon",
  '/reference:System.dll','/reference:System.Core.dll','/reference:System.Drawing.dll','/reference:System.Windows.Forms.dll','/reference:System.Management.dll','/reference:Microsoft.CSharp.dll',
  $Src
)
& $csc @cscArgs
if($LASTEXITCODE -ne 0){ throw "csc failed with exit $LASTEXITCODE" }
Add-Type -AssemblyName System.Drawing
$extracted = [System.Drawing.Icon]::ExtractAssociatedIcon($Out)
if($null -eq $extracted){ throw 'Built EXE icon extraction failed' }
$extracted.Dispose()
Get-Item -LiteralPath $Out | Select-Object FullName,Length,LastWriteTime | Format-List
"Embedded icon: $Icon"
