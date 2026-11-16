; Fetchbox installer (Inno Setup 6). Built by package.ps1:
;   ISCC /DAppVersion=2.0.4 /DStage=..\dist\Fetchbox /DOut=..\dist installer\fetchbox.iss
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
; first install: open on Settings, where the steps for adding the browser extension are
Filename: "{app}\fetchbox.exe"; Parameters: "--settings"; Description: "Open Fetchbox (shows how to add the browser extension)"; Flags: nowait postinstall skipifsilent; Check: not IsUpgrade
; upgrade: just open it
Filename: "{app}\fetchbox.exe"; Description: "Open Fetchbox"; Flags: nowait postinstall skipifsilent; Check: IsUpgrade
; an update started from inside Fetchbox (/SILENT /RELAUNCH) opens it again when done
Filename: "{app}\fetchbox.exe"; Flags: nowait; Check: Relaunch

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

function Relaunch: Boolean;
begin
  Result := WizardSilent and (Pos('/RELAUNCH', Uppercase(GetCmdTail)) > 0);
end;

// Is Fetchbox already installed? Then this run is an upgrade: same folder, same options,
// no questions, and the old version's program files are replaced.
const
  // the AppId from [Setup], with the uninstall key's "_is1" suffix
  UninstallKey = 'Software\Microsoft\Windows\CurrentVersion\Uninstall\{6B0C1F0E-7C56-4E0B-9B5F-FE7C4B0A2D11}_is1';

var
  Upgrading: Boolean;
  OldVersion: String;

function InitializeSetup: Boolean;
begin
  Upgrading := RegKeyExists(HKCU, UninstallKey);
  if not RegQueryStringValue(HKCU, UninstallKey, 'DisplayVersion', OldVersion) then
    OldVersion := '';
  Result := True;
end;

// Upgrading, the only page left is "Ready to install": make it read as an update.
procedure CurPageChanged(CurPageID: Integer);
begin
  if Upgrading and (CurPageID = wpReady) then
  begin
    WizardForm.PageNameLabel.Caption := 'Update Fetchbox';
    WizardForm.PageDescriptionLabel.Caption := 'Your settings and downloaded tools are kept.';
    if OldVersion <> '' then
      WizardForm.ReadyLabel.Caption := 'Fetchbox ' + OldVersion + ' will be replaced with version {#AppVersion}.'
    else
      WizardForm.ReadyLabel.Caption := 'The installed Fetchbox will be replaced with version {#AppVersion}.';
    WizardForm.NextButton.Caption := 'Update';
  end;
end;

function IsUpgrade: Boolean;
begin
  Result := Upgrading;
end;

// Upgrading: clear out the old version first, so nothing it shipped is left behind. Kept: tools Fetchbox
// downloaded (runtime), logs, the helper's manifests (native), settings of a former portable copy, and
// the uninstaller.
function KeepOnUpgrade(Name: String): Boolean;
begin
  Name := Lowercase(Name);
  Result := (Name = 'runtime') or (Name = 'logs') or (Name = 'native') or (Name = 'settings') or (Pos('unins', Name) = 1);
end;

procedure ClearOldVersion;
var
  Dir: String;
  F: TFindRec;
begin
  Dir := ExpandConstant('{app}');
  // only a folder of its own: never wipe a folder someone installed Fetchbox into directly (e.g. Desktop)
  if (Lowercase(ExtractFileName(RemoveBackslashUnlessRoot(Dir))) <> 'fetchbox')
     or not FileExists(Dir + '\fetchbox.exe') or not FileExists(Dir + '\unins000.exe') then
    Exit;
  if FindFirst(Dir + '\*', F) then
  try
    repeat
      if (F.Name <> '.') and (F.Name <> '..') and not KeepOnUpgrade(F.Name) then
        if F.Attributes and FILE_ATTRIBUTE_DIRECTORY <> 0 then
          DelTree(Dir + '\' + F.Name, True, True, True)
        else
          DeleteFile(Dir + '\' + F.Name);
    until not FindNext(F);
  finally
    FindClose(F);
  end;
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if (CurStep = ssInstall) and Upgrading then
    ClearOldVersion;
end;

function ShouldSkipPage(PageID: Integer): Boolean;
begin
  // licence, options and "ready to install" were all answered the first time
  Result := Upgrading and ((PageID = wpLicense) or (PageID = wpSelectTasks) or (PageID = wpReady));
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
