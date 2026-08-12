Option Explicit

Dim shell
Dim fso
Dim scriptDir
Dim rootDir
Dim nodeDaemonPath
Dim command

Set shell = CreateObject("WScript.Shell")
Set fso = CreateObject("Scripting.FileSystemObject")

scriptDir = fso.GetParentFolderName(WScript.ScriptFullName)
rootDir = fso.GetParentFolderName(scriptDir)

If WScript.Arguments.Count > 0 Then
  nodeDaemonPath = WScript.Arguments(0)
Else
  nodeDaemonPath = fso.BuildPath(rootDir, "bin\node-daemon\node_daemon.exe")
End If

If IsNodeDaemonRunning() Then
  WScript.Quit 0
End If

If Not fso.FileExists(nodeDaemonPath) Then
  WScript.Quit 1
End If

command = Quote(nodeDaemonPath) & " --enable_active"

On Error Resume Next
shell.CurrentDirectory = fso.GetParentFolderName(nodeDaemonPath)
shell.Run command, 0, False
If Err.Number <> 0 Then
  WScript.Quit 1
End If
On Error GoTo 0

WScript.Quit 0

Function IsNodeDaemonRunning()
  Dim service
  Dim processes

  On Error Resume Next
  Set service = GetObject("winmgmts:\\.\root\cimv2")
  If Err.Number <> 0 Then
    Err.Clear
    IsNodeDaemonRunning = False
    Exit Function
  End If

  Set processes = service.ExecQuery("SELECT ProcessId FROM Win32_Process WHERE Name = 'node_daemon.exe'")
  If Err.Number <> 0 Then
    Err.Clear
    IsNodeDaemonRunning = False
    Exit Function
  End If
  On Error GoTo 0

  IsNodeDaemonRunning = (processes.Count > 0)
End Function

Function Quote(value)
  Quote = """" & value & """"
End Function
