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
OutputBaseFilename=buckyos-{#AllowArch}-{#MyAppVersion}
Compression=lzma
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed={#AllowArch}
ArchitecturesInstallIn64BitMode={#AllowArch}

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Files]
Source: ".\rootfs\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: ".\vcredist_x64.exe"; DestDir: "{tmp}"; Flags: skipifnewer skipifsourcedoesntexist

[Icons]
Name: "{group}\{cm:UninstallProgram,{#MyAppName}}"; Filename: "{uninstallexe}"

[Registry]
Root: HKLM; Subkey: "SYSTEM\CurrentControlSet\Control\Session Manager\Environment"; ValueType: string; ValueName: "BUCKYOS_ROOT"; ValueData: "{app}"; Flags: preservestringtype uninsdeletevalue

[Run]
Filename: "sc.exe"; Parameters: "create buckyos start=auto binPath=""{app}\bin\node_daemon\node_daemon.exe --as_win_srv --enable_active"""; Flags: runhidden; StatusMsg: "Creating BuckyOS service..."
Filename: "sc.exe"; Parameters: "failure buckyos reset=3600 actions=restart/5000/restart/10000"; Flags: runhidden; StatusMsg: "Setting BuckyOS service failure actions..."
Filename: "sc.exe"; Parameters: "start buckyos"; Flags: runhidden; StatusMsg: "Starting BuckyOS service..."

[uninstallRun]
Filename: "sc.exe"; Parameters: "stop buckyos"; Flags: runhidden; RunOnceId: "StopService"; StatusMsg: "Stopping BuckyOS service..."
Filename: "python.exe"; Parameters: "{app}\bin\killall.py"; Flags: runhidden; RunOnceId: "StopProcesses"; StatusMsg: "Killing BuckyOS processes..."
Filename: "sc.exe"; Parameters: "delete buckyos"; Flags: runhidden; RunOnceId: "DeleteService"; StatusMsg: "Deleting BuckyOS service..."

[Code]
function GetMissingCrtDlls: String;
var
    Missing: String;
    SysDir: String;
begin
    Missing := '';
    SysDir := ExpandConstant('{sys}');

    if not FileExists(SysDir + '\VCRUNTIME140.dll') then
        Missing := Missing + '\r\n- VCRUNTIME140.dll';
    if not FileExists(SysDir + '\VCRUNTIME140_1.dll') then
        Missing := Missing + '\r\n- VCRUNTIME140_1.dll';
    if not FileExists(SysDir + '\api-ms-win-core-synch-l1-2-0.dll') then
        Missing := Missing + '\r\n- api-ms-win-core-synch-l1-2-0.dll';
    if not FileExists(SysDir + '\api-ms-win-crt-math-l1-1-0.dll') then
        Missing := Missing + '\r\n- api-ms-win-crt-math-l1-1-0.dll';
    if not FileExists(SysDir + '\api-ms-win-crt-string-l1-1-0.dll') then
        Missing := Missing + '\r\n- api-ms-win-crt-string-l1-1-0.dll';
    if not FileExists(SysDir + '\api-ms-win-crt-heap-l1-1-0.dll') then
        Missing := Missing + '\r\n- api-ms-win-crt-heap-l1-1-0.dll';
    if not FileExists(SysDir + '\api-ms-win-crt-utility-l1-1-0.dll') then
        Missing := Missing + '\r\n- api-ms-win-crt-utility-l1-1-0.dll';
    if not FileExists(SysDir + '\api-ms-win-crt-time-l1-1-0.dll') then
        Missing := Missing + '\r\n- api-ms-win-crt-time-l1-1-0.dll';
    if not FileExists(SysDir + '\api-ms-win-crt-runtime-l1-1-0.dll') then
        Missing := Missing + '\r\n- api-ms-win-crt-runtime-l1-1-0.dll';
    if not FileExists(SysDir + '\api-ms-win-crt-convert-l1-1-0.dll') then
        Missing := Missing + '\r\n- api-ms-win-crt-convert-l1-1-0.dll';
    if not FileExists(SysDir + '\api-ms-win-crt-stdio-l1-1-0.dll') then
        Missing := Missing + '\r\n- api-ms-win-crt-stdio-l1-1-0.dll';
    if not FileExists(SysDir + '\api-ms-win-crt-locale-l1-1-0.dll') then
        Missing := Missing + '\r\n- api-ms-win-crt-locale-l1-1-0.dll';

    Result := Missing;
end;

function TryInstallVCRedist: Boolean;
var
    ResultCode: Integer;
begin
    if FileExists(ExpandConstant('{tmp}\vcredist_x64.exe')) then
    begin
        if MsgBox('检测到缺失的 Visual C++ 运行库。\r\n是否立即安装 Microsoft Visual C++ 2015-2022 (x64) 再继续？', mbQuestion, MB_YESNO) = IDYES then
        begin
            if not Exec(ExpandConstant('{tmp}\vcredist_x64.exe'), '/quiet /norestart', '', SW_HIDE, ewWaitUntilTerminated, ResultCode) then
            begin
                MsgBox('安装 Visual C++ Redistributable 失败，安装已中止。', mbError, MB_OK);
                Result := False;
                Exit;
            end;

            if ResultCode <> 0 then
            begin
                MsgBox('Visual C++ Redistributable 安装未成功，返回码: ' + IntToStr(ResultCode) + '，安装已中止。', mbError, MB_OK);
                Result := False;
                Exit;
            end;

            MsgBox('运行库已安装，请重启安装程序。', mbInformation, MB_OK);
            Result := False;
            Exit;
        end;
    end
    else
    begin
        MsgBox('检测到缺失的 Visual C++ 运行库。请先安装 Microsoft Visual C++ 2015-2022 (x64) 后重试。', mbError, MB_OK);
    end;

    Result := False;
end;

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
    MissingDlls: String;
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

        MissingDlls := GetMissingCrtDlls();
        if MissingDlls <> '' then
        begin
            if FileExists(ExpandConstant('{tmp}\vcredist_x64.exe')) then
            begin
                if not TryInstallVCRedist() then
                begin
                    Result := False;
                    Exit;
                end;
            end
            else if MsgBox('检测到缺失的运行库：' + MissingDlls + '\r\n\r\n未检测到安装包中的 vcredist_x64.exe。\r\n是否打开微软官方下载页？', mbError, MB_YESNO) = IDYES then
            begin
                ShellExec('', 'https://aka.ms/vs/17/release/vc_redist.x64.exe', '', '', SW_SHOWNORMAL, ewNoWait, ResultCode);
                Result := False;
                Exit;
            end;

            Result := False;
            Exit;
        end;

        Result := True;
end;
