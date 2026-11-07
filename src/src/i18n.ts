/**
 * The interface language.
 *
 * Chosen from the machine's *regional* locale (Rust's `system_locale`, i.e.
 * Windows' regional format), not the display language: on plenty of machines
 * the two disagree, and from JS they cannot be told apart (WebView2's
 * `navigator.language` and `Intl` both follow the display language).
 *
 * The language is set once at boot, before the shell renders (`ready`), so the
 * first frame already has the right words.
 *
 * `t()` is keyed by the English source text. English is the identity: an
 * untranslated string still renders in English instead of disappearing. That
 * makes a missing translation a cosmetic bug, never a blank label.
 */

export type Lang = "es" | "en";

type TabWords = {
  secrets: string;
  batch: string;
  credentials: string;
  mcp: string;
  config: string;
  logs: string;
  /** The committee's notices (hook + watcher evidence), from Rust's `notices.rs`. */
  notices: string;
};

const LABELS: Record<Lang, TabWords> = {
  en: {
    secrets: "Secrets",
    batch: "Batch",
    credentials: "Credentials",
    mcp: "MCP",
    config: "Config",
    logs: "Logs",
    notices: "Notices",
  },
  es: {
    secrets: "Secretos",
    batch: "Meter",
    credentials: "Credenciales",
    mcp: "MCP",
    config: "Ajustes",
    logs: "Logs",
    notices: "Avisos",
  },
};

const TITLES: Record<Lang, TabWords> = {
  en: {
    secrets: "List OpenBao · copy · edit · delete",
    batch: "Batch 1 or N variables · same UI as the ingest-form IPC",
    credentials: "Web passwords (user / password)",
    mcp: "MCP servers: list, autostart, connection data",
    config: "Vault address, namespaces, auto-unseal",
    logs: "Which binary runs, which log it writes, what it sees of the vault",
    notices: "What an outside agent left on disk (hook + watcher): count and list",
  },
  es: {
    secrets: "Listar OpenBao · copiar · editar · borrar",
    batch: "Meter 1 o N variables · misma UI que ingest-form IPC",
    credentials: "Contraseñas web (usuario / password)",
    mcp: "Servidores MCP: lista, auto arranque, datos de conexión",
    config: "Dirección del vault, namespaces, auto-unseal",
    logs: "Qué binario corre, qué log escribe, qué ve del vault",
    notices: "Qué dejó un agente externo en disco (hook + watcher): contador y lista",
  },
};

/**
 * English source → Spanish. Keep the left side byte-identical to the JSX text:
 * it is the key. A typo there is a silent fallback to English, not an error.
 */
