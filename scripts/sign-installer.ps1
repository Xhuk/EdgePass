# Firma del instalador
#
# La firma NO se puede hacer sin un certificado de firma de código. Este script
# existe para que el día que lo tengas sea una orden, no una investigación.
#
# Requisitos:
#   1. Un certificado de firma de código instalado en Cert:\CurrentUser\My.
#      Opciones, de más barata a más formal:
#        - Azure Trusted Signing (~USD 10/mes, requiere cuenta Azure y validación
#          de identidad). Firma en la nube, sin token físico.
#        - Certificado OV (Sectigo, DigiCert, SSL.com). Entre USD 200 y 400 al año.
#          Firma local con el certificado en el almacén de Windows.
#        - Certificado EV. Más caro y con token físico. Da reputación inmediata
#          en SmartScreen, que es lo único que evita la advertencia desde el día uno.
#   2. Windows SDK, que ya trae signtool.exe.
#
# Uso:
#   powershell -File scripts\sign-installer.ps1 -Thumbprint <huella>
#   powershell -File scripts\sign-installer.ps1 -Thumbprint <huella> -Verify
#
# Nunca pases una contraseña por argumento: queda en el historial y en la lista
# de procesos. Si tu certificado es un .pfx con clave, importalo al almacén y
# firmá por huella.

param(
    [Parameter(Mandatory = $true)][string]$Thumbprint,
    [string]$Installer = "src-tauri\target\x86_64-pc-windows-msvc\release\bundle\nsis\edgePass_0.4.1_x64-setup.exe",
    [switch]$Verify
)

$ErrorActionPreference = 'Stop'

$signtool = Get-ChildItem 'C:\Program Files (x86)\Windows Kits\10\bin' -Recurse -Filter signtool.exe -ErrorAction SilentlyContinue |
    Where-Object { $_.FullName -match '\\x64\\' } |
    Select-Object -First 1

if (-not $signtool) { throw 'signtool.exe no está. Instalá el Windows SDK.' }
if (-not (Test-Path $Installer)) { throw "No existe el instalador: $Installer" }

$cert = Get-ChildItem 'Cert:\CurrentUser\My' | Where-Object { $_.Thumbprint -eq $Thumbprint }
if (-not $cert) { throw "No hay certificado con esa huella en Cert:\CurrentUser\My" }

Write-Output ("Certificado: " + $cert.Subject)
Write-Output ("Vence      : " + $cert.NotAfter)

& $signtool.FullName sign /sha1 $Thumbprint /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 $Installer

if ($LASTEXITCODE -ne 0) { throw "signtool falló con código $LASTEXITCODE" }

if ($Verify) {
    & $signtool.FullName verify /pa /v $Installer
}

Write-Output 'Firma aplicada.'
