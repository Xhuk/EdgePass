; Hooks del instalador NSIS de edgePass.
;
; Por qué existe este archivo: el widget crea una tarea programada al logon
; (nombre `edgePass`, ver src-tauri/src/autostart.rs). El desinstalador de NSIS
; borra el ejecutable pero no sabe nada de esa tarea. Si queda apuntando a un exe
; que ya no existe, Windows intenta lanzarla en cada logon y falla en silencio.
;
; Se usa `Exec` (core de NSIS) y no `nsExec::Exec` para no depender de un plugin
; del bundler. `schtasks` es parte de Windows.

!macro NSIS_HOOK_PREUNINSTALL
  DetailPrint "Quitando la tarea de arranque al logon (edgePass)..."
  Exec 'schtasks.exe /Delete /TN "edgePass" /F'
!macroend
