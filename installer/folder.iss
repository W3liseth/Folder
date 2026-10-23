; Installeur Windows de folder (Inno Setup 6).
;
; Construction locale (après `cargo build --release -p folder`) :
;   iscc /DAppVersion=0.5.1 installer\folder.iss
; Sortie : target\installer\folder-setup-<version>-x64.exe
; En CI : .github/workflows/installer.yml.
;
; Installation par utilisateur sans droits administrateur (choix « pour tous les
; utilisateurs » proposé), comme les applications du Store et VS Code. Les réglages de
; l'utilisateur (%APPDATA%\folder, %LOCALAPPDATA%\folder) sont conservés à la désinstallation.

#ifndef AppVersion
  #define AppVersion "0.5.1"
#endif
#define AppName "folder"
#define AppExe "folder.exe"
; Partie numérique de la version (0.4.0-beta.1 → 0.4.0) : seule acceptée par VersionInfoVersion.
#define NumVersion Copy(AppVersion, 1, Pos("-", AppVersion + "-") - 1)

[Setup]
; Identifiant stable : ne jamais le changer (mises à jour et désinstallation en dépendent).
AppId={{33FD5014-CFC1-42E4-95CE-F9DC3A2F70EC}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher={#AppName}
VersionInfoVersion={#NumVersion}
VersionInfoDescription=Installeur de {#AppName}
; Nom et version du produit contrôlés à la signature (.signpath/artifact-configuration).
VersionInfoProductName={#AppName}
VersionInfoProductTextVersion={#AppVersion}
#ifdef SignUninstaller
; Désinstalleur signé sans SignTool (signature distante, SignPath) : la première
; compilation dépose ici l'exemplaire à signer, la suivante en intègre la signature
; (cf. .github/workflows/installer.yml).
SignedUninstaller=yes
SignedUninstallerDir=..\target\uninstaller
#endif
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.17763
SetupIconFile=..\app\folder\folder.ico
; Icône de folder en haut de l'assistant, à 100 % et 200 % (générées par
; app\folder\icon\make_ico.ps1), avec leur transparence.
WizardSmallImageFile=wizard-small-55.bmp,wizard-small-110.bmp
WizardImageAlphaFormat=defined
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
; Processus résident de Windows + E relancé après une mise à jour (il se termine aussitôt si
; l'option est désactivée). Pour l'utilisateur, même depuis une installation élevée.
Filename: "{app}\{#AppExe}"; Parameters: "--win-e"; Flags: nowait runasoriginaluser
Filename: "{app}\{#AppExe}"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: nowait postinstall skipifsilent

[UninstallRun]
; Windows + E rendu à l'Explorateur et raccourci clavier d'ouverture retiré (réglages
; « Raccourcis » de folder), avant la suppression de l'exécutable.
Filename: "{app}\{#AppExe}"; Parameters: "--restore-explorer"; Flags: runhidden waituntilterminated; RunOnceId: "RestoreExplorer"

[Code]
// Mise à jour : le processus résident de Windows + E (sans fenêtre, que le gestionnaire de
// redémarrage ne sait pas fermer) est arrêté avant le remplacement de l'exécutable ; la
// section [Run] le relance.
function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  Exe: String;
  Code: Integer;
begin
  Exe := ExpandConstant('{app}\{#AppExe}');
  if FileExists(Exe) then
    ExecAsOriginalUser(Exe, '--stop-win-e', '', SW_HIDE, ewWaitUntilTerminated, Code);
  Result := '';
end;
