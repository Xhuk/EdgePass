/**
 * The labels' language.
 *
 * Chosen from the machine's *regional* locale (Rust's `system_locale`, i.e.
 * Windows' regional format), not the display language: on plenty of machines
 * the two disagree — this one pairs an English format with a Spanish UI — and
 * from JS they cannot be told apart (WebView2's `navigator.language` and `Intl`
 * both follow the display language).
 *
 * The language is set once at boot, before the shell renders (`ready`), so the
 * first frame already has the right words.
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

export function tabLabels(): TabWords {
  return LABELS[lang];
}

export function tabTitles(): TabWords {
  return TITLES[lang];
}
