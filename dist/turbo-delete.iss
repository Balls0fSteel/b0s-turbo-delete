; Inno Setup script for Turbo Delete.
;
; Build from the repository root after `cargo build --release` has been copied
; to bin\td.exe (automate-release.py and the release workflow do both):
;
;     iscc dist\turbo-delete.iss
;
; The installer is written to bin\turbo-delete-setup.exe. It installs for the
; current user only (no administrator rights), adds the install folder to the
; user's PATH, and adds "Turbo Delete" to the folder context menu in Explorer.

#define MyAppName "Turbo Delete"
; Keep in sync with `version` in Cargo.toml (the release workflow checks it).
#define MyAppVersion "1.0.0"
#define MyAppPublisher "XtremeDevX"
#define MyAppURL "https://github.com/Balls0fSteel/b0s-turbo-delete"

[Setup]
; NOTE: The value of AppId uniquely identifies this application.
; Do not use the same AppId value in installers for other applications.
AppId={{033D0649-03A3-4BF3-8700-4F054EC32FF6}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}
; All paths below are relative to the repository root.
SourceDir=..
DefaultDirName={localappdata}\Programs\turbo-delete
DisableDirPage=yes
DisableProgramGroupPage=yes
LicenseFile=LICENSE.txt
OutputDir=bin
OutputBaseFilename=turbo-delete-setup
UninstallDisplayName={#MyAppName}
UninstallDisplayIcon={app}\td.exe
Compression=lzma2
SolidCompression=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ChangesEnvironment=yes

[Messages]
SetupWindowTitle=Turbo Delete Setup

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Files]
Source: "bin\td.exe"; DestDir: "{app}"; Flags: ignoreversion

[Registry]
; Folder context menu entry, for the current user only. Explorer passes the
; folder as %1; --yes because choosing the menu entry is the confirmation.
Root: HKCU; Subkey: "Software\Classes\Directory\shell\turbo-delete"; ValueType: string; ValueName: ""; ValueData: "Turbo Delete"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\Directory\shell\turbo-delete\command"; ValueType: string; ValueName: ""; ValueData: """{app}\td.exe"" --yes ""%1"""

; environment.iss is a [Code] section, so it must be included here: anything
; that follows it is compiled as Pascal Script.
#include "environment.iss"

[Code]
procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then
    EnvAddPath(ExpandConstant('{app}'));
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usPostUninstall then
    EnvRemovePath(ExpandConstant('{app}'));
end;
