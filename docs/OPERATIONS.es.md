# edgePass

Widget de escritorio (Windows + macOS). **No es un servidor**: es un `.exe` que habla con tu
OpenBao local.

## Comportamiento del vault (Rust)

1. **Cuando necesitas un secreto** (copiar / sync / nuevo) → si OpenBao está sellado, **Rust
   lo desbloquea solo** leyendo las keys del init local (nunca las muestra).
2. **Sellar es opt-in.** Por defecto edgePass **no vuelve a sellar** el vault: sellar corta a
   *todos* sus clientes (otros proyectos, scripts, servicios), así que no se dispara porque el
   widget se quede quieto. Si quieres el comportamiento antiguo, pon `autoSealIdleSecs` > 0 en
   Config; `0` = nunca.
3. Botón **Abrir / Cerrar** en la pastilla para forzar el estado ahora, cuando tú lo decides.
4. **KV v1 o v2, mount por mount.** El índice lee `sys/mounts` en cada namespace y habla las
   rutas de la versión que tenga `kv/` (v2: `data`/`metadata`; v1: el path directo, sin
   envelope). Antes asumía v2 en todas partes: un namespace con el mount en v1 (o un engine
   distinto) no aportaba **ni una fila**, en silencio. El log deja una línea por namespace:
   `kv: namespace <ns> tiene mount 'kv' en KV v1|v2`.

Las configs antiguas que traían `300` se migran a `0` una sola vez al arrancar (queda anotado
en el log).

## Si algo no arranca

Antes de tocar nada, abre el tab **Logs**: dice qué binario está corriendo, con qué fecha de
build, qué versión de WebView2 hay, dónde están el config, la base y el **log**, y las últimas
líneas de ese log. El botón *Copiar informe* lo deja en el portapapeles listo para pegar.

- Log: `%LOCALAPPDATA%\edgePass\logs\edgepass.log` (rota a `.1` a los 4 MB).
- Los errores de arranque y los panics también van ahí: en release no hay consola.

## LLM → formulario de secretos

Con edgePass corriendo, un agente puede **preguntar primero** si el secreto ya existe — solo
nombres, nunca valores, y sin abrir el vault:

```json
{"op":"search","query":"infisi cb2","limit":20}
```

Responde `{ ok, count, hits: [{ namespace, path, key, useCount, lastUsedAt }] }`. Si la
búsqueda no ve algo que sabes que existe, el índice está viejo: `{"op":"sync_index"}` lo
reconstruye (responde `{ ok, count }`) — es el `↻` de la UI, sin abrirla. Si falta de verdad,
pide el formulario (tampoco ve valores):

```powershell
powershell -File scripts\edgepass-ingest-form.ps1 `
  -Namespace buenaidee `
  -Path "projects/cb2/infisical" `
  -Keys DATABASE_URL,DB_USER,DB_PASSWORD,API_TOKEN `
  -Title "CB2 Infisical"
```

Pipe JSON equivalente **al named pipe** `\\.\pipe\edgepass` (no por stdin):

```json
{"op":"ingest-form","namespace":"buenaidee","path":"projects/cb2/infisical","keys":["DATABASE_URL","DB_USER","DB_PASSWORD","API_TOKEN"]}
```

La ventana crece para caber las filas (sin scroll). Tú pegas los valores y guardas el lote.

### Ver los datos de un secreto en el Meter

`search` contesta **qué existe**; `meter` trae los **valores a la pantalla**, que es lo que pides
cuando le dices a un agente «usa edge pass». El valor **no pasa por el pipe**: el op solo nombra
un path, y el widget lo lee con su token y abre el tab **Batch** con las filas ya rellenas.

```json
{"op":"meter","query":"betty"}
```

```json
{"op":"meter","namespace":"prod","path":"projects/app/users"}
```

```powershell
powershell -File scripts\edgepass-meter.ps1 -Query "betty"
```

Responde `{ ok, id, namespace, path }` — nombres, nunca valores. Una `query` solo se abre si
**todos** sus resultados caen en el mismo path; si caen en varios contesta `ambiguo` con la lista
`candidates` en vez de adivinar.

Ya en el panel: cada fila trae **Variable** y **Valor**, con `ver`/`ocultar` para esa fila y `⧉`
para copiar ese valor. El pie tiene `⧉ copiar todo` (el bloque como líneas `KEY=valor`) y `ver
todo`/`ocultar todo`. Los valores se abren visibles y se **auto-ocultan a los 8 s**; el ojo los
vuelve a mostrar. Sigue siendo el editor de siempre: `Enter` o ✓ guarda versión nueva en OpenBao,
«Descartar» no escribe nada.

Por eso un agente que pide un secreto **no acaba con el valor en la mano** (nada que pegar en un
chat, un log o un commit), y el canal remoto (Grok / HTTP) sigue viendo solo nombres. Nada de esto
escribe valores a disco: el log anota el path y cuántos campos, jamás cuáles.

### Ejecutar un comando con secretos

`run` inyecta los secretos en el **entorno del proceso hijo**: nunca se escriben a disco.

