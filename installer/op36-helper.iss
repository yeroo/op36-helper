; OP36 Helper installer (Inno Setup 6): per-user, no admin, bundles the firmware pair.
; Build with installer\build.ps1, which stages the files and passes /DAppVersion=<ver>.
#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#define AppName "OP36 Helper"
#define AppExe "op36-helper.exe"
#define AppPublisher "Boris Kudriashov"
#define AppUrl "https://github.com/yeroo/op36-helper"

[Setup]
AppId={{7D2C6E1A-4B8F-4F2E-9C3A-0036B7E1A5C1}
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher={#AppPublisher}
AppPublisherURL={#AppUrl}
AppSupportURL={#AppUrl}
VersionInfoVersion={#AppVersion}
DefaultDirName={localappdata}\Programs\OP36 Helper
DisableProgramGroupPage=yes
DisableDirPage=auto
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
SetupIconFile=..\helper\assets\icon_on.ico
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName}
WizardStyle=modern
Compression=lzma2
SolidCompression=yes
; A running helper is closed before its files are replaced (the self-updater exits first anyway).
CloseApplications=yes
RestartApplications=no
OutputDir=Output
OutputBaseFilename=op36-helper-setup-{#AppVersion}
LicenseFile=stage\LICENSE

[Tasks]
Name: "startup";     Description: "Start {#AppName} with Windows";  GroupDescription: "Start-up:"
Name: "desktopicon"; Description: "Create a &desktop shortcut";     GroupDescription: "Shortcuts:"; Flags: unchecked

[Files]
Source: "stage\*"; DestDir: "{app}"; Flags: recursesubdirs createallsubdirs ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"; IconFilename: "{app}\assets\icon_on.ico"
Name: "{autodesktop}\{#AppName}";  Filename: "{app}\{#AppExe}"; IconFilename: "{app}\assets\icon_on.ico"; Tasks: desktopicon
; The same shortcut the helper's own "Start with Windows" setting manages, so Options reflects it.
Name: "{userstartup}\{#AppName}";  Filename: "{app}\{#AppExe}"; IconFilename: "{app}\assets\icon_on.ico"; Tasks: startup

[Run]
Filename: "{app}\{#AppExe}"; Description: "Launch {#AppName}"; Flags: nowait postinstall skipifsilent

[UninstallDelete]
Type: filesandordirs; Name: "{app}"
; Settings, the keymap cache and the log stay in %APPDATA%\op36-helper on purpose.
