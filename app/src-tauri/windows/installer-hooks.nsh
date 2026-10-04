; FrameForge NSIS installer hooks (Tauri v2)
;
; PresentMon counts frames through Windows Event Tracing. Windows only lets
; members of the built-in "Performance Log Users" group (SID S-1-5-32-559)
; start such a trace without administrator rights. We add the installing user
; once, so FrameForge itself never has to run as administrator.
; The SID is used instead of the group name because the name is translated on
; non-English Windows.

!macro NSIS_HOOK_POSTINSTALL
  DetailPrint "Allowing FrameForge to count frames (Performance Log Users)…"
  nsExec::ExecToLog 'powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command "try { Add-LocalGroupMember -SID S-1-5-32-559 -Member ([System.Security.Principal.WindowsIdentity]::GetCurrent().Name) -ErrorAction Stop } catch { if ($$_.FullyQualifiedErrorId -notlike \"*MemberExists*\") { exit 1 } }"'
  Pop $0
  ${If} $0 != 0
    DetailPrint "Could not add the user to Performance Log Users (code $0). FrameForge will explain how to fix this if needed."
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ; Make sure no FrameForge capture session is left running.
  nsExec::Exec 'logman stop FrameForgeCapture -ets'
!macroend
