Option Explicit

Dim shell
Dim fso
Dim scriptDir
Dim rootDir
Dim psScript
Dim nodeDaemonPath
Dim command

Set shell = CreateObject("WScript.Shell")
Set fso = CreateObject("Scripting.FileSystemObject")

scriptDir = fso.GetParentFolderName(WScript.ScriptFullName)
rootDir = fso.GetParentFolderName(scriptDir)
psScript = fso.BuildPath(scriptDir, "node_daemon_loader.ps1")

If WScript.Arguments.Count > 0 Then
  nodeDaemonPath = WScript.Arguments(0)
Else
  nodeDaemonPath = fso.BuildPath(rootDir, "bin\node-daemon\node_daemon.exe")
End If

command = "powershell.exe -NoProfile -NonInteractive -WindowStyle Hidden -ExecutionPolicy Bypass -File " _
  & Quote(psScript) & " -NodeDaemonPath " & Quote(nodeDaemonPath)

shell.Run command, 0, False

Function Quote(value)
  Quote = """" & value & """"
End Function
