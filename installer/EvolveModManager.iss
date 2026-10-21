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
; The GUI executable itself requests requireAdministrator via app.manifest.
; It must live in an administrator-controlled folder, never writable
; LocalAppData, or a lower-privilege user could replace a privileged EXE.
; Inno Setup will request UAC itself before copying or launching the GUI.
DefaultDirName={autopf}\EvolveModManager
DefaultGroupName=Evolve Mod Manager
PrivilegesRequired=admin
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
; Old nightlies installed the admin-required GUI in a user-writable
; LocalAppData directory. Do not reuse that unsafe location on upgrade.
; User settings/assets live separately in LocalAppData and remain untouched.
UsePreviousAppDir=no

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