const ES: Record<string, string> = {
  "(0 = on, 2 = off, 3 = amber; the first stdout line is the tooltip).": "(0 = encendido, 2 = apagado, 3 = ámbar; la primera línea de stdout es el tooltip).",
  "(reads the process list, runs nothing) and": "(mira la lista de procesos, no ejecuta nada) y",
  "Access": "Acceso",
  "Add a credential to the list": "Agregar una credencial a la lista",
  "Add example": "Agregar ejemplo",
  "Add to the list": "Agregar a la lista",
  "Allow": "Allow",
  "applies on save": "se aplican al guardar",
  "Approximate strength": "Fuerza aproximada",
  "Arguments": "Argumentos",
  "arguments, space separated": "argumentos, separados por espacio",
  "Auto-collapse when idle (s · 0 = never)": "Auto-colapsar sin mouse (s · 0 = nunca)",
  "Auto-seal after idle (s · 0 = never)": "Auto-sellar tras idle (s · 0 = nunca)",
  "Auto-unseal on use": "Auto-unseal al usar",
  "Batch · new secret": "Meter · nuevo secreto",
  "Button label (1-4 characters)": "Qué dice el botón (1-4 caracteres)",
  "C:\\path\\server.exe": "C:\\ruta\\server.exe",
  "Cancel": "Cancelar",
  "Capitalize": "Mayúsculas",
  "Choose CSV…": "Elegir CSV…",
  "Clear": "Limpiar",
  "Clear search": "Limpiar búsqueda",
  "Configured address:": "Dirección configurada:",
  "Connect a vault": "Conectar un vault",
  "Container image": "Imagen de contenedor",
  "Copy all entries as KEY=value lines": "Copiar todas las entradas como líneas KEY=valor",
  "Copy JSON": "Copiar JSON",
  "Copy password": "Copiar password",
  "Copy report": "Copiar informe",
  "Copy the client URL": "Copiar la URL que va en el cliente",
  "Copy the path": "Copiar la ruta",
  "Copy the value": "Copiar el valor",
  "Copy user": "Copiar usuario",
  "Copy value": "Copiar valor",
  "Ctrl / right click": "Ctrl / clic derecho",
  "Delete saved token": "Borrar token guardado",
  "Delete this variable": "Borrar esta variable",
  "Deny": "Rechazar",
  "Digits": "Dígitos",
  "Directory": "Directorio",
  "Discard": "Descartar",
  "Down": "Bajar",
  "Drag widget": "Arrastrar widget",
  "Each header button is a row: what": "Cada botón del header es una fila: qué",
  "Each row publishes a probe": "Cada fila publica un probe",
  "edgePass": "edgePass",
  "edgePass is not the vault: it is the helper. It needs a local OpenBao running on loopback and a token: address, mount and namespace are set in Settings → Vault.": "edgePass no es el vault: es el helper. Necesita un OpenBao local corriendo en loopback y un token: dirección, mount y namespace se definen en Ajustes → Vault.",
  "Edit": "Editar",
  "Edit in Batch (overwrite)": "Editar en Batch (sobrescribir)",
  "Empty": "Vaciar",
  "Executable": "Ejecutable",
  "FIELD": "CAMPO",
  "First time. Choose how the phrase is built and it is saved. Change it later in Settings.": "Primera vez. Elegí cómo se arma la frase y queda guardado. Después se cambia en Ajustes.",
  "Fresh characters per phrase; they are not stored. The install salt only diversifies across machines.": "Caracteres frescos por frase; no se guardan. El salt de instalación solo diversifica entre máquinas.",
  "ghcr.io/user/server:tag (optional, instead of the executable)": "ghcr.io/usuario/servidor:tag (opcional, en vez del ejecutable)",
  "GitHub": "GitHub",
  "github.com": "github.com",
  "Header toggles": "Toggles del header",
  "Hide JSON": "Ocultar JSON",
  "Home": "Inicio",
  "How the phrase suggested by the Batch tab's ✦ button is built. Asked once, then saved.": "Cómo se arma la frase que sugiere el botón ✦ del Meter. Se pregunta una vez y se guarda.",
  "https://host:8443": "https://host:8443",
  "https://host:8443 (the MCP server public origin)": "https://host:8443 (el origen público del server MCP)",
  "Import CSV": "Importar CSV",
  "Include the dock watcher": "Incluir el vigilante de acople",
  "Install OpenBao": "Instalar OpenBao",
  "JSON": "JSON",
  "KV": "KV",
  "label (e.g. my-api)": "etiqueta (ej. mi-api)",
  "Last log lines": "Últimas líneas del log",
  "LEDs; a service without a LED is not probed.": "LEDs; un servicio sin LED no se consulta.",
  "Letters and digits only": "Solo letras y números",
  "Load token from init": "Cargar token del init",
  "M2M · infisi cb2 · variable path…": "M2M · infisi cb2 · path variable…",
  "M2M secrets": "Secretos M2M",
  "Manual token": "Token manual",
  "Mark all as seen": "Marcar todo como visto",
  "MCP servers": "Servidores MCP",
  "Move widget": "Mover widget",
  "Name": "Nombre",
  "Named tunnel": "Túnel nombrado",
  "named tunnel (cloudflared tunnel run …) — optional": "túnel nombrado (cloudflared tunnel run …) — opcional",
  "Namespace": "Namespace",
  "No credentials yet. «+ Add» creates the first one (or import a CSV below).": "Sin credenciales aún. «+ Agregar» crea la primera (o importa un CSV abajo).",
  "No MCP servers yet. A command, arguments and «auto» are enough to give a server like my-api execution persistence.": "Todavía no hay servidores MCP. Con comando, argumentos y «auto» alcanza para darle persistencia de ejecución a un server como mi-api.",
  "No MCP servers yet. An executable and its arguments, or an image, or a public URL is enough. With «auto» the server starts with edgePass, and that is the persistence.": "Todavía no hay servidores MCP. Alcanza con un ejecutable y sus argumentos, o una imagen, o una URL pública. Con «auto» el servidor arranca junto con edgePass, y eso es la persistencia.",
  "No notes yet.": "Sin notas todavía.",
  "No notices yet. They appear by themselves when an outside agent writes.": "Todavía no hay avisos. Aparecen solos cuando un agente externo escribe.",
  "No secrets indexed yet": "Sin secretos indexados aún",
  "no separator": "sin separador",
  "No services defined.": "Sin servicios definidos.",
  "No toggles: the header has no buttons.": "Sin toggles: el header queda sin botones.",
  "No, thanks": "No, gracias",
  "Not a Windows service (a UI cannot live in Session 0): a logon task that waits for the OpenBao service and launches the widget once.": "No es un servicio de Windows (la UI no puede vivir en Session 0): tarea al logon que espera al servicio OpenBao y lanza el widget una vez.",
  "Notes": "Notas",
  "Notes for this secret (why, what for)": "Notas de este secreto (por qué se pidió, para qué)",
  "ns": "ns",
  "ns · .": "ns · .",
  "One object or a list. Fields: label, command, args, cwd, image, env, secretEnv, publicUrl, tunnel, path, autostart. Anything missing takes the empty value.": "Un objeto o una lista. Campos: label, command, args, cwd, image, env, secretEnv, publicUrl, tunnel, path, autostart. Lo que falte toma el valor vacío.",
  "Open Settings": "Abrir Ajustes",
  "OpenBao answers but is not initialized: run `bao operator init` once and keep the init file.": "OpenBao responde pero no está inicializado: corré bao operator init una vez y guardá el init file.",
  "OpenBao is not running": "OpenBao no está corriendo",
  "OpenBao must run on loopback (127.0.0.1).": "OpenBao tiene que correr en loopback (127.0.0.1).",
  "OpenBao offline — start the OpenBao service": "OpenBao offline — arranca el servicio OpenBao",
  "optional (cloudflared tunnel run …)": "opcional (cloudflared tunnel run …)",
  "Panel": "Panel",
  "Password style saved": "Estilo de contraseñas guardado",
  "Paste a JSON object with kvpath + fields": "Pegar objeto JSON con kvpath + campos",
  "Paste JSON in Batch": "Pegar JSON en Batch",
  "Paste or copy servers as JSON": "Pegar o copiar servidores como JSON",
  "path": "path",
  "Public URL": "URL pública",
  "Random": "Aleatorio",
  "Re-read process state": "Releer el estado de los procesos",
  "read only": "solo mirar",
  "Read-only: no action.": "Solo mira: no tiene acción.",
  "Reading vault credentials…": "Leyendo credenciales del vault…",
  "Reading…": "Leyendo…",
  "Reconnect": "Reconectar",
  "Refresh": "Refrescar",
  "Regenerate": "Regenerar",
  "Reindex": "Reindexar",
  "Reindex on open": "Reindexar al abrir",
  "Remove from the list (applies on save)": "Quitar de la lista (se aplica al guardar)",
  "Remove this toggle": "Quitar este toggle",
  "Remove this variable from the row": "Quitar esta variable de la fila",
  "Replace path": "Reemplazar path",
  "Replace path if it exists": "Reemplazar path si ya existe",
  "Restart": "Reiniciar",
  "Retry": "Reintentar",
  "Save": "Guardar",
  "Save and use": "Guardar y usar",
  "Save note": "Guardar nota",
  "Save style": "Guardar estilo",
  "Search credential": "Buscar credencial",
  "Search site, user or domain…": "Buscar sitio, usuario o dominio…",
  "separator": "separador",
  "Separator": "Separador",
  "server label (e.g. my-server)": "etiqueta del servidor (ej. mi-servidor)",
  "Services": "Servicios",
  "Services as JSON": "Servicios en JSON",
  "Show or hide all values": "Mostrar u ocultar todos los valores",
  "space": "espacio",
  "space separated": "separados por espacio",
  "Start": "Arrancar",
  "Start OpenBao Windows service": "Iniciar servicio Windows OpenBao",
  "Start service": "Arrancar servicio",
  "Start with edgePass, and edgePass starts at logon": "Arrancar con edgePass, y edgePass arranca al iniciar sesión",
  "Stop": "Detener",
  "Suggest password": "Sugerir contraseña",
  "Suggested passwords": "Contraseñas sugeridas",
  "Symbol": "Símbolo",
  "Sync": "Sincronización",
  "The configured init file does not exist: check the path in Settings → Vault or move the file.": "El init file configurado no existe: revisá la ruta en Ajustes → Vault o mové el archivo.",
  "The header shows up to": "El header muestra hasta",
  "The OpenBao service is stopped. Do you want edgePass to start it?": "El servicio de OpenBao está detenido. ¿Quieres que edgePass lo arranque?",
  "The vault answers, but no token is loaded: paste one in Settings → Vault, or set the Init JSON so the widget reads it on its own.": "El vault responde, pero no hay token cargado: pegá uno manual en Ajustes → Vault o define el Init JSON para que el widget lo lea solo.",
  "The vault is sealed and there is no init file to open it: unseal with the keys or set initPath in Settings → Vault.": "El vault está sellado y no hay init file para abrirlo: unseal con las keys o configurá initPath en Ajustes → Vault.",
  "Two probe kinds:": "Dos clases de sonda:",
  "Up": "Subir",
  "Update": "Actualizar",
  "URL / domain": "URL / dominio",
  "User": "Usuario",
  "Value": "Valor",
  "value / placeholder… · Enter saves": "valor / placeholder… · Enter guarda",
  "Variable": "Variable",
  "VARIABLE": "VARIABLE",
  "Vault": "Vault",
  "vault init file path (optional)": "ruta del init del vault (opcional)",
  "vault path (e.g. personal/server)": "ruta en el vault (ej. personal/servidor)",
  "What an outside agent left on disk: the MCP server hook (what it writes) and the folder watcher (a note copied by hand, a restored file). The counter is what you have not marked as seen yet.": "Lo que un agente externo dejó en disco: el hook del servidor MCP (lo que escribe por el tablero) y el vigilante de la carpeta (una nota copiada a mano, un archivo restaurado). El contador son las que todavía no marcaste como vistas.",
  "What runs on this machine that the binary does not know about: an API, a container, a tunnel.": "Lo que corre en tu máquina y el binario no conoce: una API, un contenedor, un túnel.",
  "When the pointer leaves the expanded widget, it returns to the pill after N seconds.": "Si el puntero sale del widget expandido, vuelve a la píldora tras N segundos.",
  "When turning off": "Al apagar",
  "When turning on": "Al encender",
  "Which binary runs, which log it writes and what it sees of the vault. It is the first thing you need when something does not start.": "Qué binario corre, qué log escribe y qué ve del vault. Es lo primero que hace falta cuando algo no arranca.",
  "why was it requested? what is it for?": "¿por qué se pidió? ¿para qué sirve?",
  "with its actions": "con sus acciones",
  "words": "palabras",
  "Words": "Palabras",
  "working directory (optional)": "directorio de trabajo (opcional)",

  // Diagnostics report and the dock-only log hint (App.tsx)
  "(empty)": "(vacío)",
  "(not configured)": "(sin configurar)",
  "(only dock watcher lines; tick the box to see them)":
    "(sólo líneas del vigilante de acople; marcá la casilla para verlas)",
  "--- log (last lines, redacted) ---": "--- log (últimas líneas, redactado) ---",
  "edgePass · diagnostics report": "edgePass · informe de diagnóstico",
  "loaded": "cargado",
  "no token": "sin token",

  // Product surface moved out of raw literals (App.tsx, meterJson.ts, loginsCsv.ts)
  "Opening the vault if needed…": "Abriendo vault si hace falta…",
  "Save the list first: without an id there is nothing to talk to.":
    "Guardá la lista primero: sin id no hay a quién hablarle.",
  "Save the list first: without an id there is nothing to mint for.":
    "Guardá la lista primero: sin id no hay a quién acuñarle nada.",
  "Fill in variable, path and field, then save: the row has to declare them.":
    "Completá variable, path y campo, y guardá: la fila tiene que declararlos.",
  "Empty CSV, or no usable rows": "CSV vacío o sin filas útiles",
  "(unchanged)": "(sin tocar)",
  "Load JSON → editable rows (does not save to the vault yet)":
    "Cargar JSON → filas editables (aún no guarda en vault)",
  "Save to the vault (Enter) · no auto-save": "Guardar en vault (Enter) · no auto-save",
  "Rows with «auto» start with edgePass — and edgePass starts at logon, so that is the persistence.":
    "Los que tengan «auto» arrancan con edgePass — y edgePass arranca al iniciar sesión, así que eso es la persistencia.",
  "Name of the Cloudflare tunnel that publishes this server. edgePass starts and stops it with the server, and the MCP LED only says «on» when the connector is connected.":
    "Nombre del túnel de Cloudflare que publica este servidor. edgePass lo arranca y lo para junto con el servidor, y el LED MCP sólo dice «encendido» si el conector está conectado.",
  "Run the command that mints the client id: only the server can issue it":
    "Correr el comando que acuña el client id: solo el servidor puede emitirlo",
  "Mint a new value, save it to OpenBao (at the path this row declares) and show it to copy":
    "Acuñar un valor nuevo, guardarlo en OpenBao (donde dice esta fila) y mostrarlo para copiar",
  "— none —": "— sin extra —",
  "service": "servicio",
  "Sure? Quit now": "¿Seguro? Salir ahora",
  "Quit edgePass": "Salir de edgePass",
  "Esc to cancel.": "Esc para cancelar.",
  "Closes the widget. The vault stays as it is.": "Cierra el widget. El vault sigue como esté.",
  "Each header button is a row: what it": "Cada botón del header es una fila: qué",
  "watches": "mira",
  "(state) and what it": "(estado) y qué",
  "does": "hace",
  "on click. Everything lives in": "al click. Todo vive en",
  "and Rust publishes the catalog: an agent can read or rewrite it over the pipe":
    "y el catálogo lo publica Rust: un agente puede leerlo o reescribirlo por el pipe",
  ", without touching code.": ", sin tocar código.",
  "Empty JSON": "JSON vacío",
  "expected an object { kvpath, …fields }": "se espera un objeto { kvpath, …campos }",
  "missing 'kvpath' (OpenBao path)": "falta 'kvpath' (path OpenBao)",
  "no fields (only kvpath/ns/title metadata)": "sin campos (solo meta kvpath/ns/title)",
  "This CSV does not look like Google Password Manager (missing url/username/password columns)":
    "CSV no parece de Google Password Manager (faltan columnas url/username/password)",
};

function fromTag(tag: string): Lang {
  return tag.trim().toLowerCase().startsWith("es") ? "es" : "en";
}

/** Fallback for the platforms Rust answers nothing for. */
let lang: Lang = fromTag(
  (() => {
    try {
      return navigator.language ?? "";
    } catch {
      return "";
    }
  })(),
);

export function setLocale(tag: string) {
  if (tag.trim()) lang = fromTag(tag);
}

export function getLocale(): Lang {
  return lang;
}

/** Translate an English source string. Unknown keys pass through unchanged. */
export function t(en: string): string {
  if (lang === "en") return en;
  return ES[en] ?? en;
}

/** The Spanish dictionary, exported for the coverage guard (`i18n.test.ts`). */
export const TRANSLATIONS: Readonly<Record<string, string>> = ES;

export function tabLabels(): TabWords {
  return LABELS[lang];
}

export function tabTitles(): TabWords {
  return TITLES[lang];
}
