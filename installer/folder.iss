; Installeur Windows de folder (Inno Setup 6).
;
; Construction locale (après `cargo build --release -p folder`) :
;   iscc /DAppVersion=0.2.0 installer\folder.iss
; Sortie : target\installer\folder-setup-<version>-x64.exe
; En CI : .github/workflows/installer.yml.
;
; Installation par utilisateur sans droits administrateur (choix « pour tous les
; utilisateurs » proposé), comme les applications du Store et VS Code. Les réglages de
; l'utilisateur (%APPDATA%\folder, %LOCALAPPDATA%\folder) sont conservés à la désinstallation.

#ifndef AppVersion
  #define AppVersion "0.2.0"
#endif
#define AppName "folder"
#define AppExe "folder.exe"

[Setup]
; Identifiant stable : ne jamais le changer (mises à jour et désinstallation en dépendent).
AppId={{33FD5014-CFC1-42E4-95CE-F9DC3A2F70EC}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher={#AppName}
VersionInfoVersion={#AppVersion}
VersionInfoDescription=Installeur de {#AppName}
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.17763
SetupIconFile=..\app\folder\folder.ico
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName}
WizardStyle=modern
Compression=lzma2/ultra64
SolidCompression=yes
OutputDir=..\target\installer
OutputBaseFilename=folder-setup-{#AppVersion}-x64
; Ferme folder s'il est ouvert (gestionnaire de redémarrage) avant de remplacer l'exécutable.
CloseApplications=yes
RestartApplications=no

[Languages]
Name: "french"; MessagesFile: "compiler:Languages\French.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "..\target\release\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#AppExe}"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: nowait postinstall skipifsilent
