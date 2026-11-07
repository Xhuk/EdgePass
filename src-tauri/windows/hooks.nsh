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
  ; El token vive en el Credential Manager (keyring: servicio edgepass, usuario
  ; bao_token), no en un archivo: el desinstalador tiene que borrarlo o queda una
  ; credencial de un vault que ya no está en la máquina.
  DetailPrint "Quitando el token de OpenBao del Credential Manager..."
  Exec 'cmdkey.exe /delete:bao_token.edgepass'
!macroend
