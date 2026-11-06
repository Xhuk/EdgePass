# Verifica la cadena MCP de punta a punta, sin abrir la app.
#
# Uso: .\verify-mcp.ps1 [-Id reasonixmcp] [-Port 8788]
#      .\verify-mcp.ps1 -PublicUrl https://host.tailnet.ts.net:8443
#
# Existe porque la cadena tiene cinco eslabones y fallar en cualquiera se ve
# igual desde afuera ("no responde"): la fila guardada, el proceso vivo, el
# puerto escuchando, el túnel alcanzándolo y el endpoint contestando 401.
#
# El origen público sale del `public_url` del toml local (una sola fuente de
# verdad: si no es el mismo que usa el cliente, los documentos de discovery
# mienten). El túnel se acepta en las tres formas que tuvo esta cadena: un
# **túnel nombrado** de Cloudflare (hostname fijo, la forma buena), un **quick
# tunnel** (`cloudflared --url`, hostname al azar), o un Funnel de Tailscale.

param(
  [string]$Id = "reasonixmcp",
  [int]$Port = 8788,
  [string]$PublicUrl = ""
)

$ErrorActionPreference = "Continue"
$pass = 0
$fail = 0

function Step {
  param([string]$Name, [bool]$Ok, [string]$Detail)
  if ($Ok) { $script:pass++ } else { $script:fail++ }
  $mark = if ($Ok) { "OK  " } else { "FALLA" }
  Write-Host ("  [{0}] {1}" -f $mark, $Name)
  if ($Detail) { Write-Host ("         {0}" -f $Detail) }
}

# El origen que el server anuncia: `public_url` del toml del repo.
function Read-PublicUrl {
  $toml = Join-Path (Split-Path -Parent $PSScriptRoot) "mcp\reasonixmcp\reasonixmcp.toml"
  if (-not (Test-Path $toml)) { return "" }
  $found = Select-String -Path $toml -Pattern '^\s*public_url\s*=\s*"([^"]+)"' | Select-Object -First 1
  if ($found) { return $found.Matches[0].Groups[1].Value.TrimEnd('/') }
  return ""
}

# ¿Hay un cloudflared con `--url http://127.0.0.1:<puerto>` en su línea de comandos?
# (quick tunnel: hostname al azar, distinto en cada arranque)
function Test-Cloudflared([int]$p) {
  $pattern = "--url\s+https?://(127\.0\.0\.1|localhost):$p(\s|$)"
  foreach ($proc in @(Get-CimInstance Win32_Process -Filter "Name='cloudflared.exe'" -ErrorAction SilentlyContinue)) {
    if ($proc.CommandLine -match $pattern) { return $true }
  }
  return $false
}

# ¿Hay un conector de túnel NOMBRADO (`cloudflared tunnel run <nombre>`) cuyo
# config.yml rutee a este puerto? Es la forma estable: el hostname no cambia.
function Get-NamedTunnel([int]$p) {
  $running = $false
  foreach ($proc in @(Get-CimInstance Win32_Process -Filter "Name='cloudflared.exe'" -ErrorAction SilentlyContinue)) {
    if ($proc.CommandLine -match 'tunnel\s+run') { $running = $true }
  }
  if (-not $running) { return "" }
  $cfg = Join-Path $env:USERPROFILE ".cloudflared\config.yml"
  if (-not (Test-Path $cfg)) { return "corriendo, sin config.yml en ~\.cloudflared" }
  $raw = Get-Content $cfg -Raw
  if ($raw -match "(?m)^\s*service:\s*https?://(127\.0\.0\.1|localhost):$p\s*$") {
    $host_ = if ($raw -match '(?m)^\s*-?\s*hostname:\s*(\S+)') { $Matches[1] } else { "" }
    return $host_
  }
  return ""
}

Write-Host ""
Write-Host "edgePass MCP chain - servidor '$Id'"
Write-Host "-----------------------------------------"

