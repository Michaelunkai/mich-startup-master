$ErrorActionPreference = 'Stop'
$Root = Split-Path -Path $PSScriptRoot -Parent
$OutDir = Join-Path $Root 'build'
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

# The machine's C:\Program Files\dotnet SDK is incomplete (no Sdks folder), so prefer the
# complete SDK copy at C:\DotnetRepair when present; otherwise fall back to the system dotnet.
$dotnet = @(
  'C:\DotnetRepair\dotnet.exe',
  (Get-Command dotnet -ErrorAction SilentlyContinue).Source
) | Where-Object { $_ -and (Test-Path -LiteralPath $_) } | Select-Object -First 1
if (-not $dotnet) { throw 'A working dotnet SDK was not found. Install one or restore C:\DotnetRepair.' }

# Stop any running copy of the app so its files are not locked.
Get-CimInstance Win32_Process -Filter "Name='MichStartupMaster.exe'" -ErrorAction SilentlyContinue |
  ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }

# Clean the previous layout so stale binaries cannot be mistaken for current ones.
Get-ChildItem -LiteralPath $OutDir -File -ErrorAction SilentlyContinue | Remove-Item -Force -ErrorAction SilentlyContinue

& $dotnet publish (Join-Path $Root 'MichStartupMaster.csproj') -c Release -r win-x64 --self-contained true -o $OutDir
if ($LASTEXITCODE -ne 0) { throw "dotnet publish failed with exit $LASTEXITCODE" }

# The icon and hidden VBS launcher are declared as content in the project but are copied
# explicitly so the deployed layout always carries them.
Copy-Item -LiteralPath (Join-Path $Root 'assets\MichStartupMaster.ico') -Destination (Join-Path $OutDir 'MichStartupMaster.ico') -Force
Copy-Item -LiteralPath (Join-Path $Root 'MichStartupMasterAgent.vbs') -Destination (Join-Path $OutDir 'MichStartupMasterAgent.vbs') -Force

Add-Type -AssemblyName System.Drawing
$extracted = [System.Drawing.Icon]::ExtractAssociatedIcon((Join-Path $OutDir 'MichStartupMaster.exe'))
if ($null -eq $extracted) { throw 'Built EXE icon extraction failed' }
$extracted.Dispose()

Get-Item -LiteralPath (Join-Path $OutDir 'MichStartupMaster.exe') | Select-Object FullName, Length, LastWriteTime | Format-List
"Publish SDK: $dotnet"
"Output: $OutDir"
