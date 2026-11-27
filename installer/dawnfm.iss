; Inno Setup script for Dawn FM.
; Build: cargo build --release, then
;   iscc installer\dawnfm.iss
; Output: installer\Output\dawnfm-setup-v<version>.exe
; ARM64: cargo build --release --target aarch64-pc-windows-msvc, then
;   iscc /DArm64 installer\dawnfm.iss  ->  dawnfm-setup-v<version>-arm64.exe

#define MyAppName "Dawn FM"
#define MyAppVersion "2.6.0"
#define MyAppPublisher "Dawn FM Team"
#define MyAppURL "https://github.com/wenujacodes/DawnFM"
#define MyAppExeName "dawnfm.exe"

#ifdef Arm64
  #define Arch "arm64"
  #define Suffix "-arm64"
  #define BinDir "..\target\aarch64-pc-windows-msvc\release"
#else
  #define Arch "x64compatible"
  #define Suffix ""
  #define BinDir "..\target\release"
#endif

[Setup]
AppId={{9B73C488-8422-4B1D-8FE9-DAWNFM2024}}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}
AppUpdatesURL={#MyAppURL}
DefaultDirName={autopf}\{#MyAppName}
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed={#Arch}
ArchitecturesInstallIn64BitMode={#Arch}
MinVersion=10.0
OutputDir=..\dist
OutputBaseFilename=dawnfm-setup-v{#MyAppVersion}{#Suffix}
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern
SetupIconFile=..\assets\icon\icon.ico
UninstallDisplayIcon={app}\{#MyAppExeName}
CloseApplications=yes

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "..\target\release\{#MyAppExeName}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\target\release\WebView2Loader.dll"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist

[Icons]
Name: "{group}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"
Name: "{group}\Uninstall {#MyAppName}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; Tasks: desktopicon

[Registry]
; ytm-rs:// links (jam invites). HKA is HKCU for per-user installs, HKLM for all-users.
; Also registers HKCR protocol handler with proper path escaping.
Root: HKA; Subkey: "Software\Classes\ytm-rs"; ValueType: string; ValueName: ""; ValueData: "URL:ytm-rs Protocol"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\ytm-rs"; ValueType: string; ValueName: "URL Protocol"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\ytm-rs\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: """{app}\{#MyAppExeName}"",0"
Root: HKA; Subkey: "Software\Classes\ytm-rs\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#MyAppExeName}"" ""%1"""

[UninstallDelete]
Type: filesandordirs; Name: "{userappdata}\dawnfm\data\updates"

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "{cm:LaunchProgram,{#MyAppName}}"; Flags: nowait postinstall skipifsilent

[Code]
// WebView2 Bootstrapper check
function NeedsWebView2(): Boolean;
var
  InstalledVersion: String;
begin
  // Check if WebView2 runtime is installed via registry
  Result := not RegQueryStringValue(HKEY_LOCAL_MACHINE,
    'SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-2347-4E5A-A045-8B264E54DA95}',
    'pv', InstalledVersion);
end;
