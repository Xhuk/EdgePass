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
    batch: "Batch",
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
    batch: "Meter 1 or N variables · same UI as the ingest-form IPC",
    credentials: "Web passwords (user / password)",
    mcp: "MCP servers: list, autostart, connection data",
    config: "Vault address, namespaces, auto-unseal",
    logs: "Which binary runs, which log it writes, what it sees of the vault",
    notices: "What the committee left on disk (hook + watcher): count and list",
  },
  es: {
    secrets: "Listar OpenBao · copiar · editar · borrar",
    batch: "Meter 1 o N variables · misma UI que ingest-form IPC",
    credentials: "Contraseñas web (usuario / password)",
    mcp: "Servidores MCP: lista, auto arranque, datos de conexión",
    config: "Dirección del vault, namespaces, auto-unseal",
    logs: "Qué binario corre, qué log escribe, qué ve del vault",
    notices: "Qué dejó el comité en disco (hook + watcher): contador y lista",
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
  Sincronización: "Sync",
  "Toggles del header": "Header toggles",
  "Token manual": "Manual token",
  "Últimas líneas del log": "Last log lines",
  "URL / dominio": "URL / domain",
  Usuario: "User",
  Valor: "Value",
  "Cada botón del header es una fila: qué": "Each header button is a row: what",

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
  "etiqueta (ej. reasonixmcp)": "label (e.g. reasonixmcp)",
  "https://host:8443 (el origen público del server MCP)":
    "https://host:8443 (the MCP server public origin)",
  "Iniciar servicio Windows OpenBao": "Start OpenBao Windows service",
  "Limpiar búsqueda": "Clear search",
  "M2M · infisi cb2 · path variable…": "M2M · infisi cb2 · variable path…",
  "Mostrar u ocultar todos los valores": "Show or hide all values",
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
  "Secretos M2M": "M2M secrets",
  Subir: "Up",
  "túnel nombrado (cloudflared tunnel run …) — opcional":
    "named tunnel (cloudflared tunnel run …) — optional",
  "valor / placeholder… · Enter guarda": "value / placeholder… · Enter saves",

  // Body prose (multi-line blocks)
  "El servicio de OpenBao está detenido. ¿Quieres que edgePass lo arranque?":
    "The OpenBao service is stopped. Do you want edgePass to start it?",
  "Lo que el comité dejó en disco: el hook del servidor MCP (lo que se escribe por el board) y el vigilante de la carpeta (una nota copiada a mano, un archivo restaurado). El contador son las que todavía no marcaste como vistas.":
    "What the committee left on disk: the MCP server hook (what the board writes) and the folder watcher (a note copied by hand, a restored file). The counter is what you have not marked as seen yet.",
  "No es un servicio de Windows (la UI no puede vivir en Session 0): tarea al logon que espera al servicio OpenBao y lanza el widget una vez.":
    "Not a Windows service (a UI cannot live in Session 0): a logon task that waits for the OpenBao service and launches the widget once.",
  "Qué binario corre, qué log escribe y qué ve del vault. Es lo primero que hace falta cuando algo no arranca.":
    "Which binary runs, which log it writes and what it sees of the vault. It is the first thing you need when something does not start.",
  "Si el puntero sale del widget expandido, vuelve a la píldora tras N segundos.":
    "When the pointer leaves the expanded widget, it returns to the pill after N seconds.",
  "Sin credenciales aún. «+ Agregar» crea la primera (o importa un CSV abajo).":
    "No credentials yet. «+ Add» creates the first one (or import a CSV below).",
  "Todavía no hay avisos. Aparecen solos cuando el comité escribe.":
    "No notices yet. They appear by themselves when the committee writes.",
  "Todavía no hay servidores MCP. Con comando, argumentos y «auto» alcanza para darle persistencia de ejecución a un server como reasonixmcp.":
    "No MCP servers yet. A command, arguments and «auto» are enough to give a server like reasonixmcp execution persistence.",
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

export function tabLabels(): TabWords {
  return LABELS[lang];
}

export function tabTitles(): TabWords {
  return TITLES[lang];
}
