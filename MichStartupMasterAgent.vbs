Option Explicit

Dim shell, fileSystem, installDirectory, executablePath
Set shell = CreateObject("WScript.Shell")
Set fileSystem = CreateObject("Scripting.FileSystemObject")
installDirectory = fileSystem.GetParentFolderName(WScript.ScriptFullName)
executablePath = fileSystem.BuildPath(installDirectory, "MichStartupMaster.exe")

shell.Run Chr(34) & executablePath & Chr(34) & " --agent", 0, False
