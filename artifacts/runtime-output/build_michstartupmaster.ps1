$ErrorActionPreference='Stop'
$Root='F:\study\Windows\Applications\StartupManager\MichStartupMaster'
$Src=Join-Path $Root 'src\MichStartupMaster.cs'
$OutDir=Join-Path $Root 'build'
$Out=Join-Path $OutDir 'MichStartupMaster.exe'
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
Get-CimInstance Win32_Process | Where-Object { $_.ExecutablePath -eq $Out } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
$candidates=@(
  "$env:WINDIR\Microsoft.NET\Framework64\v4.0.30319\csc.exe",
  "$env:WINDIR\Microsoft.NET\Framework\v4.0.30319\csc.exe"
)
$csc=$candidates | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
if(-not $csc){ throw 'csc.exe not found' }
& $csc /nologo /target:winexe /platform:anycpu /optimize+ /out:$Out /reference:System.dll /reference:System.Core.dll /reference:System.Drawing.dll /reference:System.Windows.Forms.dll /reference:System.Management.dll /reference:Microsoft.CSharp.dll $Src
if($LASTEXITCODE -ne 0){ throw "csc failed $LASTEXITCODE" }
Get-Item -LiteralPath $Out | Select-Object FullName,Length,LastWriteTime | Format-List
