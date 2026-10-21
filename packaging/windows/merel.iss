; Inno Setup script for the Windows installer: a short wizard that installs Merel for the
; current user (no administrator needed), adds it to the Start menu and can start it at the end.
; Build from the repository root after `cargo build --release -p merel`:
;   iscc /DAppVersion=0.1.0 packaging\windows\merel.iss

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif

[Setup]
AppId={{6F1C3B8E-5A2D-4E7B-9C41-8D2F6A0B7E53}
AppName=Merel
AppVersion={#AppVersion}
AppVerName=Merel {#AppVersion}
AppPublisher=Merel
AppPublisherURL=https://github.com/alainrees/merel
AppUpdatesURL=https://github.com/alainrees/merel/releases
DefaultDirName={autopf}\Merel
DefaultGroupName=Merel
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\..\dist
OutputBaseFilename=Merel-{#AppVersion}-windows-setup
SetupIconFile=merel.ico
UninstallDisplayIcon={app}\merel.exe
Compression=lzma2
SolidCompression=yes
WizardStyle=modern

[Languages]
Name: "dutch"; MessagesFile: "compiler:Languages\Dutch.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "..\..\target\release\merel.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "merel.ico"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\Merel"; Filename: "{app}\merel.exe"; IconFilename: "{app}\merel.ico"
Name: "{autodesktop}\Merel"; Filename: "{app}\merel.exe"; IconFilename: "{app}\merel.ico"; Tasks: desktopicon

[Run]
Filename: "{app}\merel.exe"; Description: "{cm:LaunchProgram,Merel}"; Flags: nowait postinstall skipifsilent
; An update from inside Merel runs this installer silently; then Merel starts again by itself.
Filename: "{app}\merel.exe"; Flags: nowait skipifnotsilent
