; Crew's per-user installer. Build through scripts/windows_installer.py.
; No services, PATH edits, automatic application closure, or user-data deletion.
#ifndef CrewVersion
  #error CrewVersion is required
#endif
#ifndef PayloadDir
  #error PayloadDir is required
#endif
#ifndef InstallerOutputDir
  #error InstallerOutputDir is required
#endif
#ifndef InstallerBaseName
  #error InstallerBaseName is required
#endif

[Setup]
AppId=urtiger101-tw.Crew
AppName=Crew
AppVersion={#CrewVersion}
AppVerName=Crew {#CrewVersion}
AppPublisher=urtiger101-tw
AppPublisherURL=https://github.com/urtiger101-tw/Crew
AppSupportURL=https://github.com/urtiger101-tw/Crew/issues
AppUpdatesURL=https://github.com/urtiger101-tw/Crew/releases
DefaultDirName={localappdata}\Programs\Crew
DefaultGroupName=Crew
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=
SetupArchitecture=x64
ArchitecturesAllowed=x64os
ArchitecturesInstallIn64BitMode=x64os
MinVersion=10.0.17763
UsePreviousAppDir=yes
UninstallLogMode=append
UninstallDisplayIcon={app}\crew.exe
LicenseFile={#PayloadDir}\LICENSE
OutputDir={#InstallerOutputDir}
OutputBaseFilename={#InstallerBaseName}
VersionInfoVersion={#CrewVersion}.0
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
CloseApplications=no
RestartApplications=no
ChangesEnvironment=no
AlwaysRestart=no
SetupLogging=yes

[Files]
Source: "{#PayloadDir}\crew.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDir}\tmuxlike.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDir}\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDir}\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDir}\manifest.json"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\Crew"; Filename: "{app}\crew.exe"; WorkingDir: "{userprofile}"

[Code]
const
  UninstallKey = 'Software\Microsoft\Windows\CurrentVersion\Uninstall\urtiger101-tw.Crew_is1';

function ReportFailure(const Message: String): Boolean;
begin
  Log(Message);
  SuppressibleMsgBox(Message, mbError, MB_OK, IDOK);
  Result := False;
end;

function InitializeSetup: Boolean;
var
  I: Integer;
  Argument: String;
begin
  Result := True;
  for I := 1 to ParamCount do begin
    Argument := Uppercase(ParamStr(I));
    if (Argument = '/CLOSEAPPLICATIONS') or (Argument = '/FORCECLOSEAPPLICATIONS') or
       (Argument = '/RESTARTAPPLICATIONS') or (Argument = '/ALLUSERS') then begin
      Result := ReportFailure('Crew installs for the current user and never closes or restarts running applications. Remove the conflicting command-line option.');
      Exit;
    end;
  end;
end;

function NormalPath(const Value: String): String;
begin
  Result := RemoveBackslashUnlessRoot(ExpandFileName(Value));
end;

function HasEntries(const Directory: String): Boolean;
var
  Entry: TFindRec;
begin
  Result := False;
  if FindFirst(AddBackslash(Directory) + '*', Entry) then begin
    try
      repeat
        if (Entry.Name <> '.') and (Entry.Name <> '..') then begin
          Result := True;
          Exit;
        end;
      until not FindNext(Entry);
    finally
      FindClose(Entry);
    end;
  end;
end;

function VersionPart(var Value: String): Integer;
var
  Dot: Integer;
  Part: String;
begin
  Dot := Pos('.', Value);
  if Dot = 0 then begin
    Part := Value;
    Value := '';
  end else begin
    Part := Copy(Value, 1, Dot - 1);
    Delete(Value, 1, Dot);
  end;
  Result := StrToIntDef(Part, -1);
end;

function VersionAllowed(Previous, Candidate: String): Boolean;
var
  I, OldPart, NewPart, Comparison: Integer;
begin
  Result := False;
  Comparison := 0;
  for I := 1 to 3 do begin
    OldPart := VersionPart(Previous);
    NewPart := VersionPart(Candidate);
    if (OldPart < 0) or (NewPart < 0) or (OldPart > 65535) or (NewPart > 65535) then Exit;
    if Comparison = 0 then begin
      if NewPart > OldPart then Comparison := 1;
      if NewPart < OldPart then Comparison := -1;
    end;
  end;
  Result := (Previous = '') and (Candidate = '') and (Comparison >= 0);
end;

function LockedPayload: String;
var
  I: Integer;
  Names: TArrayOfString;
  Path: String;
  Probe: TFileStream;
begin
  Result := '';
  SetArrayLength(Names, 5);
  Names[0] := 'crew.exe';
  Names[1] := 'tmuxlike.exe';
  Names[2] := 'README.md';
  Names[3] := 'LICENSE';
  Names[4] := 'manifest.json';
  for I := 0 to GetArrayLength(Names) - 1 do begin
    Path := AddBackslash(ExpandConstant('{app}')) + Names[I];
    if FileExists(Path) then begin
      try
        { Open existing data without truncation. Running executable images deny writes. }
        Probe := TFileStream.Create(Path, $0002 or $0010);
        Probe.Free;
      except
        Result := 'Crew cannot update or remove ' + Path + '. Close Crew and its daemon yourself, or correct file permissions, then retry. No process was stopped.';
        Log(Result);
        Exit;
      end;
    end;
  end;
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  PreviousDir, PreviousVersion: String;
begin
  Result := '';
  NeedsRestart := False;
  if RegKeyExists(HKCU, UninstallKey) then begin
    if not RegQueryStringValue(HKCU, UninstallKey, 'InstallLocation', PreviousDir) or
       (CompareText(NormalPath(PreviousDir), NormalPath(ExpandConstant('{app}'))) <> 0) then begin
      Result := 'Upgrade Crew in its existing directory. To move it, uninstall the existing program first; user settings and data are preserved.';
      Exit;
    end;
    if not RegQueryStringValue(HKCU, UninstallKey, 'DisplayVersion', PreviousVersion) or
       not VersionAllowed(PreviousVersion, '{#CrewVersion}') then begin
      Result := 'An older installer cannot replace this Crew version. Uninstall the program first if you intentionally need an older version; user data is preserved.';
      Exit;
    end;
  end else if HasEntries(ExpandConstant('{app}')) then begin
    Result := 'The destination contains existing files and is not a registered Crew installation. Select an empty directory; existing files will not be overwritten.';
    Exit;
  end;
  Result := LockedPayload;
end;

function InitializeUninstall: Boolean;
var
  Failure: String;
begin
  Failure := LockedPayload;
  Result := Failure = '';
  if not Result then ReportFailure(Failure);
end;
