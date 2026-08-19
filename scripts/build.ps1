$ErrorActionPreference = 'Stop'
$Root = Split-Path -Path $PSScriptRoot -Parent
$OutDir = Join-Path $Root 'build'
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

# The machine's C:\Program Files\dotnet SDK is incomplete (no Sdks folder), so prefer a
# complete SDK. Order: project-local .dotnet (merged toolchain), the Codex tools SDK, a
# DotnetRepair copy, then the system dotnet.
$dotnetCandidates = @(
  (Join-Path $Root '.dotnet\dotnet.exe'),
  'C:\Users\micha\.codex\tools\dotnet-sdk-10.0.301\dotnet.exe',
  'C:\DotnetRepair\dotnet.exe',
  (Get-Command dotnet -ErrorAction SilentlyContinue).Source
)
$dotnet = $dotnetCandidates | Where-Object { $_ -and (Test-Path -LiteralPath $_) } | Select-Object -First 1
if (-not $dotnet) { throw 'A working dotnet SDK was not found.' }

# When building with a relocated SDK, force the muxer to use that install's own runtime
# instead of falling back to the (incomplete) system dotnet.
$env:DOTNET_ROOT = Split-Path -Path $dotnet -Parent
$env:DOTNET_MULTILEVEL_LOOKUP = '0'
$env:DOTNET_CLI_TELEMETRY_OPTOUT = '1'

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
