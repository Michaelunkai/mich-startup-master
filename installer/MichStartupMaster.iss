; Mich Startup Master — per-user Windows installer (no administrator rights required).
; Build with:  ISCC.exe installer\MichStartupMaster.iss
; Output:      dist\MichStartupMaster-Setup.exe

#define MyAppName "Mich Startup Master"
#define MyAppVersion "1.0.0"
#define MyAppPublisher "Michaelunkai"
#define MyAppExeName "MichStartupMaster.exe"
#define MyAppId "{9E4E8B9C-5E2A-4D8B-9C3E-7F1A2B3C4D5E}"

[Setup]
AppId={{#MyAppId}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
DefaultDirName={localappdata}\Programs\MichStartupMaster
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
DisableWelcomePage=no
; Per-user install: works on any Windows 11 machine without elevation.
PrivilegesRequired=lowest
OutputDir=..\dist
OutputBaseFilename=MichStartupMaster-Setup
SetupIconFile=..\assets\MichStartupMaster.ico
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
UninstallDisplayIcon={app}\{#MyAppExeName}
UninstallDisplayName={#MyAppName}
CloseApplications=yes
RestartApplications=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "Create a &desktop shortcut"; GroupDescription: "Additional shortcuts:"

[Files]
Source: "..\build\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{group}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; IconFilename: "{app}\MichStartupMaster.ico"
Name: "{autoprograms}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; IconFilename: "{app}\MichStartupMaster.ico"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; IconFilename: "{app}\MichStartupMaster.ico"; Tasks: desktopicon

[Run]
; Start the hidden agent now (it self-registers the startup agent + managed tasks).
; nowait so the installer never blocks; postinstall so it only runs after a clean install.
Filename: "{app}\{#MyAppExeName}"; Parameters: "--agent"; Flags: nowait postinstall skipifsilent; Description: "Launch {#MyAppName} now"

[UninstallDelete]
Type: filesandordirs; Name: "{app}"
