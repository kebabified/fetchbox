; Fetchbox installer (Inno Setup 6). Built by package.ps1:
;   ISCC /DAppVersion=2.0.1 /DStage=..\dist\Fetchbox /DOut=..\dist installer\fetchbox.iss
;
; Installs for the current user only (no admin prompt) into %LOCALAPPDATA%\Programs\Fetchbox,
; registers the browser helper, and adds an uninstaller.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef Stage
  #define Stage "..\dist\Fetchbox"
#endif
#ifndef Out
  #define Out "..\dist"
#endif

[Setup]
AppId={{6B0C1F0E-7C56-4E0B-9B5F-FE7C4B0A2D11}
AppName=Fetchbox
AppVersion={#AppVersion}
AppPublisher=Fetchbox contributors
DefaultDirName={localappdata}\Programs\Fetchbox
DefaultGroupName=Fetchbox
DisableProgramGroupPage=yes
DisableDirPage=auto
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir={#Out}
OutputBaseFilename=Fetchbox-{#AppVersion}-setup
SetupIconFile=..\fetchbox.ico
UninstallDisplayIcon={app}\fetchbox.exe
UninstallDisplayName=Fetchbox
LicenseFile=..\LICENSE
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
CloseApplications=no

[Tasks]
Name: "desktopicon"; Description: "Add a Desktop shortcut"; Flags: unchecked

[Files]
; no portable.txt here: an installed copy keeps its settings in %APPDATA%\Fetchbox
Source: "{#Stage}\*"; DestDir: "{app}"; Excludes: "portable.txt"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{userprograms}\Fetchbox"; Filename: "{app}\fetchbox.exe"
Name: "{userdesktop}\Fetchbox"; Filename: "{app}\fetchbox.exe"; Tasks: desktopicon

[Run]
; connect Chrome, Edge, Brave, Vivaldi, Helium… to the helper (current user's registry)
Filename: "{app}\fetchbox-host.exe"; Parameters: "install"; Flags: runhidden; StatusMsg: "Connecting your browsers..."
; open on Settings, where the steps for adding the browser extension are
Filename: "{app}\fetchbox.exe"; Parameters: "--settings"; Description: "Open Fetchbox (shows how to add the browser extension)"; Flags: nowait postinstall skipifsilent

[UninstallRun]
Filename: "{app}\fetchbox-host.exe"; Parameters: "uninstall"; Flags: runhidden; RunOnceId: "UnregisterHelper"

[UninstallDelete]
; tools Fetchbox downloaded for itself, logs and the helper's manifest
Type: filesandordirs; Name: "{app}\runtime"
Type: filesandordirs; Name: "{app}\logs"
Type: filesandordirs; Name: "{app}\native"
Type: filesandordirs; Name: "{app}\tmp"
Type: dirifempty; Name: "{app}"

[Code]
// Fetchbox and its background service (and the cobalt server it may have started) must not be
// running while files are replaced or removed.
procedure StopFetchbox;
var
  Code: Integer;
begin
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/IM fetchbox.exe /F', '', SW_HIDE, ewWaitUntilTerminated, Code);
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/IM fetchbox-host.exe /T /F', '', SW_HIDE, ewWaitUntilTerminated, Code);
  Sleep(400);
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  StopFetchbox;
  Result := '';
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usUninstall then
    StopFetchbox;
  if CurUninstallStep = usPostUninstall then
    if MsgBox('Also remove your Fetchbox settings?', mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES then
      DelTree(ExpandConstant('{userappdata}\Fetchbox'), True, True, True);
end;
