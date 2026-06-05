$ErrorActionPreference='Stop'
$Repo='F:\study\Windows\Applications\Desktop\Utilities\System\Startup\Managers\mich-startup-master'
$Exe=Join-Path $Repo 'build\MichStartupMaster.exe'
$Icon=Join-Path $Repo 'assets\MichStartupMaster.ico'
Add-Type -AssemblyName System.Drawing
if(-not (Test-Path -LiteralPath $Exe)){ throw "Missing EXE $Exe" }
if(-not (Test-Path -LiteralPath $Icon)){ throw "Missing ICO $Icon" }
$ex=[System.Drawing.Icon]::ExtractAssociatedIcon($Exe)
if($null -eq $ex){ throw 'EXE has no extractable icon' }
"EXE_ICON_SIZE=$($ex.Width)x$($ex.Height)"
$ex.Dispose()
$ic=New-Object System.Drawing.Icon($Icon)
"ICO_SIZE=$($ic.Width)x$($ic.Height)"
$ic.Dispose()
$pinnedDir=Join-Path $env:APPDATA 'Microsoft\Internet Explorer\Quick Launch\User Pinned\TaskBar'
"PINNED_DIR=$pinnedDir"
$shell=New-Object -ComObject WScript.Shell
$matches=@()
if(Test-Path -LiteralPath $pinnedDir){
  Get-ChildItem -LiteralPath $pinnedDir -Filter '*.lnk' -Force | ForEach-Object {
    $sc=$shell.CreateShortcut($_.FullName)
    $target=[string]$sc.TargetPath
    $iconLoc=[string]$sc.IconLocation
    if($_.Name -like '*Mich*Startup*' -or $target -like '*MichStartupMaster.exe' -or $iconLoc -like '*MichStartupMaster*'){
      $sc.TargetPath=$Exe
      $sc.WorkingDirectory=(Split-Path -Path $Exe -Parent)
      $sc.IconLocation="$Exe,0"
      $sc.Description='Mich Startup Master - Windows Boot Control'
      $sc.Save()
      $matches += [pscustomobject]@{Shortcut=$_.FullName; Target=$Exe; Icon="$Exe,0"}
    }
  }
}
if($matches.Count -eq 0){
  $shortcut=Join-Path $pinnedDir 'Mich Startup Master.lnk'
  if(Test-Path -LiteralPath $pinnedDir){
    $sc=$shell.CreateShortcut($shortcut)
    $sc.TargetPath=$Exe
    $sc.WorkingDirectory=(Split-Path -Path $Exe -Parent)
    $sc.IconLocation="$Exe,0"
    $sc.Description='Mich Startup Master - Windows Boot Control'
    $sc.Save()
    $matches += [pscustomobject]@{Shortcut=$shortcut; Target=$Exe; Icon="$Exe,0"; Note='created shortcut; pin state depends on Explorer taskbar pins'}
  }
}
$matches | Format-List | Out-String -Width 220
