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
 * `t()` is keyed by the Spanish source text. Spanish is the identity: an
 * untranslated string still renders in Spanish instead of disappearing. That
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
 * Spanish source → English. Keep the left side byte-identical to the JSX text:
 * it is the key. A typo there is a silent fallback to Spanish, not an error.
 */
const EN: Record<string, string> = {
  // Labels, buttons, states
  Acceso: "Access",
  Actualizar: "Update",
  "Al apagar": "When turning off",
  "Al encender": "When turning on",
  "Auto-colapsar sin mouse (s · 0 = nunca)": "Auto-collapse when idle (s · 0 = never)",
  "Auto-sellar tras idle (s · 0 = nunca)": "Auto-seal after idle (s · 0 = never)",
  "Auto-unseal al usar": "Auto-unseal on use",
  "Borrar token guardado": "Delete saved token",
  Cancelar: "Cancel",
  "Cargar token del init": "Load token from init",
  "Copiar informe": "Copy report",
  "Ctrl / clic derecho": "Ctrl / right click",
  Descartar: "Discard",
  "Elegir CSV…": "Choose CSV…",
  Guardar: "Save",
  "Guardar nota": "Save note",
  "Importar CSV": "Import CSV",
  Inicio: "Home",
  "Leyendo credenciales del vault…": "Reading vault credentials…",
  "Leyendo…": "Reading…",
  Limpiar: "Clear",
  "Marcar todo como visto": "Mark all as seen",
  "No, gracias": "No, thanks",
  Nombre: "Name",
  Notas: "Notes",
  "OpenBao no está corriendo": "OpenBao is not running",
  Panel: "Panel",
  Rechazar: "Deny",
  Reconectar: "Reconnect",
  "Reemplazar path": "Replace path",
  "Reemplazar path si ya existe": "Replace path if it exists",
  Refrescar: "Refresh",
  Regenerar: "Regenerate",
  "Reindexar al abrir": "Reindex on open",
  "Servidores MCP": "MCP servers",
  "Sin notas todavía.": "No notes yet.",
  "Sin toggles: el header queda sin botones.": "No toggles: the header has no buttons.",
  // Servicios (owner-defined services behind a svc.<id> LED)
  Servicios: "Services",
  "Lo que corre en tu máquina y el binario no conoce: una API, un contenedor, un túnel.":
    "What runs on this machine that the binary does not know about: an API, a container, a tunnel.",
  "Cada fila publica un probe": "Each row publishes a probe",
  "con sus acciones": "with its actions",
  "Dos clases de sonda:": "Two probe kinds:",
  "(mira la lista de procesos, no ejecuta nada) y": "(reads the process list, runs nothing) and",
  "(0 = encendido, 2 = apagado, 3 = ámbar; la primera línea de stdout es el tooltip).":
    "(0 = on, 2 = off, 3 = amber; the first stdout line is the tooltip).",
  "Servicios en JSON": "Services as JSON",
  "Sin servicios definidos.": "No services defined.",
  "Agregar ejemplo": "Add example",
  Vaciar: "Clear",
  "se aplican al guardar": "applies on save",
  "solo mirar": "read only",
  "Solo mira: no tiene acción.": "Read-only: no action.",
  "El header muestra hasta": "The header shows up to",
  "LEDs; un servicio sin LED no se consulta.": "LEDs; a service without a LED is not probed.",
  "Meter · nuevo secreto": "Batch · new secret",

  // Names and placeholders that are passed through t() but read the same in
  // English. Listed so the coverage guard stays strict without exceptions.
  Allow: "Allow",
  GitHub: "GitHub",
  KV: "KV",
  Namespace: "Namespace",
  VARIABLE: "VARIABLE",
  Variable: "Variable",
  Vault: "Vault",
  edgePass: "edgePass",
  "github.com": "github.com",
  "https://host:8443": "https://host:8443",
  ns: "ns",
  "ns · .": "ns · .",
  path: "path",
  Sincronización: "Sync",
  "Toggles del header": "Header toggles",
  "Token manual": "Manual token",
  "Últimas líneas del log": "Last log lines",
  "URL / dominio": "URL / domain",
  Usuario: "User",
  Valor: "Value",
  "Cada botón del header es una fila: qué": "Each header button is a row: what",

  "Incluir el vigilante de acople": "Include the dock watcher",
  // MCP agnostico: el panel nombra las tres formas de ejecutar
  Ejecutable: "Executable",
  Directorio: "Directory",
  Argumentos: "Arguments",
  "Imagen de contenedor": "Container image",
  "URL pública": "Public URL",
  "Túnel nombrado": "Named tunnel",
  "separados por espacio": "space separated",
  "ghcr.io/usuario/servidor:tag (opcional, en vez del ejecutable)":
    "ghcr.io/user/server:tag (optional, instead of the executable)",
  "opcional (cloudflared tunnel run …)": "optional (cloudflared tunnel run …)",
  "etiqueta del servidor (ej. mi-servidor)": "server label (e.g. my-server)",
  "Pegar o copiar servidores como JSON": "Paste or copy servers as JSON",
  JSON: "JSON",
  "Ocultar JSON": "Hide JSON",
  "Agregar a la lista": "Add to the list",
  "Copiar JSON": "Copy JSON",
  "Un objeto o una lista. Campos: label, command, args, cwd, image, env, secretEnv, publicUrl, tunnel, path, autostart. Lo que falte toma el valor vacío.":
    "One object or a list. Fields: label, command, args, cwd, image, env, secretEnv, publicUrl, tunnel, path, autostart. Anything missing takes the empty value.",
  // Passphrase generator (Meter)
  "Sugerir contraseña": "Suggest password",
  "Contraseñas sugeridas": "Suggested passwords",
  "Cómo se arma la frase que sugiere el botón ✦ del Meter. Se pregunta una vez y se guarda.":
    "How the phrase suggested by the Batch tab's ✦ button is built. Asked once, then saved.",
  "Estilo de contraseñas guardado": "Password style saved",
  "Fuerza aproximada": "Approximate strength",
  Aleatorio: "Random",
  "Caracteres frescos por frase; no se guardan. El salt de instalación solo diversifica entre máquinas.":
    "Fresh characters per phrase; they are not stored. The install salt only diversifies across machines.",
  "Guardar estilo": "Save style",
  "Guardar y usar": "Save and use",
  "Mostrar u ocultar todos los valores": "Show or hide all values",
  Palabras: "Words",
  "Primera vez. Elegí cómo se arma la frase y queda guardado. Después se cambia en Ajustes.":
    "First time. Choose how the phrase is built and it is saved. Change it later in Settings.",
  Separador: "Separator",
  "Todavía no hay servidores MCP. Alcanza con un ejecutable y sus argumentos, o una imagen, o una URL pública. Con «auto» el servidor arranca junto con edgePass, y eso es la persistencia.":
    "No MCP servers yet. An executable and its arguments, or an image, or a public URL is enough. With «auto» the server starts with edgePass, and that is the persistence.",
  palabras: "words",
  separador: "separator",
  espacio: "space",
  "sin separador": "no separator",
  Mayúsculas: "Capitalize",
  "Dígitos": "Digits",
  "Símbolo": "Symbol",
  "Solo letras y números": "Letters and digits only",

  // Visible attributes
  "¿por qué se pidió? ¿para qué sirve?": "why was it requested? what is it for?",
  "Agregar una credencial a la lista": "Add a credential to the list",
  "argumentos, separados por espacio": "arguments, space separated",
  Arrancar: "Start",
  "Arrancar con edgePass, y edgePass arranca al iniciar sesión":
    "Start with edgePass, and edgePass starts at logon",
  "Arrastrar widget": "Drag widget",
  Bajar: "Down",
  "Borrar esta variable": "Delete this variable",
  "Buscar credencial": "Search credential",
  "Buscar sitio, usuario o dominio…": "Search site, user or domain…",
  "C:\\ruta\\server.exe": "C:\\path\\server.exe",
  CAMPO: "FIELD",
  "Copiar el valor": "Copy the value",
  "Copiar la ruta": "Copy the path",
  "Copiar la URL que va en el cliente": "Copy the client URL",
  "Copiar password": "Copy password",
  "Copiar todas las entradas como líneas KEY=valor": "Copy all entries as KEY=value lines",
  "Copiar usuario": "Copy user",
  "Copiar valor": "Copy value",
  Detener: "Stop",
  "directorio de trabajo (opcional)": "working directory (optional)",
  Editar: "Edit",
  "Editar en Batch (sobrescribir)": "Edit in Batch (overwrite)",
  "etiqueta (ej. mi-api)": "label (e.g. my-api)",
  "https://host:8443 (el origen público del server MCP)":
    "https://host:8443 (the MCP server public origin)",
  "Iniciar servicio Windows OpenBao": "Start OpenBao Windows service",
  "Limpiar búsqueda": "Clear search",
  "M2M · infisi cb2 · path variable…": "M2M · infisi cb2 · variable path…",
  "OpenBao offline — arranca el servicio OpenBao": "OpenBao offline — start the OpenBao service",
  "Sin secretos indexados aún": "No secrets indexed yet",
  "edgePass no es el vault: es el helper. Necesita un OpenBao local corriendo en loopback y un token: dirección, mount y namespace se definen en Ajustes → Vault.":
    "edgePass is not the vault: it is the helper. It needs a local OpenBao running on loopback and a token: address, mount and namespace are set in Settings → Vault.",
  "Conectar un vault": "Connect a vault",
  "El vault responde, pero no hay token cargado: pegá uno manual en Ajustes → Vault o define el Init JSON para que el widget lo lea solo.":
    "The vault answers, but no token is loaded: paste one in Settings → Vault, or set the Init JSON so the widget reads it on its own.",
  "Abrir Ajustes": "Open Settings",
  "Mover widget": "Move widget",
  "Notas de este secreto (por qué se pidió, para qué)":
    "Notes for this secret (why, what for)",
  "Pegar JSON en Batch": "Paste JSON in Batch",
  "Pegar objeto JSON con kvpath + campos": "Paste a JSON object with kvpath + fields",
  "Qué dice el botón (1-4 caracteres)": "Button label (1-4 characters)",
  "Quitar de la lista (se aplica al guardar)": "Remove from the list (applies on save)",
  "Quitar esta variable de la fila": "Remove this variable from the row",
  "Quitar este toggle": "Remove this toggle",
  Reindexar: "Reindex",
  Reiniciar: "Restart",
  "Releer el estado de los procesos": "Re-read process state",
  "ruta en el vault (ej. personal/servidor)": "vault path (e.g. personal/server)",
  "ruta del init del vault (opcional)": "vault init file path (optional)",
  "Secretos M2M": "M2M secrets",
  Subir: "Up",
  "túnel nombrado (cloudflared tunnel run …) — opcional":
    "named tunnel (cloudflared tunnel run …) — optional",
  "valor / placeholder… · Enter guarda": "value / placeholder… · Enter saves",

  // Body prose (multi-line blocks)
  "El servicio de OpenBao está detenido. ¿Quieres que edgePass lo arranque?":
    "The OpenBao service is stopped. Do you want edgePass to start it?",
  "Lo que un agente externo dejó en disco: el hook del servidor MCP (lo que escribe por el tablero) y el vigilante de la carpeta (una nota copiada a mano, un archivo restaurado). El contador son las que todavía no marcaste como vistas.":
    "What an outside agent left on disk: the MCP server hook (what it writes) and the folder watcher (a note copied by hand, a restored file). The counter is what you have not marked as seen yet.",
  "No es un servicio de Windows (la UI no puede vivir en Session 0): tarea al logon que espera al servicio OpenBao y lanza el widget una vez.":
    "Not a Windows service (a UI cannot live in Session 0): a logon task that waits for the OpenBao service and launches the widget once.",
  "Qué binario corre, qué log escribe y qué ve del vault. Es lo primero que hace falta cuando algo no arranca.":
    "Which binary runs, which log it writes and what it sees of the vault. It is the first thing you need when something does not start.",
  "Si el puntero sale del widget expandido, vuelve a la píldora tras N segundos.":
    "When the pointer leaves the expanded widget, it returns to the pill after N seconds.",
  "Sin credenciales aún. «+ Agregar» crea la primera (o importa un CSV abajo).":
    "No credentials yet. «+ Add» creates the first one (or import a CSV below).",
  "Todavía no hay avisos. Aparecen solos cuando un agente externo escribe.":
    "No notices yet. They appear by themselves when an outside agent writes.",
  "Todavía no hay servidores MCP. Con comando, argumentos y «auto» alcanza para darle persistencia de ejecución a un server como mi-api.":
    "No MCP servers yet. A command, arguments and «auto» are enough to give a server like my-api execution persistence.",
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

/** Translate a Spanish source string. Unknown keys pass through unchanged. */
export function t(es: string): string {
  if (lang === "es") return es;
  return EN[es] ?? es;
}

/** The English dictionary, exported for the coverage guard (`i18n.test.ts`). */
export const TRANSLATIONS: Readonly<Record<string, string>> = EN;

export function tabLabels(): TabWords {
  return LABELS[lang];
}

export function tabTitles(): TabWords {
  return TITLES[lang];
}
