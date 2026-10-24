; The app is also the `cranibox` command. A GUI program started from a terminal does
; not hold the prompt, so PATH gets bin\cranibox.cmd: cmd waits for programs that a
; batch file starts, and the app attaches to that terminal for its output.
; The directory reaches PowerShell through the environment, so no path is quoted.

!macro CRANIBOX_PATH OPERATION
  System::Call 'Kernel32::SetEnvironmentVariable(t "CRANIBOX_BIN", t "$INSTDIR\bin")'
  nsExec::Exec `powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command "$$d = $$env:CRANIBOX_BIN; $$k = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey('Environment'); $$p = [string]$$k.GetValue('Path', '', 'DoNotExpandEnvironmentNames'); $$parts = @($$p -split ';' | Where-Object { $$_ -and $$_ -ne $$d }); if ('${OPERATION}' -eq 'add') { $$parts += $$d }; $$k.SetValue('Path', ($$parts -join ';'), 'ExpandString'); [Environment]::SetEnvironmentVariable('CRANIBOX_PATH_CHANGED', $$null, 'User')"`
  Pop $0
!macroend

!macro NSIS_HOOK_POSTINSTALL
  CreateDirectory "$INSTDIR\bin"
  FileOpen $0 "$INSTDIR\bin\cranibox.cmd" w
  FileWrite $0 `@"%~dp0..\cranibox.exe" %*$\r$\n`
  FileClose $0
  !insertmacro CRANIBOX_PATH add
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  !insertmacro CRANIBOX_PATH remove
  Delete "$INSTDIR\bin\cranibox.cmd"
  RMDir "$INSTDIR\bin"
!macroend
