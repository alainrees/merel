; Inno Setup script for the Windows installer: a short wizard that installs Mailcraft for the
; current user (no administrator needed), adds it to the Start menu and can start it at the end.
; Build from the repository root after `cargo build --release -p mailcraft`:
;   iscc /DAppVersion=0.1.0 packaging\windows\mailcraft.iss

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif

[Setup]
AppId={{6F1C3B8E-5A2D-4E7B-9C41-8D2F6A0B7E53}
AppName=Mailcraft
AppVersion={#AppVersion}
AppVerName=Mailcraft {#AppVersion}
AppPublisher=Mailcraft
AppPublisherURL=https://github.com/alainrees/mailcraft
AppUpdatesURL=https://github.com/alainrees/mailcraft/releases
DefaultDirName={autopf}\Mailcraft
DefaultGroupName=Mailcraft
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\..\dist
OutputBaseFilename=Mailcraft-{#AppVersion}-windows-setup
SetupIconFile=mailcraft.ico
UninstallDisplayIcon={app}\mailcraft.exe
Compression=lzma2
SolidCompression=yes
WizardStyle=modern

[Languages]
Name: "dutch"; MessagesFile: "compiler:Languages\Dutch.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "..\..\target\release\mailcraft.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "mailcraft.ico"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\Mailcraft"; Filename: "{app}\mailcraft.exe"; IconFilename: "{app}\mailcraft.ico"
Name: "{autodesktop}\Mailcraft"; Filename: "{app}\mailcraft.exe"; IconFilename: "{app}\mailcraft.ico"; Tasks: desktopicon

[Run]
Filename: "{app}\mailcraft.exe"; Description: "{cm:LaunchProgram,Mailcraft}"; Flags: nowait postinstall skipifsilent
