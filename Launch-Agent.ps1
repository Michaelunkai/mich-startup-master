$exe = 'F:\study\Windows\Applications\Desktop\Utilities\System\Startup\Managers\mich-startup-master\build\MichStartupMaster.exe'
$workingDirectory = Split-Path -Parent $exe
$process = Start-Process -FilePath $exe -ArgumentList '--agent' -WorkingDirectory $workingDirectory -PassThru
$process.Id
