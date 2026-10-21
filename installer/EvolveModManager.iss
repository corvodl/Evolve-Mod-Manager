#define AppName "Evolve Mod Manager"
#ifndef AppVersion
  #error AppVersion must be specified when building this installer.
#endif

[Setup]
AppId={{E0C92C92-0DC4-4894-BC7C-53870D3458A0}
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher=Evolve Mod Manager
AppPublisherURL=https://github.com/corvodl/Evolve-Mod-Manager
AppSupportURL=https://github.com/corvodl/Evolve-Mod-Manager/issues
DefaultDirName={localappdata}\EvolveModManager
DefaultGroupName=Evolve Mod Manager
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
CloseApplications=yes
RestartApplications=no
UninstallDisplayIcon={app}\evolve-mod-manager.exe
OutputDir=..\dist
OutputBaseFilename=EvolveModManager-Setup-{#AppVersion}
SetupIconFile=..\assets\hunt.ico
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
AllowNoIcons=yes
DisableProgramGroupPage=yes
UsePreviousAppDir=yes

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Shortcuts:"; Flags: unchecked
Name: "startmenuicon"; Description: "Add to the Start Menu"; GroupDescription: "Shortcuts:"; Flags: checkedonce

[Files]
Source: "..\target\release\evolve-mod-manager.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\target\release\evolve-cli.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\assets\hunt.ico"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\Evolve Mod Manager"; Filename: "{app}\evolve-mod-manager.exe"; IconFilename: "{app}\hunt.ico"; Tasks: startmenuicon
Name: "{autodesktop}\Evolve Mod Manager"; Filename: "{app}\evolve-mod-manager.exe"; IconFilename: "{app}\hunt.ico"; Tasks: desktopicon

[Run]
Filename: "{app}\evolve-mod-manager.exe"; Description: "Launch Evolve Mod Manager"; Flags: postinstall nowait skipifsilent

; User preferences, game files, project assets and backups are never deleted.