```json
{"op":"run","entries":[{"namespace":"buenaidee","path":"projects/cb2/infisical"}],"cmd":["npm","run","migrate"]}
```

Si el comando se cuelga, edgePass lo mata a los `timeoutSecs` (300 por defecto, máx 3600) y
responde `{ ok: false, timeout: true, … }` en vez de dejar al llamante esperando para
siempre. El kill va al **árbol** de procesos (`taskkill /T`): matar solo el `npm.cmd` dejaría
su `node` vivo, con los pipes abiertos.

`cmd[0]` se resuelve por `PATH` + `PATHEXT` antes de lanzarlo, porque Windows solo añade
`.exe` por su cuenta y `npm` es en realidad `npm.cmd`: el ejemplo de arriba funciona tal cual,
sin envolverlo en `cmd /c`.

## Overrides por entorno (no se guardan)

Para una ejecución concreta, sin tocar `config.json`:

| Variable | Efecto |
|---|---|
| `EDGEPASS_AUTO_UNSEAL` | `1`/`true`/`yes`/`on` · `0`/`false`/`no`/`off` |
| `EDGEPASS_AUTO_SEAL_IDLE` | segundos de idle antes de sellar (`0` = nunca) |
| `EDGEPASS_NAMESPACES` | lista separada por comas; `.` = root |

El proceso anota en el log al arrancar qué overrides están en efecto, y **Config nunca los
persiste**: guardar desde la UI escribe lo que está en pantalla, no el valor forzado.

## Uso

- **Teclado**: `↑` `↓` mueven la selección, `Enter` copia, `Esc` colapsa.
- Un clic en un resultado copia el valor al portapapeles (nunca se muestra en pantalla).
- Arrastra la pastilla cerca de un borde para anclarla; lejos, para que flote. Se **absorbe** en el
  borde en cuanto se acerca (el morph ocurre mientras arrastras), y quien decide al soltar es el
  **puntero**: soltándola sobre el borde queda clavada ahí —aunque la sueltes justo pasado el borde
  de la pantalla—; para desanclarla hay que tirar más de 96 px hacia dentro.
- **Recuerda dónde quedó**: borde y altura si está anclada, o su sitio si flota, y el siguiente
  arranque abre exactamente ahí (no en el sitio por defecto del sistema). Se guarda en
  `%LOCALAPPDATA%\edgePass\placement.json` **relativo al monitor**, así que un cambio de resolución o
  de DPI mueve la pastilla con la pantalla en vez de dejarla fuera de vista; si la pantalla donde
  vivía ya no está, vuelve al monitor principal.
- **Salir**: tab **Logs** → *Salir de edgePass* (dos pasos; `Esc` cancela el primero). El
  widget no tiene marco ni entrada en la barra de tareas, así que este es su único cierre
  ordenado. Cerrar **no** sella el vault: para eso está el LED **KV**.
- Abrir el `.exe` dos veces no lanza una segunda instancia: le pasa el foco a la que corre.
- **Rótulos según la máquina**: los nombres de las pestañas siguen el **formato regional** de
  Windows — `Secrets · Batch · Credentials · Config · Logs` en inglés, `Secretos · Batch ·
  Credenciales · Ajustes · Logs` en español. El idioma se pregunta a Rust al arrancar
  (`GetUserDefaultLocaleName`), porque el idioma de interfaz y el formato regional son ajustes
  distintos y desde la webview no se distinguen.
- **Credentials**: buscador por sitio, usuario o dominio (`+ Agregar` abre un renglón expandido
  para teclear la credencial; al guardar queda en la lista). La lista lee la metadata de cada
  `personal/logins/*` para poder buscar por lo que hay dentro, no solo por el slug: abrir el tab
  necesita el vault abierto (se desbloquea solo, como el resto).

## Toggles del header (KV / GW / GOV…)

Los botones del header son **datos**, no código de interfaz: cada uno es una fila
en `config.json` → `toggles[]`, que dice **qué mira** (`probe`) y **qué hace** al
click (`on` / `off`, y `alt` para Ctrl o clic derecho).

```jsonc
{ "id": "kv", "label": "KV", "probe": "vault.seal",
  "on": "vault.unseal", "off": "vault.seal", "alt": null }
```

- **Desde Ajustes → Toggles del header**: etiqueta (1-4 letras), sonda, las dos
  acciones (y la de Ctrl/clic derecho), `↑` `↓` para ordenar, `×` para quitar y
  `＋ Agregar un toggle`. Máximo **6**, y se aplican al pulsar *Guardar*.
- **Desde el pipe**, sin abrir el widget:

  ```jsonc
  {"op":"toggles"}                       // leer los LEDs + el vocabulario
  {"op":"toggles","set":[ … ]}           // reescribir la lista (validada)
  {"op":"toggle","id":"kv","dir":"flip"} // el click: flip | on | off | alt
  ```

  Helper: `powershell -File scripts\edgepass-toggles.ps1 -List` (o `-Catalog`,
  `-Fire kv -Dir off`, `-Set (Get-Content .\toggles.json -Raw)`).