# 1. La fila esta guardada, y con autostart.
$cfgPath = Join-Path $env:APPDATA "edgePass\config.json"
$row = $null
if (Test-Path $cfgPath) {
  try {
    $cfg = Get-Content $cfgPath -Raw | ConvertFrom-Json
    $row = @($cfg.mcpServers) | Where-Object { $_.id -eq $Id } | Select-Object -First 1
  } catch {
    Step "config.json legible" $false "no pude parsearlo: $($_.Exception.Message)"
  }
}
Step "fila '$Id' guardada en config.json" ($null -ne $row) $(if ($row) {
    "command=$($row.command) autostart=$($row.autostart)"
  } else {
    "abri el tab MCP, cargala y dale a Guardar (comando, directorio, argumentos, auto)"
  })

# 2 y 3. El proceso, deducido de la propia fila (no de un nombre fijo).
if ($row) {
  $image = if ($row.image) { $row.image } else { (Split-Path -Leaf $row.command) }
  if ($image -and -not $image.Contains(".")) { $image = "$image.exe" }
  $proc = Get-Process -Name ([IO.Path]::GetFileNameWithoutExtension($image)) -ErrorAction SilentlyContinue
  Step "proceso '$image' vivo" ($null -ne $proc) $(if ($proc) { "pid $($proc.Id -join ', ')" } else { "edgePass lo arranca si autostart esta marcado y la app arranco con esa fila guardada" })
} else {
  Step "proceso del servidor vivo" $false "sin fila no hay comando que mirar; el paso 1 es el que falta"
}

$listening = Get-NetTCPConnection -LocalPort $Port -State Listen -ErrorAction SilentlyContinue
Step "puerto $Port escuchando" ($null -ne $listening) $(if ($listening) { "pid $($listening[0].OwningProcess)" } else { "nadie escucha: o el server no arranco, o escucha en otro puerto" })

# 4. El túnel tiene que seguir apuntando al puerto.
$namedHost = Get-NamedTunnel $Port
if ($namedHost) {
  Step "túnel nombrado rutea a 127.0.0.1:$Port" $true $(if ($namedHost -like '*.*') { "hostname $namedHost (el public_url deberia ser https://$namedHost)" } else { $namedHost })
} elseif (Test-Cloudflared $Port) {
  Step "cloudflared apunta a 127.0.0.1:$Port" $true "quick tunnel vivo: el hostname cambia en cada arranque, así que el public_url del toml y el registro del cliente se rompen al reiniciar"
} else {
  $ts = "C:\Program Files\Tailscale\tailscale.exe"
  if (Test-Path $ts) {
    $funnel = & $ts funnel status 2>&1 | Out-String
    $mapped = $funnel -match [regex]::Escape("127.0.0.1:$Port")
    $funnelHost = if ($funnel -match 'https://([a-z0-9.-]+\.ts\.net)') { $Matches[1] } else { "" }
    Step "Funnel apunta a 127.0.0.1:$Port" $mapped $(if ($mapped) { "ok, pero Gemini rechaza un FQDN *.ts.net:8443 (usa un túnel nombrado de Cloudflare)" } else { "ni túnel nombrado ni cloudflared --url ni el Funnel: dale al LED MCP del widget (arranca server + túnel), o corré cloudflared tunnel run <nombre>" })
  } else {
    Step "túnel apuntando a 127.0.0.1:$Port" $false "no hay conector de cloudflared ni tailscale.exe: dale al LED MCP del widget (arranca server + túnel) o corré cloudflared tunnel run <nombre>"
  }
}

