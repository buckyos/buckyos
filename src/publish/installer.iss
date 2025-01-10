#define MyAppName "BuckyOS"
#define MyAppPublisher "Buckyos"
#define MyAppURL "https://github.com/buckyos"

[Setup]
AppId={{7EB09290-A787-4E01-AD45-D2453632223F}}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}
AppUpdatesURL={#MyAppURL}
DefaultDirName=C:\{#MyAppName}
UsePreviousAppDir=no
DisableProgramGroupPage=yes
OutputDir=.
OutputBaseFilename=buckyos-installer-{#MyAppVersion}
Compression=lzma
SolidCompression=yes
WizardStyle=modern

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Files]
Source: ".\rootfs\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{group}\{cm:UninstallProgram,{#MyAppName}}"; Filename: "{uninstallexe}"

[Registry]
Root: HKLM; Subkey: "SYSTEM\CurrentControlSet\Control\Session Manager\Environment"; ValueType: string; ValueName: "BUCKYOS_ROOT"; ValueData: "{app}"; Flags: preservestringtype uninsdeletevalue

[Run]
Filename: "sc.exe"; Parameters: "create buckyos start=auto binPath=""{app}\bin\node_daemon.exe --as_win_srv --enable_active"""; Flags: runhidden; StatusMsg: "Creating BuckyOS service..."
Filename: "sc.exe"; Parameters: "failure buckyos reset=3600 actions=restart/5000/restart/10000"; Flags: runhidden; StatusMsg: "Setting BuckyOS service failure actions..."
Filename: "sc.exe"; Parameters: "start buckyos"; Flags: runhidden; StatusMsg: "Starting BuckyOS service..."

[uninstallRun]
Filename: "sc.exe"; Parameters: "stop buckyos"; Flags: runhidden; RunOnceId: "StopService"; StatusMsg: "Stopping BuckyOS service..."
Filename: "python.exe"; Parameters: "{app}\bin\killall.py"; Flags: runhidden; RunOnceId: "StopProcesses"; StatusMsg: "Killing BuckyOS processes..."
Filename: "sc.exe"; Parameters: "delete buckyos"; Flags: runhidden; RunOnceId: "DeleteService"; StatusMsg: "Deleting BuckyOS service..."

[Code]
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
    if CurUninstallStep = usPostUninstall then
    begin
        if MsgBox('Do you want to delete your data and identity?', mbConfirmation, MB_YESNO) = IDYES then
            begin
                DelTree('{app}', True, True, True);
            end;
    end
end;

function IsPythonInstalled(): Boolean;
var
    ResultCode: Integer;
begin
    Result := Exec('python.exe', '--version', '', SW_HIDE, ewWaitUntilTerminated, ResultCode) and (ResultCode = 0);
end;

procedure CurStepChanged(CurInstallStep: TSetupStep);
var
    BuckyRoot: String;
    ResultCode: Integer;
begin
    if CurInstallStep = ssInstall then
    begin
        if RegQueryStringValue(HKLM, 'SYSTEM\CurrentControlSet\Control\Session Manager\Environment', 'BUCKYOS_ROOT', BuckyRoot) then
        begin
            Exec('sc.exe', 'stop buckyos', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
            Exec('python.exe', '%BUCKYOS_ROOT%\bin\killall.py', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
            Exec('sc.exe', 'delete buckyos', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
        end;
    end;
    
end;

function InitializeSetup(): Boolean;
var
    ResultCode: Integer;
begin
    if not IsPythonInstalled() then
    begin
        if MsgBox('Python is not installed. Do you want to download it?', mbConfirmation, MB_YESNO) = IDYES then
        begin
            ShellExec('', 'https://www.python.org/downloads/', '', '', SW_SHOWNORMAL, ewNoWait, ResultCode);
        end;
        Result := False;
        Exit;
    end;

    Result := True;
end;