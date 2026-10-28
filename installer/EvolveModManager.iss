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
Name: "gfxeditors"; Description: "Include Ruffle Desktop/Web and FFDec for live GFX editing (about 50 MB additional download size)"; GroupDescription: "Optional GFX tools:"; Flags: checkedonce
Name: "audiotools"; Description: "Include vgmstream decoder and Vorbis FSB5 sound compiler tools (about 4.5 MB additional download size)"; GroupDescription: "Optional Audio tools:"; Flags: unchecked

[Files]
Source: "..\target\release\evolve-mod-manager.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\target\release\evolve-cli.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\target\release\evolve-teleport-overlay.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\assets\hunt.ico"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\assets\default_loading.dds"; DestDir: "{app}\assets"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion
; In CI smoke tests the official vendor bundle is intentionally absent; release
; builds must populate this path from the verified private release first.
#if FileExists("..\dist\third_party\OfficialStage2\evolve_tool.exe")
Source: "..\dist\third_party\OfficialStage2\*"; DestDir: "{app}\third_party\OfficialStage2"; Flags: ignoreversion recursesubdirs createallsubdirs
#endif
; Bundle verified upstream portable distributions, never game assets.
; User can deselect the GFX tools task; only manager is required.
Source: "..\dist\third_party\FFDec\*"; DestDir: "{app}\third_party\FFDec"; Flags: ignoreversion recursesubdirs createallsubdirs; Tasks: gfxeditors
Source: "..\dist\third_party\Ruffle\*"; DestDir: "{app}\third_party\Ruffle"; Flags: ignoreversion recursesubdirs createallsubdirs; Tasks: gfxeditors
Source: "..\dist\third_party\RuffleWeb\*"; DestDir: "{app}\third_party\RuffleWeb"; Flags: ignoreversion recursesubdirs createallsubdirs; Tasks: gfxeditors
Source: "..\dist\third_party\THIRD_PARTY_TOOLS.txt"; DestDir: "{app}\third_party"; Flags: ignoreversion; Tasks: gfxeditors
Source: "..\dist\third_party\vgmstream\*"; DestDir: "{app}\third_party\vgmstream"; Flags: ignoreversion recursesubdirs createallsubdirs; Tasks: audiotools
Source: "..\dist\third_party\oggvorbis2fsb5\*"; DestDir: "{app}\third_party\oggvorbis2fsb5"; Flags: ignoreversion recursesubdirs createallsubdirs; Tasks: audiotools

[Icons]
Name: "{group}\Evolve Mod Manager"; Filename: "{app}\evolve-mod-manager.exe"; IconFilename: "{app}\hunt.ico"; Tasks: startmenuicon
Name: "{autodesktop}\Evolve Mod Manager"; Filename: "{app}\evolve-mod-manager.exe"; IconFilename: "{app}\hunt.ico"; Tasks: desktopicon

[Run]
Filename: "{app}\evolve-mod-manager.exe"; Description: "Launch Evolve Mod Manager"; Flags: postinstall nowait skipifsilent

; User preferences, game files, project assets and backups are never deleted.
