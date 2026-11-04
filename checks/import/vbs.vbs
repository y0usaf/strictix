Option Explicit

Dim shell
Set shell = CreateObject("WScript.Shell")
If WScript.Arguments.Count > 0 Then
	shell.Run WScript.Arguments(0), 0, False
End If
