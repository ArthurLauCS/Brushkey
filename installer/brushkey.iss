; Brushkey installer (Inno Setup 6). Built by scripts\installer.ps1, which compiles both DLLs, the
; settings app and the dictionary first and passes the version.
; Installs to Program Files and needs admin: registering the IME writes HKLM.
; Every app that takes text has the DLL and the dictionary loaded, so they can't be overwritten,
; only renamed: install and uninstall first rename the old files to .old. New processes load the
; new files, running ones switch when restarted; no "close applications" page, no reboot.

#ifndef Version
  #define Version "0.0.0"
#endif

[Setup]
AppId=Brushkey
AppName=Brushkey
AppVersion={#Version}
AppVerName=Brushkey {#Version}
AppPublisher=ArthurLau
AppPublisherURL=https://github.com/ArthurLauCS/Brushkey
DefaultDirName={autopf}\Brushkey
DisableProgramGroupPage=yes
PrivilegesRequired=admin
; x64 and x86 DLLs only: native ARM64 apps couldn't load them
ArchitecturesAllowed=x64os
ArchitecturesInstallIn64BitMode=x64os
MinVersion=10.0
LicenseFile=..\LICENSE
OutputDir=..\target\installer
OutputBaseFilename=Brushkey-Setup-{#Version}
SetupIconFile=..\assets\app.ico
UninstallDisplayIcon={app}\brushkey-settings.exe
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
; files in use are replaced by renaming (see [Code]), not by closing every app that types
CloseApplications=no

[Languages]
Name: "zh"; MessagesFile: "ChineseSimplified.isl"
Name: "en"; MessagesFile: "compiler:Default.isl"

[Messages]
zh.FinishedLabel=已安装 [name]。按 Win+空格 切换到“Brushkey 拼音”。设置在开始菜单的“Brushkey 设置”里，右键任务栏托盘里的 简/繁/英 也能打开。
en.FinishedLabel=Setup has installed [name]. Press Win+Space to switch to Brushkey. Open "Brushkey Settings" from the Start menu, or right-click the mode button in the taskbar tray, to change settings.

[CustomMessages]
zh.Settings=Brushkey 设置
en.Settings=Brushkey Settings
zh.DeleteData=同时删除学过的词和设置吗？%n%n删掉后要重新学词。重装时保留它们可以接着用。
en.DeleteData=Also delete the learned words and settings?%n%nOnce deleted, Brushkey has to learn your words again. Keep them if you plan to reinstall.

[Files]
Source: "..\target\release\brushkey.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\target\i686-pc-windows-msvc\release\brushkey.dll"; DestDir: "{app}"; DestName: "brushkey32.dll"; Flags: ignoreversion
Source: "..\target\release\brushkey-settings.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\target\dict.bin"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion
Source: "..\data\opencc\LICENSE"; DestDir: "{app}"; DestName: "LICENSE-OpenCC.txt"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{cm:Settings}"; Filename: "{app}\brushkey-settings.exe"

[Run]
; 64-bit and 32-bit apps each load their own DLL
Filename: "{sys}\regsvr32.exe"; Parameters: "/s ""{app}\brushkey.dll"""; Flags: runhidden
Filename: "{syswow64}\regsvr32.exe"; Parameters: "/s ""{app}\brushkey32.dll"""; Flags: runhidden

[Code]
// Renames the files to .old, then deletes every .old it can. One still loaded somewhere stays:
// install leaves it to the next install, uninstall queues it for deletion at reboot.
procedure MoveAside(Uninstall: Boolean);
var
  Dir, Old: String;
  R: TFindRec;
begin
  Dir := ExpandConstant('{app}\');
  Old := '.' + GetDateTimeString('yyyymmddhhnnss', #0, #0) + '.old';
  RenameFile(Dir + 'brushkey.dll', Dir + 'brushkey.dll' + Old);
  RenameFile(Dir + 'brushkey32.dll', Dir + 'brushkey32.dll' + Old);
  RenameFile(Dir + 'brushkey-settings.exe', Dir + 'brushkey-settings.exe' + Old);
  RenameFile(Dir + 'dict.bin', Dir + 'dict.bin' + Old);
  if FindFirst(Dir + '*.old', R) then
  try
    repeat
      if not DeleteFile(Dir + R.Name) and Uninstall then
        RestartReplace(Dir + R.Name, '');
    until not FindNext(R);
  finally
    FindClose(R);
  end;
end;

procedure Unregister(RegSvr, Dll: String);
var
  Code: Integer;
begin
  Exec(ExpandConstant(RegSvr), '/s /u "' + ExpandConstant('{app}\') + Dll + '"', '', SW_HIDE, ewWaitUntilTerminated, Code);
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssInstall then
    MoveAside(False);
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usUninstall then
  begin
    // unregister first so new processes stop loading it; regsvr32 loads the DLL, so rename after
    Unregister('{sys}\regsvr32.exe', 'brushkey.dll');
    Unregister('{syswow64}\regsvr32.exe', 'brushkey32.dll');
    MoveAside(True);
    // queued after the .old files: at reboot the folder is empty by the time it's deleted
    RestartReplace(ExpandConstant('{app}'), '');
  end;
  // a silent uninstall takes the default No and keeps the data
  if CurUninstallStep = usPostUninstall then
    if SuppressibleMsgBox(CustomMessage('DeleteData'), mbConfirmation, MB_YESNO or MB_DEFBUTTON2, IDNO) = IDYES then
      DelTree(ExpandConstant('{userappdata}\Brushkey'), True, True, True);
end;