# 5. El endpoint, desde internet. 401 es el resultado correcto: vivo y protegido.
#
# El probe va por POST, que es el método del transporte: `GET /mcp` contesta
# **405** con cuerpo JSON-RPC a proposito (leer eso como "sin auth" fue el
# tropiezo #3 del vault), asi que probar con GET medía lo que no era.
if (-not $PublicUrl) {
  $PublicUrl = Read-PublicUrl
  if ($PublicUrl) {
    Write-Host ""
    Write-Host "  origen: public_url del toml -> $PublicUrl"
  } else {
    Step "public_url en el toml" $false "no lo encontre en mcp\reasonixmcp\reasonixmcp.toml: pasalo con -PublicUrl"
  }
}
$url = "$($PublicUrl.TrimEnd('/'))/mcp"
$body = '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"verify-mcp","version":"1"}}}'
$headers = @(& curl.exe -s -D - -o NUL --max-time 12 -X POST -H "Content-Type: application/json" -d $body $url 2>$null)
$code = $headers | Select-String -Pattern '^HTTP/[0-9.]+ (\d+)' | Select-Object -First 1
$codeText = if ($code) { $code.Matches[0].Groups[1].Value } else { "000" }
$challenge = @($headers | Select-String -Pattern '^WWW-Authenticate:' | Select-Object -First 1).Count -gt 0
$verdict = switch ($codeText) {
  "401" { if ($challenge) { "vivo y protegido, con el desafio del recurso: esto es lo que Gemini necesita ver" } else { "401 pero sin WWW-Authenticate: mira el gate del endpoint" } }
  "405" { "el POST contesta 405: ahi no hay (o no es) el transporte Streamable HTTP" }
  "502" { "el tunel esta, pero no hay nada escuchando del otro lado (mira el paso del puerto)" }
  "200" { "contesta sin auth: revisa que no hayas arrancado con -dev-token" }
  "000" {
    $host_ = ([uri]$url).Host
    $resuelve = $null -ne (Resolve-DnsName $host_ -ErrorAction SilentlyContinue)
    if (-not $resuelve) {
      # Un NXDOMAIN cacheado en el resolutor local (pasa cuando el hostname se
      # acaba de crear: la prueba anterior dejo el negativo en cache) se ve
      # igual que "no responde". No es la cadena: es esta maquina.
      "no resuelve '$host_' desde esta maquina (DNS cacheado o el hostname no existe): proba ipconfig /flushdns y de nuevo"
    } elseif ($funnelHost) {
      # El caso que más veces nos costó: el túnel vivo es el Funnel de Tailscale y
      # el public_url apunta a otro origen. Gemini rechaza un FQDN *.ts.net:8443
      # (INVALID_ARGUMENT / bard_error_code 1179), así que esta cadena no sirve
      # aunque "haya túnel".
      "no se pudo conectar. El túnel vivo es el Funnel de Tailscale (${funnelHost}:8443) y el public_url apunta a otro origen: para Gemini hay que levantar el túnel nombrado y actualizar el public_url del toml"
    } else {
      "no se pudo conectar (sin internet, o el tunnel caido)"
    }
  }
  default { "inesperado" }
}
Step "$url responde 401 al POST" ($codeText -eq "401" -and $challenge) "$codeText - $verdict"

# 6. Que dice edgePass de los arranques MCP.
$log = Join-Path $env:APPDATA "edgePass\logs\edgepass.log"
if (Test-Path $log) {
  $lines = Select-String -Path $log -Pattern "MCP" | Select-Object -Last 4
  if ($lines) {
    Write-Host ""
    Write-Host "  edgePass dijo:"
    $lines | ForEach-Object { Write-Host "    $($_.Line)" }
  }
}

Write-Host "-----------------------------------------"
if ($fail -eq 0) {
  Write-Host "CADENA COMPLETA: $pass/$($pass + $fail) - el comite puede alcanzar '$Id'."
} else {
  Write-Host "$pass OK, $fail por resolver (ver arriba)."
  Write-Host "Recorda: la persistencia real se prueba cerrando edgePass y volviendola a abrir"
  Write-Host "(autostart corre en el setup) o cerrando sesion, que es cuando la tarea 'edgePass' la lanza."
}
Write-Host ""