El widget **publica su vocabulario** (`get_status` y cada respuesta de `toggles`
llevan `catalog`), así que nadie — ni un agente ni un contribuidor — tiene que
adivinar un id: una sonda o acción desconocida se rechaza *con* la lista de las
válidas. Un agente nombra el **LED**, nunca la acción: qué acción significa un
click se resuelve en Rust desde `config.json`.

- **Instalación nueva**: solo el LED **KV** (el único backend garantizado es el
  vault). `QR`/`GOV`/`GW` se añaden desde Ajustes con las sondas `qring.process` /
  `qring.governance` / `gha.runner` — así una máquina sin el pack de Q-ring y sin Docker
  Desktop no arrastra luces ámbar.
- **Actualización**: conserva los tres de siempre (`KV`, `GW`, `GOV`) y el `GW` viejo se
  muda a **`QR`** (v3→v4), porque `GW` pasó a vigilar el runner local de GHA: tu header
  queda `KV · QR · GOV · GW` sin perder el control del gateway.
- **`GW` ya no es el gateway del Q-ring**: `GW` vigila **Docker Desktop + el runner local
  de GitHub Actions** (el contenedor que deploya por vos, sin gastar minutos hospedados).
  Click = arreglo real: si Docker no responde lo arranca, si falta la imagen la construye,
  y si el contenedor está caído, parado o en loop de credencial, lo **recrea con un
  registration token nuevo** (`gh api`). Con todo sano, el click solo para el contenedor
  (`docker stop`: Docker Desktop queda quieto). `Ctrl`/clic derecho = abrir DaemonCurier.
  Runbook: [`docker/gha-local/README.md`](docker/gha-local/README.md) y
  `scripts\gha-local.ps1` para el camino manual.
- **El proceso del Q-ring vive en `QR`**: `QR` es **proceso** (arranca/para/reinicia el
  gateway, `Ctrl`/clic derecho = reiniciar) y `GOV` es **política** (si los rings pueden
  correr *y gastar*). Están acoplados en un sentido para que no queden estados
  mentirosos: parar el gateway apaga la política (`qring.off` escribe
  `qringGoverned=false`), y encender la política con el gateway parado lo arranca primero
  (con el vault, como `qring.on`). Al revés sí es legítimo, y es la razón de que exista
  `GOV`: `GOV` off con `QR` on = gateway arriba, nadie gasta.
  El **pulso de Allow** (Allow once / 2h / Deny) vive en `GOV`; el chip `Allow` del
  header lo muestra aunque hayas quitado ese LED.
- **Añadir un tipo nuevo de LED** (sonda o acción) es una fila en
  `src-tauri/src/toggles.rs` + su rama; el catálogo, los desplegables de Ajustes,
  los tooltips y el contrato del pipe salen de esas tablas.
- Contrato completo (campos, reglas, glosario, mapeo a `/v1/sys/*` de OpenBao y
  qué falta para apuntar un LED a otro KV compatible): [`docs/TOGGLE-CONTRACT.md`](docs/TOGGLE-CONTRACT.md).

## Autostart (tras servicio OpenBao)

OpenBao corre **por su cuenta** como servicio de Windows (`OpenBao`, NSSM). edgePass es el
**cliente**: se conecta, desbloquea al usar y solo sella si tú lo pides. El autostart solo
lanza el widget una vez al logon, con una tarea programada nativa (sin wrapper de
PowerShell ni `-ExecutionPolicy Bypass`) y 8 s de retardo para la ráfaga del logon. No
espera a OpenBao: si el vault todavía no está, el widget lo dice y ofrece el botón Svc.

```powershell
powershell -File scripts\install-autostart.ps1
powershell -File scripts\install-autostart.ps1 -Remove
```

## Instalaciones y reinicios: siempre avisar

Una instalacion del producto **mata el proceso que esta sirviendo el vault**. Paso el
6 de noviembre: una instalacion silenciosa del 0.4.0 cerro el widget a las 11:10:49 sin
cierre ordenado y dejo el pipe sin servicio. Reel lo descubrio porque estaba por grabar.

Reglas:

1. **Ninguna instalacion sin aviso.** Un instalador que mata el proceso que sirve el
   vault es un corte de servicio, aunque dure tres minutos.
2. **Antes de instalar, cerra el widget de forma ordenada** y decilo. Si el proceso
   muere por el instalador, el log no registra cierre y parece una caida.
3. **Despues de instalar, verifica en una llamada aparte**: `ping` con la version
   esperada y el proceso apuntando al exe que instalaste (instalador:
   `%LOCALAPPDATA%\edgePass\edgepass.exe`; Scoop: `…\scoop\apps\edgepass\current`).
4. Si el proceso no arranca, `Start-ScheduledTask -TaskName edgePass` y volve a
   verificar. Nunca `Start-Process`: el job del shell lo mata al cerrarse.

## Watchdog del widget

No hay watchdog automático: la tarea `edgePass` corre **una vez al logon**
(`MultipleInstances = IgnoreNew`). Relanzar cada N minutos se probó y se quitó:
un widget que murió vuelve con `Start-ScheduledTask -TaskName edgePass`, y la
instancia única rechaza el segundo proceso antes de abrir el pipe.
