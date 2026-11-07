import { useCallback, useEffect, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type MouseEvent as ReactMouseEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { AnimatePresence, motion } from "motion/react";
import {
  animateTo,
  BATCH_VISIBLE_ROWS,
  DOCK_ISLAND_W,
  edgeDistances,
  expandGeometry,
  FLOAT_COLLAPSED,
  geometryFromPlacement,
  getWindowLogic,
  isMorphing,
  nearestEdge,
  placementFrom,
  pointerEdgeDistance,
  prefersReducedMotion,
  setMorphingListener,
  sizeDockTo,
  sizeForBatchRows,
  sizeForChrome,
  setPanelHeight,
  snapToDock,
  pushedRecently,
  traceUi,
  UNDOCK_PX,
  type DockEdge,
  type Placement,
} from "./dock";
import { LogoMark } from "./Logo";
import { setLocale, t, tabLabels, tabTitles } from "./i18n";
import { spring, tap } from "./motion";
import { parseGooglePasswordCsv, slugifyLogin, type LoginDraft } from "./loginsCsv";
import { parseMeterJson } from "./meterJson";
import "./App.css";

/** Horizontal chrome the rows do not report: the shell's and the drawer's
 *  padding plus the hairlines. Small, and it only ever makes the window a touch
 *  wider than it strictly needs. */
const CHROME_PAD = 26;

/**
 * Width a row *needs*, walking its children instead of reading the container.
 *
 * A squeezed row reports the room it was given, not what it wants: `.actions`
 * shrinks its LEDs (`App.css`, from four up) and `.tabs-strip` scrolls, so
 * `getBoundingClientRect()` on either of them is the answer to "how much space
 * do I have", which is the question that produced the bitten labels. Summing
 * the children gives the answer to "how much space do I need".
 */
function neededWidth(el: HTMLElement | null, depth = 0): number {
  if (!el) return 0;
  const style = getComputedStyle(el);
  const pad =
    (Number.parseFloat(style.paddingLeft) || 0) + (Number.parseFloat(style.paddingRight) || 0);
  const kids = Array.from(el.children) as HTMLElement[];
  if (!kids.length || depth > 2) {
    // A leaf: `scrollWidth` is what a `nowrap` control wants, `rect.width` what
    // it was given. Take the larger.
    return Math.max(el.getBoundingClientRect().width, el.scrollWidth);
  }
  const gap = Number.parseFloat(style.columnGap || "0") || 0;
  const inner = kids.reduce((sum, kid) => sum + neededWidth(kid, depth + 1), 0);
  return inner + gap * (kids.length - 1) + pad;
}

/**
 * How wide the window has to be for both summary rows to fit whole: the header
 * (mark + LEDs) and the tabs (strip + reindex).
 *
 * The strips are `min-width: 0` with `overflow`, so nothing pushes the window
 * out by itself — which is exactly why the sixth tab and the fifth LED were
 * being clipped. This measures, and `sizeForChrome` widens.
 */
function chromeWidth(actions: HTMLElement | null, strip: HTMLElement | null): number {
  // `scrollWidth` de la tira, no sólo la suma de los hijos: la tira es
  // `overflow-x: auto`, así que con siete pestañas su `scrollWidth` es el ancho
  // real del contenido y los nombres no se comen. Se toma el mayor de los dos.
  const stripNeed = Math.max(
    neededWidth(strip?.parentElement ?? null),
    strip ? strip.scrollWidth + 44 : 0,
  );
  return Math.max(neededWidth(actions?.parentElement ?? null), stripNeed) + CHROME_PAD;
}

/** Half-remembered canvas context for text measurement (see [`textWidth`]). */
let measureCtx: CanvasRenderingContext2D | null = null;

/** How wide a piece of text paints, in CSS px, at a given font. */
function textWidth(text: string, font: string): number {
  if (!measureCtx) measureCtx = document.createElement("canvas").getContext("2d");
  if (!measureCtx) return text.length * 7;
  measureCtx.font = font;
  return measureCtx.measureText(text).width;
}

/**
 * The docked rail in CSS px: as tall as the column it stacks, as wide as its
 * widest LED label plus the chrome around it.
 *
 * Width has to come from the **text**, not from the boxes. Every box in the rail
 * is `width: 100%`, so each one reports the room it was given — which is the
 * wrong question — and the label is what silently overflows when that room is a
 * pixel short. That is exactly how `SYNC` got clipped, then wrapped, then pushed
 * the column past the window and dropped `KV` on top of the `edgePass` letters.
 *
 * Height does come from the boxes: the children of the tab do not stretch, so
 * their rects are their own content, plus the tab's padding and row gaps.
 */
function dockNeed(el: HTMLElement | null): { w: number; h: number } {
  if (!el) return { w: 0, h: 0 };
  const cs = getComputedStyle(el);
  const num = (v: string) => Number.parseFloat(v) || 0;
  const padX = num(cs.paddingLeft) + num(cs.paddingRight);
  const kids = Array.from(el.children) as HTMLElement[];
  let sum = num(cs.paddingTop) + num(cs.paddingBottom) + num(cs.rowGap) * Math.max(0, kids.length - 1);
  // Lo que ocupa cada hijo **por su contenido**, no por su caja: `scrollHeight`
  // del propio tab nunca baja (vale al menos su altura), así que usarlo como
  // medida convertía el ajuste en un ratchet que sólo crecía — el rail quedaba
  // ancho para siempre. El `scrollHeight` del hijo sí es su contenido, incluso
  // cuando la ventana todavía es corta y la columna de LEDs se desborda.
  for (const kid of kids) {
    sum += Math.max(kid.getBoundingClientRect().height, kid.scrollHeight);
  }
  const h = sum;

  const labels = Array.from(el.querySelectorAll<HTMLElement>(".led-label"));
  const labelCs = labels[0] ? getComputedStyle(labels[0]) : null;
  const font = labelCs
    ? `${labelCs.fontWeight} ${labelCs.fontSize} ${labelCs.fontFamily}`
    : "600 10px system-ui";
  const spacing = labelCs ? num(labelCs.letterSpacing) : 0;
  const tile = el.querySelector<HTMLElement>(".led-toggle");
  const tileCs = tile ? getComputedStyle(tile) : null;
  const tilePad = tileCs ? num(tileCs.paddingLeft) + num(tileCs.paddingRight) : 4;

  let widest = 0;
  for (const label of labels) {
    const text = (label.textContent ?? "").trim();
    widest = Math.max(widest, textWidth(text, font) + spacing * text.length + tilePad);
  }
  // The island puts the label beside the dot, so the tile gets one dot wider.
  if (el.classList.contains("island")) widest += 8 + 6;
  // Sólo texto de las etiquetas: el `scrollWidth` del tab también es un ratchet
  // (nunca baja de su ancho), y era lo que dejaba el rail con el ancho de la isla.
  return { w: Math.ceil(widest + padX), h: Math.ceil(h) };
}

/**
 * A search row. Deliberately carries **no** database id: the backend rewrites
 * its index on every sync, so a rowid can come to mean a different secret —
 * clicking a stale row would copy the wrong value. `(namespace, path, key)` is
 * the stable identity and is what goes back on copy.
 */
type SearchHit = {
  namespace: string;
  path: string;
  key: string;
  useCount: number;
  lastUsedAt: string | null;
  /** Cuántas notas tiene el path. El valor del secreto nunca viaja acá. */
  notes: number;
  score: number;
};

/**
 * Una nota ligada a un secreto. Vive en la base local (`edgepass.db`), no en
 * OpenBao: el vault guarda valores y su metadata es del motor KV — meterle
 * comentarios obliga a reescribir la versión, ensucia el path y en KV v1 ni
 * existe. `kind = "auto"` la escribió edgePass al entregar el secreto.
 */
type Note = {
  id: number;
  namespace: string;
  path: string;
  key: string;
  body: string;
  kind: string;
  actor: string;
  createdAt: string;
};

/** Identidad estable de un hit, la misma que usa el pipe: sin rowids. */
const hitKey = (h: { namespace: string; path: string; key: string }) =>
  `${h.namespace}\u0000${h.path}\u0000${h.key}`;

/**
 * One web credential, with the metadata read from the vault so the tab can be
 * searched by site, user or domain. The **password** is never kept here: only
 * whether it exists. Copying goes through `reveal_and_copy` (vault → clipboard,
 * use count included), exactly like the Secrets tab.
 */
type LoginRow = {
  path: string;
  slug: string;
  name: string;
  url: string;
  username: string;
  notes: string;
  hasPassword: boolean;
};

type SealStatus = {
  reachable: boolean;
  healthCode: number;
  initialized: boolean;
  sealed: boolean;
  progress: number;
  threshold: number;
  version: string | null;
  message: string;
};

type StatusDto = {
  baoAddr: string;
  mount: string;
  namespaces: string[];
  hasToken: boolean;
  indexed: number;
  lastSync: string | null;
  autoUnseal: boolean;
  autoSealIdleSecs: number;
  autoCollapseIdleSecs: number;
  autoSync: boolean;
  autoLoadToken: boolean;
  initPath: string;
  seal: SealStatus;
  idleSecs: number;
  service: {
    name: string;
    installed: boolean;
    running: boolean;
    state: string;
    message: string;
  };
  /** The header LEDs — data, not markup: what `config.toggles[]` says, each with
   *  what its probe sees right now. See `docs/TOGGLE-CONTRACT.md`. */
  toggles: ToggleView[];
  /** The vocabulary those specs are written in (probes + actions). Published by
   *  Rust so nothing here — and no agent reading the pipe — hardcodes an id. */
  catalog: ToggleCatalog;
  /** The MCP list with live process state (see the `mcp` module in Rust). */
  mcp: McpStatus;
};

/** One LED as stored in `config.json`. */
type ToggleSpec = {
  id: string;
  label: string;
  /** Probe id: how "is it on?" is answered. */
  probe: string;
  /** Action run when the probe says off / on. */
  on: string;
  off: string;
  /** Optional third action for Ctrl / right click. */
  alt?: string | null;
};

type ToggleState = {
  on: boolean;
  /** Reachable but unhealthy (vault unreachable, pack missing): amber. */
  warn: boolean;
  /** An Allow is waiting on this channel: the LED asks. */
  ask: boolean;
  /** `false` = nothing a click could do; the button is disabled. */
  live: boolean;
  /** What the probe saw, for the tooltip. */
  detail: string;
};

type ToggleView = ToggleSpec & {
  state: ToggleState;
  docs: { probe: string; on: string; off: string; alt?: string | null };
};

type ToggleCatalog = {
  maxToggles: number;
  maxLabelChars: number;
  probes: {
    id: string;
    doc: string;
    needsQring: boolean;
    /** What a new toggle on this probe should do. */
    on: string;
    off: string;
    alt?: string | null;
  }[];
  actions: { id: string; doc: string; needsVault: boolean }[];
};

/** How long the Meter keeps the values visible after an agent asked for them
 *  (`meter`): long enough to read and click copy, short enough that they do not
 *  sit on screen. */
const METER_AUTO_MASK_MS = 8000;

/** Rows the Avisos panel asks for. The counter is unaffected: `notices_list`
 *  reports the total either way, and the file only grows (nothing prunes it),
 *  so the panel is a window onto its tail. */
const NOTICE_ROWS = 50;

type IngestForm = {
  id: number;
  namespace: string;
  path: string;
  keys: string[];
  title?: string | null;
  /** `"vault"` (op `meter`) = the widget reads `path` itself and fills the same
   *  panel with its real fields. The values exist only here: the pipe that asked
   *  for them carries the path, never the secret. */
  fill?: string | null;
};

type GrantRequest = {
  id: string;
  actor: string;
  action: string;
  namespace: string;
  path: string;
  keys: string[];
  cmd: string[];
  cwd?: string | null;
  reason: string;
  detach: boolean;
  createdAt: number;
  expiresAt: number;
};

type BatchRow = { key: string; value: string };

type Panel = "list" | "batch" | "webPassword" | "settings" | "diag" | "mcp" | "notices";

/**
 * One line of `%APPDATA%\edgePass\steering-notices.jsonl`, as Rust's
 * `notices.rs` hands it over. Every field can come back empty: the file is
 * written by two PowerShell ends (the MCP hook and the folder watcher) and a
 * line that arrives half-written costs that line, not the panel.
 */
type Notice = {
  /** ISO-8601 with offset — what the writer saw, not what we render. */
  at: string;
  /** `steering` (a note) or `board` (a message on the board). */
  kind: string;
  /** Which end saw it: `mcp` (the hook), `watch` (the watcher), or a tag a
   *  manual run passed in. */
  source: string;
  /** The MCP tool involved, when there was one. */
  tool: string;
  root: string;
  path: string;
  name: string;
  bytes: number;
};

/** The tail of that file plus the counter: `unseen` is what the pill shows. */
type Notices = {
  items: Notice[];
  unseen: number;
  total: number;
  /** The `.jsonl` exists. False = the two writer scripts are not installed, so
   *  this tab has nothing to say and its vocabulary is not ours to impose. */
  available?: boolean;
};

/**
 * One MCP server as stored in `config.json` → `mcpServers[]`.
 *
 * The panel owns this list the way Ajustes owns `toggles[]`: it is edited here
 * and sent back **whole** on save, which is why the backend publishes the full
 * spec per row and not a summary.
 */
type McpServerSpec = {
  id: string;
  label: string;
  command: string;
  args: string[];
  cwd: string;
  /** Process image `tasklist` looks for. Empty = the command's file name. */
  image: string;
  /** Non-secret environment for the child. **Credentials do not belong here**:
   *  this is written to `config.json` in clear text. */
  env: { key: string; value: string }[];
  /** Credentials read from OpenBao **at spawn** and handed to the child's
   *  environment only. No value can live here — that is the point. */
  secretEnv: { key: string; namespace: string; path: string; field: string }[];
  /** Public origin this server is reachable at (the tunnel), for the URL a
   *  client pastes into its "MCP server" field. */
  publicUrl: string;
  /** Named Cloudflare tunnel that publishes `publicUrl` — the name after
   *  `cloudflared tunnel run`. Empty = local only, nothing to bring up: the
   *  routing and the credentials live in `~/.cloudflared`, so this is a name. */
  tunnel: string;
  path: string;
  /** Which clients this row is meant for. Documentation for the hint list. */
  clients: string[];
  /** Command that mints a client credential. Only the server can issue a valid
   *  client id, so the widget runs this instead of inventing one. */
  credentialCommand: string;
  /** Come up with the widget — and therefore at logon, via the `edgePass` task. */
  autostart: boolean;
};

/** A row as published: the whole spec flattened in, plus what the sweep sees. */
type McpServerView = McpServerSpec & {
  running: boolean;
  detail: string;
  /** The row declares a tunnel **and** its connector is running: a server on
   *  `127.0.0.1` that nobody can reach is not an "on" chain. */
  tunnelRunning: boolean;
  url: string;
  watchedImage: string;
  envKeys: string[];
  secretEnvKeys: string[];
};

type McpStatus = {
  total: number;
  running: number;
  autostart: number;
  tunnels: number;
  tunnelsUp: number;
  /** A missing binary (server or `cloudflared`): a fix, not a click. */
  broken: boolean;
  detail: string;
  servers: McpServerView[];
};

/** The spec fields of a published row, for the drafts the editor sends back. */
function mcpSpecOf(v: McpServerView): McpServerSpec {
  return {
    id: v.id,
    label: v.label,
    command: v.command,
    args: v.args ?? [],
    cwd: v.cwd ?? "",
    image: v.image ?? "",
    env: v.env ?? [],
    secretEnv: v.secretEnv ?? [],
    publicUrl: v.publicUrl ?? "",
    tunnel: v.tunnel ?? "",
    path: v.path ?? "/mcp",
    clients: v.clients ?? [],
    credentialCommand: v.credentialCommand ?? "",
    autostart: v.autostart,
  };
}

/** A new row. `id` is left empty on purpose: Rust derives it from the command
 *  and makes it unique, so the panel never has to invent one. */
function blankServer(): McpServerSpec {
  return {
    id: "",
    label: "",
    command: "",
    args: [],
    cwd: "",
    image: "",
    env: [],
    secretEnv: [],
    publicUrl: "",
    tunnel: "",
    path: "/mcp",
    clients: [],
    credentialCommand: "",
    autostart: true,
  };
}

/** One MCP server, or a list of them, pasted as JSON.
 *
 *  This is what makes the panel agnostic: any harness can emit
 *  `{ label, command, args, cwd, secretEnv }` and edgePass picks it up, instead
 *  of a person filling seven inputs by hand. Unknown fields are ignored and
 *  missing ones take the blank defaults, so a minimal object is enough.
 */
function parseMcpJson(text: string): McpServerSpec[] {
  const raw: unknown = JSON.parse(text);
  const items = Array.isArray(raw) ? raw : [raw];
  return items.map((item) => {
    const src = (typeof item === "object" && item !== null ? item : {}) as Record<string, unknown>;
    const base = blankServer();
    return {
      ...base,
      ...src,
      args: Array.isArray(src.args) ? (src.args as unknown[]).map(String) : base.args,
      env: Array.isArray(src.env) ? (src.env as McpServerSpec["env"]) : base.env,
      secretEnv: Array.isArray(src.secretEnv)
        ? (src.secretEnv as McpServerSpec["secretEnv"])
        : base.secretEnv,
      clients: Array.isArray(src.clients) ? (src.clients as McpServerSpec["clients"]) : base.clients,
      autostart: typeof src.autostart === "boolean" ? src.autostart : base.autostart,
    };
  });
}

/** Everything needed to explain a bad install without a debugger. */
function withoutDockNoise(tail: string): string {
  const kept = tail
    .split("\n")
    .filter((line) => !/\bui:\s\d+ms\s+dock/.test(line))
    .join("\n")
    .trimEnd();
  return kept.trim() === ""
    ? "(sólo líneas del vigilante de acople; marcá la casilla para verlas)"
    : kept;
}
type Diagnostics = {
  version: string;
  exe: string;
  buildTime: string | null;
  os: string;
  configPath: string;
  dbPath: string;
  logPath: string;
  webview2: string;
  indexed: number;
  lastSync: string | null;
  hasToken: boolean;
  initPath: string;
  baoAddr: string;
  mount: string;
  namespaces: string[];
  autoUnseal: boolean;
  autoSealIdleSecs: number;
  autoCollapseIdleSecs: number;
  autoSync: boolean;
  autoLoadToken: boolean;
  seal: SealStatus;
  service: StatusDto["service"];
  logTail: string;
  runs?: Array<{
    runId: string;
    state: string;
    code: number | null;
    cmd: string;
    pid: number;
  }>;
};

/** Plain-text version of the panel, for pasting into a chat or an issue. */
function buildReport(d: Diagnostics): string {
  const rows: [string, string][] = [
    ["edgePass", d.version],
    ["build", d.buildTime ?? "-"],
    ["exe", d.exe],
    ["os", d.os],
    ["webview2", d.webview2],
    ["config", d.configPath],
    ["db", d.dbPath],
    ["log", d.logPath],
    ["baoAddr", d.baoAddr],
    ["mount", d.mount],
    ["namespaces", d.namespaces.join(", ") || "(root)"],
    ["initJson", d.initPath],
    ["token", d.hasToken ? "cargado" : "sin token"],
    ["autoUnseal", String(d.autoUnseal)],
    ["autoSealIdleSecs", String(d.autoSealIdleSecs)],
    ["autoCollapseIdleSecs", String(d.autoCollapseIdleSecs)],
    ["autoSync", String(d.autoSync)],
    ["autoLoadToken", String(d.autoLoadToken)],
    [
      "runs",
      (d.runs ?? [])
        .map((r) => `${r.runId}:${r.state}:exit=${r.code ?? "-"}:${r.cmd}`)
        .join(" | ") || "(none)",
    ],
    ["indexadas", String(d.indexed)],
    ["lastSync", d.lastSync ?? "-"],
    ["vault", `${d.seal.message} (reachable=${d.seal.reachable}, sealed=${d.seal.sealed})`],
    ["servicio", `${d.service.state} · ${d.service.message}`],
  ];
  return [
    "edgePass · informe de diagnóstico",
    ...rows.map(([k, v]) => `${k}: ${v}`),
    "",
    "--- log (últimas líneas) ---",
    d.logTail || "(vacío)",
  ].join("\n");
}

function displayNs(ns: string) {
  return ns ? ns : "root";
}

function grantCountdown(expiresAt: number, nowSecs: number): string {
  const left = Math.max(0, expiresAt - nowSecs);
  const m = Math.floor(left / 60);
  const s = left % 60;
  return `${m}:${s.toString().padStart(2, "0")}`;
}

/** `C:\proyectos\lotly` → `lotly`: of that path, the project is the useful half. */
function noticeProject(root: string): string {
  const parts = root.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? root;
}

/**
 * `2026-10-06T16:32:27-06:00` → `06/10 16:32`, in the machine's own zone.
 *
 * A stamp that will not parse falls back to the raw string: the file is written
 * by PowerShell, and an odd line has to show up as odd, not blank.
 */
function noticeWhen(at: string): string {
  const when = new Date(at);
  if (Number.isNaN(when.getTime())) return at;
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${pad(when.getDate())}/${pad(when.getMonth() + 1)} ${pad(when.getHours())}:${pad(when.getMinutes())}`;
}

export default function App() {
  const [expanded, setExpanded] = useState(false);
  const [dock, setDock] = useState<DockEdge | null>(null);
  /** The remembered spot is in place and the window may be revealed (see the
   *  restore effect). Nothing renders before that: the first frame on screen
   *  has to be the right shape in the right place. */
  const [ready, setReady] = useState(false);
  const [panel, setPanel] = useState<Panel>("list");
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<SearchHit[]>([]);
  const [status, setStatus] = useState<StatusDto | null>(null);
  const [toast, setToast] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [bootMsg, setBootMsg] = useState("Guardian…");
  const [selected, setSelected] = useState(0);
  const [diag, setDiag] = useState<Diagnostics | null>(null);
  /** The dock watcher writes three lines every five seconds. In the viewer they
   *  are hidden by default: they used to be 100% of what was on screen. */
  const [showLogNoise, setShowLogNoise] = useState(false);
  /** The committee's notices, and how many of them the human has not looked at
   *  yet. Refilled by the same tick as the header, so the pill's counter keeps
   *  working while the widget is collapsed. */
  const [notices, setNotices] = useState<Notices | null>(null);
  /** Two-step quit: the widget has no taskbar button, so a stray click on
   *  "Salir" would otherwise be unrecoverable. */
  const [confirmQuit, setConfirmQuit] = useState(false);

  const [baoAddr, setBaoAddr] = useState("http://127.0.0.1:8200");
  const [mount, setMount] = useState("kv");
  const [namespaces, setNamespaces] = useState(".");
  const [token, setToken] = useState("");
  const [autoUnseal, setAutoUnseal] = useState(true);
  const [autoSealIdleSecs, setAutoSealIdleSecs] = useState(0);
  const [autoCollapseIdleSecs, setAutoCollapseIdleSecs] = useState(10);
  const [autoSync, setAutoSync] = useState(true);
  const [autoLoadToken, setAutoLoadToken] = useState(true);
  const [initPath, setInitPath] = useState("");

  const [ingest, setIngest] = useState<IngestForm | null>(null);
  const [batchRows, setBatchRows] = useState<BatchRow[]>([]);
  const [batchReplaceAll, setBatchReplaceAll] = useState(false);
  const [batchPage, setBatchPage] = useState(0);
  /** Meter rows whose value is masked. Opening the panel always starts with the
   *  values in the clear; the agent flow (`meter`) re-masks the block by itself. */
  const [maskedRows, setMaskedRows] = useState<Set<number>>(() => new Set());
  const [meterSaveFlash, setMeterSaveFlash] = useState(false);
  const [showMeterJson, setShowMeterJson] = useState(false);
  const [meterJsonText, setMeterJsonText] = useState("");
  const [loginDrafts, setLoginDrafts] = useState<LoginDraft[]>([]);
  const [loginReplaceAll, setLoginReplaceAll] = useState(true);
  const [loginFileName, setLoginFileName] = useState<string | null>(null);
  const [loginForm, setLoginForm] = useState<LoginDraft>({
    slug: "",
    name: "",
    url: "",
    username: "",
    password: "",
    notes: "",
  });
  /** The inline row: closed, or open for a new one / for `loginEditPath`. */
  const [loginOpen, setLoginOpen] = useState(false);
  const [loginEditPath, setLoginEditPath] = useState<string | null>(null);
  /** Searched over site, user, domain, slug and notes — see `loadLoginRows`. */
  const [loginQuery, setLoginQuery] = useState("");
  const [loginRows, setLoginRows] = useState<LoginRow[]>([]);  const [loginBusy, setLoginBusy] = useState(false);
  /** Notas: qué hit las tiene abiertas, las que se leyeron y el borrador. */
  const [noteTarget, setNoteTarget] = useState<string | null>(null);
  const [noteRows, setNoteRows] = useState<Note[]>([]);
  const [noteDraft, setNoteDraft] = useState("");
  /** The LED whose action is in flight. The header buttons stay mutually
   *  exclusive, exactly as the per-provider busy flags used to keep them. */
  const [toggleBusy, setToggleBusy] = useState<string | null>(null);
  /** The Ajustes draft of `config.toggles[]`. Seeded when that panel *opens*, not
   *  on every 5 s status poll: a row being edited must survive the tick. */
  const [toggleDrafts, setToggleDrafts] = useState<ToggleSpec[]>([]);
  const [autostart, setAutostart] = useState<{
    installed: boolean;
    message: string;
    exe: string;
    taskName: string;
  } | null>(null);
  const [autostartBusy, setAutostartBusy] = useState(false);

  const [grant, setGrant] = useState<GrantRequest | null>(null);
  const [grantBusy, setGrantBusy] = useState(false);
  const [nowSecs, setNowSecs] = useState(() => Math.floor(Date.now() / 1000));
  const [showOpenBaoPrompt, setShowOpenBaoPrompt] = useState(false);
  const [busyStartingOpenBao, setBusyStartingOpenBao] = useState(false);

  const inputRef = useRef<HTMLInputElement>(null);
  const loginFileRef = useRef<HTMLInputElement>(null);
  const selectedRef = useRef<HTMLLIElement>(null);
  const debounceRef = useRef<number | null>(null);
  const moveTimer = useRef<number | null>(null);
  /** Bumped on every drop so a slow drag-and-wait cannot decide twice. */
  const settleSeq = useRef(0);
  const collapseTimer = useRef<number | null>(null);
  const dragging = useRef(false);
  /** The header row's action cluster (the LEDs) and the tab strip. Both are
   *  `min-width: 0` and clip, so the window's width has to be *asked for* from
   *  what they need — see `chromeWidth`. */
  const actionsRef = useRef<HTMLDivElement>(null);
  const stripRef = useRef<HTMLDivElement>(null);  const dockRef = useRef<DockEdge | null>(null);
  /** The docked rail's own DOM: what it needs is measured from here. */
  const dockTabRef = useRef<HTMLDivElement | null>(null);
  /** El menú interior, abierto con un clic en el escudo (no con hover: el hover
   *  se comía el clic de los LEDs, que es lo que la barra compacta sirve para
   *  hacer). Los LEDs se esconden mientras está abierto. */
  const [dockIsland, setDockIsland] = useState(false);
  /** Mirrors `ready` for the callbacks that must not fire during boot. */
  const readyRef = useRef(false);
  const expandedRef = useRef(false);
  const grantRef = useRef<GrantRequest | null>(null);
  const grantRestorePill = useRef(false);
  const autoCollapseIdleSecsRef = useRef(10);
  /** Pending "re-mask the Meter" timer (see `armAutoMask`). */
  const revealTimer = useRef<number | null>(null);

  /**
   * Dock state lives in two places on purpose: React renders the shell from
   * `dock`, and `dockRef` is what the callbacks read when they run outside a
   * render (a geometry snapshot right after a snap must see the edge that was
   * just set, not the previous one). `applyDock` is the only writer, so the two
   * cannot drift.
   */
  const applyDock = useCallback((edge: DockEdge | null) => {
    dockRef.current = edge;
    setDock(edge);
  }, []);

  useEffect(() => {
    expandedRef.current = expanded;
  }, [expanded]);
  useEffect(() => {
    grantRef.current = grant;
  }, [grant]);
  useEffect(() => {
    if (!grant) return;
    setNowSecs(Math.floor(Date.now() / 1000));
    const id = window.setInterval(() => setNowSecs(Math.floor(Date.now() / 1000)), 1000);
    return () => window.clearInterval(id);
  }, [grant]);
  useEffect(() => {
    autoCollapseIdleSecsRef.current = autoCollapseIdleSecs;
  }, [autoCollapseIdleSecs]);

  // The window animator reports its own state, so a morph never re-renders the
  // tree mid-flight and never locks input out.
  useEffect(() => {
    setMorphingListener((active) => {
      document.documentElement.classList.toggle("is-morphing", active);
    });
    return () => setMorphingListener(null);
  }, []);

  // Focus follows the panel: the search box is what the user came for.
  useEffect(() => {
    if (!expanded || grant) return;
    const timer = window.setTimeout(() => inputRef.current?.focus(), 60);
    return () => window.clearTimeout(timer);
  }, [expanded, grant]);

  // Keep the keyboard cursor in view.
  useEffect(() => {
    selectedRef.current?.scrollIntoView({
      block: "nearest",
      behavior: prefersReducedMotion() ? "auto" : "smooth",
    });
  }, [selected]);

  // Escape always gets you out: out of the alert, then out of the armed quit,
  // then out of the panel.
  useEffect(() => {
    const onKey = (ev: KeyboardEvent) => {
      if (ev.key !== "Escape") return;
      if (grantRef.current) {
        ev.preventDefault();
        const g = grantRef.current;
        setGrantBusy(true);
        invoke("resolve_grant", { id: g.id, decision: "deny" })
          .then(async () => {
            const next = await invoke<GrantRequest | null>("peek_pending_grant");
            if (next) setGrant(next);
            else {
              setGrant(null);
              if (grantRestorePill.current) {
                grantRestorePill.current = false;
                await collapsePanel();
              }
            }
          })
          .catch((e) => setError(String(e)))
          .finally(() => setGrantBusy(false));
        return;
      }
      if (showOpenBaoPrompt) {
        setShowOpenBaoPrompt(false);
        return;
      }
      if (confirmQuit) {
        setConfirmQuit(false);
        return;
      }
      if (expandedRef.current) void collapsePanel();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // collapsePanel / onGrantDecision read refs, so a stale closure here is harmless.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [showOpenBaoPrompt, confirmQuit]);

  // Leaving the panel disarms the quit: an armed button must never be waiting
  // for a second click somewhere else.
  useEffect(() => {
    if (panel !== "diag") setConfirmQuit(false);
  }, [panel]);

  const applyStatus = useCallback((s: StatusDto) => {
    setStatus(s);
    setBaoAddr(s.baoAddr);
    setMount(s.mount);
    setNamespaces(s.namespaces.map((n) => (n === "" ? "." : n)).join(", "));
    setAutoUnseal(s.autoUnseal);
    setAutoSealIdleSecs(s.autoSealIdleSecs);
    setAutoCollapseIdleSecs(s.autoCollapseIdleSecs ?? 10);
    setAutoSync(s.autoSync);
    setAutoLoadToken(s.autoLoadToken);
    setInitPath(s.initPath);
  }, []);

  const runSearch = useCallback(async (q: string) => {
    try {
      const results = await invoke<SearchHit[]>("search", { query: q, limit: 80 });
      setHits(results);
      setSelected(0);
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  const refreshStatus = useCallback(async () => {
    const s = await invoke<StatusDto>("get_status");
    applyStatus(s);
    return s;
  }, [applyStatus]);

  /**
   * The committee's notices. `notices_list` never fails — a missing or
   * half-written `steering-notices.jsonl` comes back as "no notices" — so the
   * only failure here is the IPC call itself, and a tick that fails leaves the
   * last counter in place instead of blanking the header.
   */
  const refreshNotices = useCallback(async () => {
    setNotices(await invoke<Notices>("notices_list", { limit: NOTICE_ROWS }));
  }, []);

  const refreshAutostart = useCallback(async () => {
    try {
      const s = await invoke<{
        installed: boolean;
        message: string;
        exe: string;
        taskName: string;
      }>("autostart_status");
      setAutostart(s);
    } catch {
      setAutostart(null);
    }
  }, []);

  /**
   * Fire one header LED. The widget sends *which* LED and which direction — the
   * action that means is resolved in Rust from the config, so the UI never maps
   * ids to effects (and a LED the config does not have cannot be fired at all).
   * `dir` mirrors the header gestures: Ctrl/Cmd/Alt/Shift or right click = `alt`.
   */
  const fireToggle = useCallback(
    async (led: ToggleView, dir: "flip" | "alt") => {
      if (toggleBusy) return;
      setToggleBusy(led.id);
      setError(null);
      try {
        const res = await invoke<{ toast: string }>("fire_toggle", { id: led.id, dir });
        setToast(res.toast);
      } catch (e) {
        setError(String(e));
      } finally {
        setToggleBusy(null);
        // Whatever happened (or failed), the LED's truth comes from the probe.
        await refreshStatus().catch(() => undefined);
      }
    },
    [refreshStatus, toggleBusy],
  );

  const bootstrap = useCallback(async () => {
    setBusy(true);
    setBootMsg("Abriendo vault si hace falta…");
    try {
      const result = await invoke<{
        seal: SealStatus;
        message: string;
        synced: number | null;
      }>("bootstrap");
      setBootMsg(result.message);
      const s = await refreshStatus();
      await runSearch("");
      if (result.synced != null) setToast(`${result.synced} secretos listos`);

      // Si OpenBao no está reachable y el servicio está instalado pero detenido → preguntar
      if (!result.seal.reachable && s?.service?.installed && !s?.service?.running) {
        setShowOpenBaoPrompt(true);
      }
    } catch (e) {
      setError(String(e));
      setBootMsg("Error");
    } finally {
      setBusy(false);
    }
  }, [refreshStatus, runSearch]);

  const onOpenBaoStart = useCallback(async () => {
    setBusyStartingOpenBao(true);
    setShowOpenBaoPrompt(false);
    try {
      await invoke("openbao_service_start");
      setBootMsg("OpenBao arrancando…");
      // Esperar un poco y re-bootstrap
      await new Promise((r) => setTimeout(r, 2000));
      await bootstrap();
    } catch (e) {
      setError(String(e));
      setBootMsg("Error al arrancar OpenBao");
      setShowOpenBaoPrompt(true);
    } finally {
      setBusyStartingOpenBao(false);
    }
  }, [bootstrap]);

  const trySnapEdge = useCallback(async () => {
    if (isMorphing() || expandedRef.current || dragging.current) return;
    const cur = await getWindowLogic();
    const currentDock = dockRef.current;

    if (currentDock) {
      const d = edgeDistances(cur.x, cur.y, cur.w, cur.h, cur.work)[currentDock];
      // The pointer gets a say, not just the window: mid-drag the OS move loop
      // keeps reporting the position from before the morph, so a pill dropped
      // on the edge (or just past it, off screen) reads as "far away" and would
      // come back floating — while the cursor is still sitting on that edge.
      // Either one being close means "still absorbed", so a drop never
      // un-docks what the user just dragged into the edge.
      const dc = await pointerEdgeDistance(currentDock, cur.work, cur.scale);
      if (d <= UNDOCK_PX || dc <= UNDOCK_PX) {
        // Still within the sticky band — re-pin instead of leaving it be: a pill
        // dropped 40px inside the edge must land flush on it, and a drag that
        // ended mid-morph must be put back on the edge it is docked to. (A
        // no-op re-pin is skipped inside `snapToDock`, so no heartbeat.)
        await snapToDock(currentDock);
        return;
      }
      // Pulled away — float pill
      applyDock(null);
      await animateTo({
        x: Math.max(
          cur.work.x + 8,
          Math.min(cur.x, cur.work.x + cur.work.w - FLOAT_COLLAPSED.w - 8),
        ),
        y: Math.max(
          cur.work.y + 8,
          Math.min(cur.y, cur.work.y + cur.work.h - FLOAT_COLLAPSED.h - 8),
        ),
        w: FLOAT_COLLAPSED.w,
        h: FLOAT_COLLAPSED.h,
      });
      return;
    }

    const edge = nearestEdge(cur.x, cur.y, cur.w, cur.h, cur.work);
    if (!edge) return;

    setExpanded(false);
    applyDock(edge);
    await snapToDock(edge);
  }, [applyDock]);

  /** Remember where the pill ended up, so the next launch opens here instead of
   *  at the OS default spot. Fire-and-forget: a placement that fails to save
   *  costs a spot, never a secret. */
  const persistPlacement = useCallback(async () => {
    if (!readyRef.current) return;
    // Mid-flight geometry is not a spot: saving it writes a `dock` next to a
    // position that never matched that edge (that is how a `dock: "right"`
    // ended up stored with the floating pill's numbers).
    if (isMorphing()) return;
    try {
      const cur = await getWindowLogic();
      await invoke("save_placement", {
        placement: placementFrom(cur, cur.work, dockRef.current, cur.monitor),
      });
    } catch {
      /* sin placement: el widget abre donde lo ponga el SO */
    }
  }, []);

  /** Is the left mouse button still down? "Cannot tell" (macOS, or a failed
   *  call) counts as *not* held: deciding right away is the safe fallback. */
  const leftButtonHeld = useCallback(async () => {
    try {
      return (await invoke<boolean | null>("pointer_left_down")) === true;
    } catch {
      return false;
    }
  }, []);

  /**
   * Decide the snap after the window moved — including *during* a drag, which is
   * the whole point: the pill morphing into the shape of the edge as it gets
   * close ("a drop absorbed by capillarity") is the motion the pointer is
   * following, so the geometry has to keep up with it.
   *
   * What must not happen mid-drag is *remembering* it: the OS move loop still
   * owns the window's position, so a snapshot taken now can store a `dock` next
   * to coordinates that never matched that edge. The drop is what gets stored,
   * not the pause before it.
   */
  const settleAfterDrag = useCallback(async () => {
    const seq = ++settleSeq.current;
    dragging.current = false;
    if (isMorphing() || expandedRef.current) return;
    await trySnapEdge();
    // Bounded: a button that never comes up (a wedge, a lost release) must not
    // keep the widget from remembering anything for the rest of the session.
    for (let i = 0; i < 40 && (await leftButtonHeld()); i++) {
      await new Promise((r) => window.setTimeout(r, 120));
    }
    if (seq !== settleSeq.current) return; // a newer drop superseded this one
    await persistPlacement();
  }, [leftButtonHeld, persistPlacement, trySnapEdge]);

  const clearAutoMask = useCallback(() => {
    if (revealTimer.current !== null) {
      window.clearTimeout(revealTimer.current);
      revealTimer.current = null;
    }
  }, []);

  /** Mask every row of the form `rows` later, unless it was closed or replaced
   *  in the meantime. */
  const armAutoMask = useCallback(
    (rows: number) => {
      clearAutoMask();
      revealTimer.current = window.setTimeout(() => {
        revealTimer.current = null;
        setMaskedRows(new Set(Array.from({ length: rows }, (_, i) => i)));
      }, METER_AUTO_MASK_MS);
    },
    [clearAutoMask],
  );

  useEffect(() => clearAutoMask, [clearAutoMask]);

  // The header row and the tab strip clip instead of pushing, so the window's
  // width has to be *asked for*: measure both rows after paint and widen. Re-runs
  // when the panel opens, a LED is added, or the notices counter arrives — the
  // three events that change how much room those rows need. The counter is the
  // one that lands *after* the first paint (`notices_list` is async), so without
  // it the strip and the LEDs keep the room they had and get bitten.
  useEffect(() => {
    if (!expanded) return;
    const id = requestAnimationFrame(() => {
      const need = chromeWidth(actionsRef.current, stripRef.current);
      // El alto lo fija el panel cuando tiene uno propio (Batch, Credenciales);
      // al salir de esos paneles vuelve el genérico. Así hay un solo dueño del
      // alto y las dos rutas no animan a la vez.
      const ownHeight = panel === "batch" || panel === "webPassword";
      if (!ownHeight) setPanelHeight(0);
      if (need > 0) {
        void sizeForChrome(need).catch(() => undefined);
      }
    });
    return () => cancelAnimationFrame(id);
  }, [expanded, panel, status?.toggles?.length, notices?.unseen]);

  // La pestaña activa se trae a la vista: al abrir el panel desde el menú de la
  // isla, la que elegiste puede estar fuera del recorte de la tira.
  useEffect(() => {
    if (!expanded) return;
    const strip = stripRef.current;
    if (!strip) return;
    const active = strip.querySelector<HTMLElement>("button.active");
    active?.scrollIntoView({ block: "nearest", inline: "nearest" });
  }, [panel, expanded]);

  // La isla se abre a propósito (clic en el escudo), así que también se cierra a
  // propósito: si el foco se va a otra ventana, se recoge. Dejarla desplegada
  // tapando la pantalla sería peor que un clic perdido.
  useEffect(() => {
    if (!dockIsland) return;
    let unlisten: (() => void) | null = null;
    getCurrentWindow()
      .onFocusChanged(({ payload: focused }) => {
        if (!focused) setDockIsland(false);
      })
      .then((fn) => {
        unlisten = fn;
      })
      .catch(() => undefined);
    return () => {
      if (unlisten) unlisten();
    };
  }, [dockIsland]);

  // The docked rail asks for the room its content needs — and gives it back.
  //
  // Two shapes, one rule: the slim rail (label above dot) and, while the pointer
  // rests on it, the island (label beside dot). Whatever the shape, the window is
  // widened by exactly the shortfall measured after paint, so nothing is clipped
  // on either one; when the island collapses it returns to the size at which the
  // slim rail fit. A rail that already fits is a no-op, so the 5 s poll cannot
  // make it breathe.
  useEffect(() => {
    if (dock === null || expanded || dock === "top") {
      setDockIsland(false);
      return;
    }
    const edge = dock;
    // El tab del dock se monta recién al colapsar (antes estaba el panel): se
    // reintenta un par de frames en vez de esperar al próximo poll de 5 s, que es
    // lo que dejaba el rail "compactado a medias" con el ancho de la isla.
    let tries = 12;
    let checks = 0;
    const timers: number[] = [];
    /**
     * ¿Sigue valiendo esta medición?
     *
     * El `setTimeout` de la verificación sobrevive al cambio de estado: si el
     * usuario abre el panel en ese instante, la cadena seguía viva y pedía el
     * ancho del rail mientras el panel pedía el suyo. Eso es lo que se veía en el
     * log como tres `spring→` en el mismo milisegundo (460 → 70 → 778) y en
     * pantalla como un rebote. Ahora cada paso comprueba el estado *actual*.
     */
    const alive = () => dockRef.current === edge && !expandedRef.current;
    const later = (fn: () => void, ms: number) => {
      timers.push(window.setTimeout(fn, ms));
    };
    const verify = () => {
      if (!alive()) return;
      // A mitad de animación el viewport todavía es el viejo, y recién después
      // del settle el SO aplica el tamaño: corregir antes de eso es medir una
      // ventana que ya no existe (en el log: tres `corrige` seguidos bajando
      // 96 → 72 → 66 porque el viewport iba llegando tarde).
      if (isMorphing() || pushedRecently(250)) {
        if (checks < 3) later(verify, 300);
        return;
      }
      const el2 = dockTabRef.current;
      if (!el2) return;
      const vw = document.documentElement.clientWidth || 0;
      const need2 = dockNeed(el2);
      // El piso del rail también cuenta como "lo que necesita": comparar sólo
      // con las etiquetas hacía que la verificación creyera que sobraba ancho.
      const want = dockIsland ? Math.max(DOCK_ISLAND_W, need2.w) : Math.max(need2.w, 52);
      traceUi(`dock verify vp=${vw} need=${need2.w} want=${want} isla=${dockIsland ? 1 : 0}`);
      if (vw <= want + 12 || checks++ >= 3) return;
      traceUi(`dock corrige: vp=${vw} > need=${want} (intento ${checks})`);
      void sizeDockTo(edge, want, need2.h, { remember: !dockIsland }).catch(() => undefined);
      later(verify, 300);
    };
    const apply = () => {
      if (!alive()) return;
      const el = dockTabRef.current;
      if (!el) {
        traceUi(`dock fit: sin tab todavía (quedan ${tries})`);
        if (tries-- > 0) requestAnimationFrame(apply);
        return;
      }
      const need = dockNeed(el);
      if (need.w <= 0) return;
      // La isla tiene un ancho cómodo fijo (el layout en fila lo llena, y un
      // ancho derivado de las etiquetas la haría saltar entre LEDs); el rail fino
      // pide exactamente lo que sus etiquetas necesitan.
      const w = dockIsland ? Math.max(DOCK_ISLAND_W, need.w) : need.w;
      traceUi(`dock fit need=${need.w}x${need.h} isla=${dockIsland ? 1 : 0}`);
      void sizeDockTo(edge, w, need.h, { remember: !dockIsland }).catch(() => undefined);
      later(verify, 340);
    };
    const id = requestAnimationFrame(apply);
    return () => {
      cancelAnimationFrame(id);
      // Sin esto la cadena del rail seguía corriendo mientras el panel se abría.
      for (const t of timers) window.clearTimeout(t);
    };
  }, [dock, expanded, dockIsland, status?.toggles, notices?.unseen]);

  const openIngest = useCallback(
    async (form: IngestForm, filled?: BatchRow[], autoMask = false) => {
      setIngest(form);
      setBatchRows(
        filled?.length
          ? filled
          : form.keys.map((key) => ({ key, value: "" })),
      );
      setBatchReplaceAll(false);
      setBatchPage(0);
      // Every open starts visible: reading is the point of the agent flow, and
      // typing is the point of the rest. Only `meter` re-masks on a timer.
      setMaskedRows(new Set());
      if (autoMask) armAutoMask(filled?.length ?? form.keys.length);
      else clearAutoMask();
      applyDock(null);
      setExpanded(true);
      setPanel("batch");
      setError(null);
      const n = filled?.length ?? form.keys.length;
      await sizeForBatchRows(Math.max(n, BATCH_VISIBLE_ROWS));
      setToast(`Batch · ${form.path || "(nuevo)"} (${n || form.keys.length})`);
    },
    [applyDock, armAutoMask, clearAutoMask],
  );

  /** ✎ / op `meter` → Batch, con las filas ya rellenas desde el vault.
   *  `reveal` = los valores los pidió un agente, así que el bloque se re-enmascara
   *  solo a los pocos segundos. */
  const openPathInMeter = useCallback(
    async (
      namespace: string,
      path: string,
      opts: { isNew?: boolean; reveal?: boolean; title?: string } = {},
    ) => {
      setBusy(true);
      setError(null);
      try {
        if (opts.isNew) {
          await openIngest({
            id: Date.now(),
            namespace: "",
            path: "",
            keys: [""],
            title: "Meter · nuevo secreto",
          });
          return;
        }
        const fields = await invoke<{ key: string; value: string }[]>("read_secret_path", {
          namespace: namespace || ".",
          path,
        });
        const rows = fields.length ? fields : [{ key: "", value: "" }];
        await openIngest(
          {
            id: Date.now(),
            namespace,
            path,
            keys: rows.map((r) => r.key),
            title: opts.title ?? "Editar · sobrescribir",
          },
          rows,
          opts.reveal === true,
        );
      } catch (e) {
        setError(String(e));
      } finally {
        setBusy(false);
      }
    },
    [openIngest],
  );

  /** One form from outside. `fill: "vault"` (op `meter`) means "the widget reads
   *  this path and shows the fields": the caller only ever named a path. */
  const applyIngestForm = useCallback(
    async (form: IngestForm) => {
      if (form.fill === "vault" && form.path.trim()) {
        await openPathInMeter(form.namespace, form.path, {
          reveal: true,
          title: form.title ?? "Batch · copiar",
        });
        return;
      }
      await openIngest(form);
    },
    [openIngest, openPathInMeter],
  );

  const openGrant = useCallback(async (dto: GrantRequest) => {
    setGrant(dto);
    setError(null);
    if (!expandedRef.current) {
      grantRestorePill.current = true;
      await expandPanel();
    }
  }, []);

  const onGrantDecision = useCallback(async (decision: "deny" | "allow-once" | "allow-2h") => {
    const current = grantRef.current;
    if (!current || grantBusy) return;
    setGrantBusy(true);
    try {
      await invoke("resolve_grant", { id: current.id, decision });
      const next = await invoke<GrantRequest | null>("peek_pending_grant");
      if (next) {
        setGrant(next);
      } else {
        setGrant(null);
        if (grantRestorePill.current) {
          grantRestorePill.current = false;
          await collapsePanel();
        }
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setGrantBusy(false);
    }
  }, [grantBusy]);

  /**
   * Open where the widget was left. `placement.json` holds a *relative* spot
   * (the monitor, plus the pill's centre as a fraction of its work area), so a
   * resolution or DPI change moves with the layout instead of dropping the pill
   * off screen.
   *
   * The window is created hidden (`visible: false` in `tauri.conf.json`), which
   * is what makes this invisible to the user: the OS default position is never
   * on screen. See the reveal effect below for the other half.
   */
  useEffect(() => {
    (async () => {
      try {
        // Before anything is rendered: the labels' language is part of the
        // first frame (`ready` is set at the end of this effect).
        setLocale(await invoke<string>("system_locale"));
      } catch {
        /* sin locale de la máquina: queda el fallback del navegador */
      }
      try {
        let spot = await invoke<Placement | null>("get_placement");
        if (!spot) {
          // Nothing remembered yet: keep the spot the OS picked, but go through
          // the same path anyway. The window is born at the config size (356),
          // not at the pill's (`FLOAT_COLLAPSED`), so saving the raw geometry
          // would compute a centre for the wrong width — and a first launch near
          // an edge would glue itself to it a moment later, in plain sight.
          const cur = await getWindowLogic();
          spot = placementFrom(cur, cur.work, null, cur.monitor);
        }
        const target = await geometryFromPlacement(spot);
        if (target) {
          // `applyDock` also sets the ref the spring's `onMoved` handler reads.
          applyDock(target.dock);
          await animateTo(target.rect, { instant: true });
        }
      } catch {
        // A placement that cannot be applied is not fatal: the OS default spot
        // is exactly where the widget opened before any of this existed.
      }
      readyRef.current = true;
      setReady(true);
    })();
  }, [applyDock]);

  /**
   * Reveal the window only once the remembered shape is committed, so the first
   * frame the user sees is already the right one. Rust shows it anyway after a
   * few seconds: this widget is not in the taskbar, so a window that never gets
   * revealed would be unrecoverable.
   */
  useEffect(() => {
    if (!ready) return;
    invoke("show_window").catch(() => undefined);
    // Pin the spot we just opened at. Without this a first run would only be
    // remembered after the user happened to touch the window.
    void persistPlacement();
  }, [ready, persistPlacement]);

  useEffect(() => {
    bootstrap();
    refreshAutostart().catch(() => undefined);
    refreshNotices().catch(() => undefined);
    const id = window.setInterval(() => {
      // One poll covers the whole header: `get_status` carries the LEDs' specs
      // *and* what their probes see, so there is no second provider-specific call.
      refreshStatus().catch(() => undefined);
      // Same tick for the notices: the counter has to be right while the widget
      // is collapsed, and reading a few KB of JSONL is cheaper than the status
      // call already in here.
      refreshNotices().catch(() => undefined);
      invoke<GrantRequest | null>("peek_pending_grant")
        .then((pendingGrant) => {
          if (pendingGrant) {
            if (grantRef.current?.id !== pendingGrant.id) {
              void openGrant(pendingGrant);
            }
          } else if (grantRef.current) {
            setGrant(null);
            if (grantRestorePill.current) {
              grantRestorePill.current = false;
              void collapsePanel();
            }
          }
        })
        .catch(() => undefined);
    }, 5000);

    let unlisten: (() => void) | undefined;
    let unlistenGrant: (() => void) | undefined;
    (async () => {
      const pending = await invoke<IngestForm | null>("peek_pending_ingest");
      if (pending && (pending.keys?.length || pending.fill === "vault")) {
        await applyIngestForm(pending);
      }
      unlisten = await listen<IngestForm>("ingest-form", (ev) => {
        applyIngestForm(ev.payload).catch(() => undefined);
      });
      const pendingGrant = await invoke<GrantRequest | null>("peek_pending_grant");
      if (pendingGrant) {
        await openGrant(pendingGrant);
      }
      unlistenGrant = await listen<GrantRequest>("grant-request", (ev) => {
        openGrant(ev.payload).catch(() => undefined);
      });
    })();

    return () => {
      window.clearInterval(id);
      unlisten?.();
      unlistenGrant?.();
    };
  }, [bootstrap, refreshStatus, refreshAutostart, refreshNotices, applyIngestForm, openGrant]);

  useEffect(() => {
    let unlistenMoved: (() => void) | undefined;
    const win = getCurrentWindow();

    (async () => {
      try {
        await win.setResizable(false);
      } catch {
        /* capability / older runtime */
      }
      unlistenMoved = await win.onMoved(() => {
        // Programmatic snap/animate fires onMoved too — ignore those or we
        // schedule another snap and the shell pulses forever. El `WM_MOVE` de un
        // resize nuestro puede llegar *después* del settle, así que no alcanza con
        // `isMorphing()`: también se descarta lo que movimos nosotros hace nada.
        if (isMorphing() || pushedRecently()) return;
        dragging.current = true;
        clearCollapseTimer();
        if (moveTimer.current) window.clearTimeout(moveTimer.current);
        moveTimer.current = window.setTimeout(() => {
          void settleAfterDrag();
        }, 160);
      });
    })();

    // No periodic re-snap poll: that was the heartbeat. Snap only after a drop.

    return () => {
      unlistenMoved?.();
      if (moveTimer.current) window.clearTimeout(moveTimer.current);
    };
  }, [settleAfterDrag]);

  useEffect(() => {
    if (debounceRef.current) window.clearTimeout(debounceRef.current);
    debounceRef.current = window.setTimeout(() => runSearch(query), 100);
    return () => {
      if (debounceRef.current) window.clearTimeout(debounceRef.current);
    };
  }, [query, runSearch]);

  useEffect(() => {
    if (!toast) return;
    const t = window.setTimeout(() => setToast(null), 2200);
    return () => window.clearTimeout(t);
  }, [toast]);

  function clearCollapseTimer() {
    if (collapseTimer.current) {
      window.clearTimeout(collapseTimer.current);
      collapseTimer.current = null;
    }
  }

  function armCollapseTimer() {
    clearCollapseTimer();
    if (!expandedRef.current || dragging.current || grantRef.current) return;
    const secs = autoCollapseIdleSecsRef.current;
    if (secs <= 0) return;
    collapseTimer.current = window.setTimeout(() => {
      collapseTimer.current = null;
      if (!expandedRef.current || dragging.current) return;
      void collapsePanel();
    }, secs * 1000);
  }

  useEffect(() => {
    if (!expanded) clearCollapseTimer();
  }, [expanded]);

  async function expandPanel(target?: Panel) {    // No lock-out: calling this mid-flight just re-targets the spring, which
    // continues from where the window currently is.
    clearCollapseTimer();
    setDockIsland(false);
    // El ref se actualiza acá y no en el efecto espejo: la cadena de medición del
    // rail comprueba `expandedRef` en su próximo `requestAnimationFrame`, que
    // corre antes del efecto. Sin esto alcanzaba a pedir el ancho del rail en el
    // mismo frame en que el panel se abría (el rebote 70 → 778 del log).
    expandedRef.current = true;
    // The panel you were in is where you come back to — collapsing is not
    // "forget what I was doing". `batch` needs its form, so it falls back to the
    // list when there is nothing to edit.
    const want = target ?? panel;
    const back: Panel = want === "batch" && !ingest ? "list" : want;
    const cur = await getWindowLogic();
    const target_ = expandGeometry(dockRef.current, cur.work, cur);
    traceUi(`expand panel=${back} desde=${Math.round(cur.w)}x${Math.round(cur.h)}`);
    // Deliberately *not* persisted: an expanded panel is not a spot. The last
    // collapsed placement stays saved, so the pill comes back where it belongs
    // (including the edge it was docked to) instead of at panel size.
    applyDock(null);
    setExpanded(true);
    setPanel(back);
    // Una sola animación, no dos. Antes se abría al tamaño genérico y *después*
    // se medía y se iba al ancho real: en el log eso eran dos vuelos seguidos
    // (460 → 778, 766 ms + 877 ms) y en pantalla un rebote. El panel tiene que
    // existir para poder medirlo, así que se cede un frame y se pide el tamaño
    // final de una vez.
    await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
    const need = chromeWidth(actionsRef.current, stripRef.current);
    traceUi(`expand need=${Math.round(need)} panel=${back}`);
    if (need > 0) await sizeForChrome(need);
    else await animateTo(target_);
    if (back === "list") await runSearch(query);
    await refreshStatus();
  }

  /**
   * One tab, opened from the island or from the drawer. The drawer's buttons can
   * assume they are already expanded; the island's cannot, so this is the one
   * place that both go through — that is what makes "hover → menu → Ajustes"
   * work without the caller knowing where it came from.
   */
  async function openTab(name: Panel) {
    const wasExpanded = expandedRef.current;
    if (!wasExpanded) await expandPanel(name);
    switch (name) {
      case "batch":
        if (!ingest) {
          await openPathInMeter("", "", { isNew: true });
        } else {
          setPanel("batch");
          void sizeForBatchRows(Math.max(batchRows.length, BATCH_VISIBLE_ROWS));
        }
        break;
      case "webPassword":
        setPanel("webPassword");
        void loadLoginRows().catch(() => undefined);
        void sizeForBatchRows(BATCH_VISIBLE_ROWS);
        break;
      case "settings":
        openSettings();
        break;
      case "mcp":
        openMcp();
        break;
      case "notices":
        await openNotices();
        break;
      case "diag":
        await openDiag();
        break;
      default:
        setPanel("list");
    }
    // The section wins. Expansion runs a measurement chain that can set the
    // panel on its way out; landing on the start after the person clicked a
    // section loses the click. This is the last word, after every loader.
    if (name !== "batch" || ingest) setPanel(name);
  }

  /**
   * What a collapse does to the secrets that were on screen.
   *
   * The values are never kept "just in case": the pill and the island are one
   * stray click away, and a Batch form left full of plaintext is the worst thing
   * this widget could carry in memory. The *keys* stay (so the panel reopens on
   * the same path) and every row comes back masked.
   */
  function forgetSecretValues() {
    clearAutoMask();
    setBatchRows((rows) => rows.map((r) => ({ key: r.key, value: "" })));
    setMaskedRows(new Set(Array.from({ length: batchRows.length }, (_, i) => i)));
  }

  async function collapsePanel() {
    clearCollapseTimer();
    forgetSecretValues();
    const cur = await getWindowLogic();
    // Prefer snapping to nearest edge if already close; else float pill
    const edge = nearestEdge(cur.x, cur.y, cur.w, cur.h, cur.work);
    setExpanded(false);
    if (edge) {
      applyDock(edge);
      await snapToDock(edge);
    } else {
      applyDock(null);
      await animateTo({
        x: cur.x,
        y: cur.y,
        w: FLOAT_COLLAPSED.w,
        h: FLOAT_COLLAPSED.h,
      });
    }
    // A collapse can land somewhere new (another edge, another monitor) without
    // any `onMoved` arriving after the spring settles, so remember it here
    // rather than relying on the drag path.
    void persistPlacement();
  }

  async function toggle() {
    if (expanded) await collapsePanel();
    else await expandPanel();
  }

  /** Abre (o cierra) las notas de un hit. El valor no se toca: sólo referencias. */
  async function openNotes(hit: SearchHit) {
    const key = hitKey(hit);
    if (noteTarget === key) {
      setNoteTarget(null);
      return;
    }
    try {
      const rows = await invoke<Note[]>("notes_list", {
        namespace: hit.namespace,
        path: hit.path,
        key: null,
      });
      setNoteRows(rows);
      setNoteDraft("");
      setNoteTarget(key);
    } catch (e) {
      setError(String(e));
    }
  }

  /** Guarda una nota escrita a mano al lado del secreto. */
  async function saveNote(hit: SearchHit) {
    const body = noteDraft.trim();
    if (!body) return;
    try {
      const rows = await invoke<Note[]>("notes_add", {
        namespace: hit.namespace,
        path: hit.path,
        key: null,
        body,
        actor: "ui",
      });
      setNoteRows(rows);
      setNoteDraft("");
      // El contador del chip viene del índice: se refresca al reindexar/estado.
      await refreshStatus();
      await runSearch(query);
    } catch (e) {
      setError(String(e));
    }
  }

  async function onCopy(hit: SearchHit) {
    setBusy(true);
    setError(null);
    try {
      // Sent by reference, never by index row: the value that lands in the
      // clipboard is always the one under the cursor.
      const label = await invoke<string>("reveal_and_copy", {
        namespace: hit.namespace,
        path: hit.path,
        key: hit.key,
      });
      setToast(`Copiado · ${label}`);
      await runSearch(query);
      await refreshStatus();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function onDeleteSecret(hit: SearchHit) {
    if (
      !window.confirm(
        `¿Borrar ${displayNs(hit.namespace)}/${hit.path} · ${hit.key}?\nSi era la última variable del path, se elimina el path.`,
      )
    ) {
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const label = await invoke<string>("delete_secret", {
        namespace: hit.namespace || ".",
        path: hit.path,
        key: hit.key,
      });
      setToast(label);
      await runSearch(query);
      await refreshStatus();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  /**
   * The Credentials list: the indexed `personal/logins/*` paths, each with its
   * metadata read from the vault so the tab can be searched by site, user or
   * domain (the index only knows paths and key names, by design).
   *
   * N reads on open, and that needs the vault open: it is the price of a real
   * search over what is *inside* each credential instead of only its slug.
   */
  async function loadLoginRows() {
    setLoginBusy(true);
    try {
      const hits = await invoke<SearchHit[]>("search", {
        query: "personal/logins",
        limit: 200,
      });
      const paths = Array.from(
        new Set(
          hits
            .map((h) => h.path)
            .filter((p) => p.startsWith("personal/logins/")),
        ),
      ).sort();
      const rows = await Promise.all(
        paths.map(async (path): Promise<LoginRow> => {
          const slug = path.replace(/^personal\/logins\//, "");
          const blank: LoginRow = {
            path,
            slug,
            name: "",
            url: "",
            username: "",
            notes: "",
            hasPassword: false,
          };
          try {
            const fields = await invoke<{ key: string; value: string }[]>("read_secret_path", {
              namespace: ".",
              path,
            });
            const map = Object.fromEntries(fields.map((f) => [f.key, f.value]));
            return {
              path,
              slug,
              name: map.name ?? "",
              url: map.url ?? "",
              username: map.username ?? "",
              notes: map.notes ?? "",
              hasPassword: Boolean(map.password),
            };
          } catch {
            // One unreadable credential must not empty the whole list.
            return blank;
          }
        }),
      );
      setLoginRows(rows);
    } catch {
      setLoginRows([]);
    } finally {
      setLoginBusy(false);
    }
  }

  /** Multi-token AND over site / user / domain / slug / notes, like the search. */
  function loginMatches(row: LoginRow, query: string): boolean {
    const terms = query.trim().toLowerCase().split(/\s+/).filter(Boolean);
    if (!terms.length) return true;
    const hay = [row.name, row.url, row.username, row.slug, row.notes]
      .join(" ")
      .toLowerCase();
    return terms.every((t) => hay.includes(t));
  }

  function closeLoginRow() {
    setLoginOpen(false);
    setLoginEditPath(null);
    setLoginForm({ slug: "", name: "", url: "", username: "", password: "", notes: "" });
    setError(null);
  }

  function openLoginRow(path: string | null, draft?: LoginRow) {
    setLoginEditPath(path);
    setLoginForm({
      slug: draft?.slug ?? "",
      name: draft?.name ?? "",
      url: draft?.url ?? "",
      username: draft?.username ?? "",
      password: "",
      notes: draft?.notes ?? "",
    });
    setLoginOpen(true);
  }

  async function onSaveLoginForm() {
    const name = loginForm.name.trim() || loginForm.url.trim() || loginForm.username.trim();
    let slug = loginForm.slug.trim() || slugifyLogin(name, loginForm.url);
    slug = slug.toLowerCase().replace(/[^a-z0-9-]+/g, "-").replace(/^-+|-+$/g, "");
    if (!slug) {
      setError("slug / nombre requerido");
      return;
    }
    // Editing an existing one: the password may be left empty on purpose ("no
    // la toco"), since it is never read into the form.
    if (!loginEditPath && !loginForm.username.trim() && !loginForm.password.trim()) {
      setError("usuario o password requerido");
      return;
    }
    setBusy(true);
    setError(null);
    try {
      // Editing must not blank the stored password when the field is untouched.
      let password = loginForm.password;
      if (loginEditPath && !password) {
        const fields = await invoke<{ key: string; value: string }[]>("read_secret_path", {
          namespace: ".",
          path: loginEditPath,
        });
        password = fields.find((f) => f.key === "password")?.value ?? "";
      }
      const result = await invoke<{ imported: number; message: string }>("import_logins", {
        rows: [
          {
            slug,
            name: loginForm.name.trim() || name,
            url: loginForm.url.trim(),
            username: loginForm.username,
            password,
            notes: loginForm.notes || null,
          },
        ],
        replaceAll: true,
      });
      setToast(result.message);
      closeLoginRow();
      await loadLoginRows();
      await refreshStatus();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function onEditLogin(path: string) {
    const row = loginRows.find((r) => r.path === path);
    if (row) {
      // Metadata is already here; the password is fetched by `onSaveLoginForm`
      // only if the user types a new one.
      openLoginRow(path, row);
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const fields = await invoke<{ key: string; value: string }[]>("read_secret_path", {
        namespace: ".",
        path,
      });
      const map = Object.fromEntries(fields.map((f) => [f.key, f.value]));
      const slug = path.replace(/^personal\/logins\//, "");
      setLoginForm({
        slug,
        name: map.name ?? "",
        url: map.url ?? "",
        username: map.username ?? "",
        password: map.password ?? "",
        notes: map.notes ?? "",
      });
      setLoginOpen(true);
      setLoginEditPath(path);
      await sizeForBatchRows(BATCH_VISIBLE_ROWS);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  /** Keyboard is the fastest path through a secret picker: type, arrows, Enter. */
  function onSearchKey(ev: ReactKeyboardEvent<HTMLInputElement>) {
    if (ev.key === "ArrowDown" || ev.key === "ArrowUp") {
      ev.preventDefault();
      if (!hits.length) return;
      setSelected((i) =>
        ev.key === "ArrowDown" ? Math.min(hits.length - 1, i + 1) : Math.max(0, i - 1),
      );
      return;
    }
    if (ev.key === "Enter") {
      ev.preventDefault();
      const hit = hits[selected] ?? hits[0];
      if (hit && !busy) void onCopy(hit);
    }
  }

  // The old `onToggleSeal` lived here: opening/closing the vault is now the `KV`
  // LED's own spec (`vault.seal` probe, `vault.unseal`/`vault.seal` actions), so
  // the header and the pipe take the same path. See `src-tauri/src/toggles.rs`.

  async function onSync() {
    setBusy(true);
    setError(null);
    try {
      const n = await invoke<number>("sync_index");
      setToast(`Índice: ${n}`);
      await runSearch(query);
      await refreshStatus();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  /**
   * Open Ajustes *and* take a copy of the current LEDs. The draft is seeded here
   * rather than in `applyStatus`, because that runs on the 5 s poll and would
   * overwrite a row being edited.
   */
  /** The MCP list this panel edits, exactly like `toggles[]` in Ajustes. */
  const [mcpDrafts, setMcpDrafts] = useState<McpServerSpec[]>([]);
  /** Paste-a-server box. A harness can emit the JSON; a person can copy it back. */
  const [showMcpJson, setShowMcpJson] = useState(false);
  const [mcpJsonText, setMcpJsonText] = useState("");
  /** Live process state, refreshed on open and after every control click. */
  const [mcpLive, setMcpLive] = useState<McpStatus | null>(null);
  const [mcpMsg, setMcpMsg] = useState<string | null>(null);
  /** The value `mcp_mint_token` just acuñó: vive acá sólo para que el humano lo
   *  copie, y no se vuelve a mostrar (Rust no lo escribe en `config.json`, ni
   *  en el log, ni lo manda por el pipe). */
  const [mcpSecret, setMcpSecret] = useState<{ id: string; key: string; value: string } | null>(
    null,
  );

  async function refreshMcp() {
    try {
      setMcpLive(await invoke<McpStatus>("mcp_status"));
    } catch (e) {
      setError(String(e));
    }
  }

  function openMcp() {
    setMcpDrafts((status?.mcp?.servers ?? []).map(mcpSpecOf));
    setMcpMsg(null);
    setMcpLive(status?.mcp ?? null);
    setPanel("mcp");
    void refreshMcp();
    // This panel is a table, and the summary rows above it have to fit whole.
    sizeForChrome(chromeWidth(actionsRef.current, stripRef.current)).catch(() => undefined);
  }

  function editMcp(index: number, patch: Partial<McpServerSpec>) {
    setMcpDrafts((rows) => rows.map((r, i) => (i === index ? { ...r, ...patch } : r)));
  }

  /**
   * Edit the row's `secretEnv` list: the variables edgePass reads from the vault
   * **at spawn time** and puts only in the child's environment.
   *
   * `entry` is the index to patch, `-1` appends a new one and `null` removes it.
   */
  function editMcpSecret(
    row: number,
    entry: number,
    patch: Partial<McpServerSpec["secretEnv"][number]> | null,
  ) {
    setMcpDrafts((rows) =>
      rows.map((r, i) => {
        if (i !== row) return r;
        const list = [...r.secretEnv];
        if (patch === null) {
          list.splice(entry, 1);
        } else if (entry < 0) {
          list.push({ key: "", namespace: "", path: "", field: "", ...patch });
        } else {
          list[entry] = { ...list[entry], ...patch };
        }
        return { ...r, secretEnv: list };
      }),
    );
  }

  async function saveMcp() {
    if (!status) return;
    setBusy(true);
    try {
      // Only the MCP list travels: a `null` on every other field means "keep
      // what is stored", so saving this panel cannot stamp over Ajustes.
      await invoke("save_settings", {
        baoAddr: status.baoAddr,
        mount: status.mount,
        namespaces: status.namespaces,
        token: null,
        autoUnseal: null,
        autoSealIdleSecs: null,
        autoCollapseIdleSecs: null,
        autoSync: null,
        autoLoadToken: null,
        initPath: null,
        toggles: null,
        mcpServers: mcpDrafts,
      });
      setMcpMsg("Lista guardada.");
      setToast("MCP guardados");
      // Re-read the list **and** the drafts: Rust derives the `id` from the
      // command on save, so the rows the panel is holding still say "". Without
      // this, ▶ ■ ↻ would answer "guardá primero" right after saving.
      const fresh = await invoke<McpStatus>("mcp_status");
      setMcpLive(fresh);
      setMcpDrafts(fresh.servers.map(mcpSpecOf));
    } catch (e) {
      // Rust validates the list (ids, the cap, a command per row): show why.
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function mcpControl(id: string, dir: "on" | "off" | "restart") {
    if (!id.trim()) {
      setMcpMsg("Guardá la lista primero: sin id no hay a quién hablarle.");
      return;
    }
    setBusy(true);
    try {
      setMcpLive(await invoke<McpStatus>("mcp_control", { id, dir }));
      setMcpMsg(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  /** Runs the row's own credential command (a server mints its own client id)
   *  and shows what it printed, so the values can be copied into the client. */
  async function mcpCredential(id: string) {
    setBusy(true);
    try {
      const out = await invoke<string>("mcp_credential_command", { id });
      setMcpMsg(out);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  /**
   * Acuña un valor nuevo para una variable `secretEnv` de la fila: Rust lo
   * genera, lo escribe en OpenBao (en el namespace/path/campo que declara la
   * fila) y lo devuelve **una sola vez** para que el humano lo copie.
   *
   * El server lee estas variables al arrancar, así que el valor nuevo sólo vale
   * cuando la fila vuelve a arrancar: si estaba corriendo, se reinicia sola.
   */
  async function mcpRegenerate(row: McpServerSpec, key: string) {
    const id = row.id.trim();
    if (!id) {
      setMcpMsg("Guardá la lista primero: sin id no hay a quién acuñarle nada.");
      return;
    }
    if (row.secretEnv.some((e) => !e.key.trim() || !e.path.trim() || !e.field.trim())) {
      setMcpMsg("Completá variable, path y campo, y guardá: la fila tiene que declararlos.");
      return;
    }
    setBusy(true);
    setMcpMsg(null);
    setMcpSecret(null);
    try {
      // Rust lee la config *del archivo*, así que la fila tiene que estar
      // guardada antes de esto (el botón vive debajo del mismo aviso).
      const value = await invoke<string>("mcp_mint_token", { id, key });
      setMcpSecret({ id, key, value });
      const live = mcpLive?.servers.find((s) => s.id === id);
      if (live?.running) {
        setMcpLive(await invoke<McpStatus>("mcp_control", { id, dir: "restart" }));
        setMcpMsg(`${key}: valor nuevo en OpenBao y la fila se reinició para tomarlo.`);
      } else {
        setMcpMsg(`${key}: valor nuevo en OpenBao. Arrancá la fila para que el server lo tome.`);
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  function openSettings() {
    setToggleDrafts(
      (status?.toggles ?? []).map((t) => ({
        id: t.id,
        label: t.label,
        probe: t.probe,
        on: t.on,
        off: t.off,
        alt: t.alt ?? null,
      })),
    );
    setPanel("settings");
  }

  /** Catalog entry for a probe: what the picker shows and what a new row prefills. */
  function probeOf(id: string): ToggleCatalog["probes"][number] | undefined {
    return status?.catalog.probes.find((p) => p.id === id);
  }

  function actionDoc(id: string): string {
    return status?.catalog.actions.find((a) => a.id === id)?.doc ?? "";
  }

  /** A new row: the first probe in the catalog plus whatever it suggests. */
  function addToggle() {
    const probe = status?.catalog.probes[0];
    if (!probe) return;
    setToggleDrafts((rows) => [
      ...rows,
      {
        id: "",
        label: "",
        probe: probe.id,
        on: probe.on,
        off: probe.off,
        alt: probe.alt ?? null,
      },
    ]);
  }

  function patchToggle(index: number, patch: Partial<ToggleSpec>) {
    setToggleDrafts((rows) => rows.map((row, i) => (i === index ? { ...row, ...patch } : row)));
  }

  /** Changing the probe resets both sides to what *that* probe suggests: the old
   *  pair almost never makes sense for the new one. */
  function setDraftProbe(index: number, probeId: string) {
    const probe = probeOf(probeId);
    patchToggle(index, {
      probe: probeId,
      on: probe?.on ?? "",
      off: probe?.off ?? "",
      alt: probe?.alt ?? null,
    });
  }

  function moveToggle(index: number, delta: number) {
    setToggleDrafts((rows) => {
      const next = [...rows];
      const to = index + delta;
      if (to < 0 || to >= next.length) return rows;
      [next[index], next[to]] = [next[to], next[index]];
      return next;
    });
  }

  function removeToggle(index: number) {
    setToggleDrafts((rows) => rows.filter((_, i) => i !== index));
  }

  async function onSaveSettings() {
    setBusy(true);
    setError(null);
    try {
      const normalized = namespaces
        .split(",")
        .map((s) => s.trim())
        .map((s) => (s === "." ? "" : s));
      await invoke("save_settings", {
        baoAddr,
        mount,
        namespaces: normalized.length ? normalized : [""],
        token: token.trim() ? token : null,
        autoUnseal,
        autoSealIdleSecs,
        autoCollapseIdleSecs,
        autoSync,
        autoLoadToken,
        initPath: initPath || null,
        // The whole list, always: the editor owns it, and Rust validates the ids
        // against the catalog before anything is stored.
        toggles: toggleDrafts.map((t) => ({
          id: t.id,
          label: t.label,
          probe: t.probe,
          on: t.on,
          off: t.off,
          alt: t.alt || null,
        })),
      });
      setToken("");
      setToast("Ajustes guardados");
      setPanel("list");
      await bootstrap();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function onApplyMeterJson() {
    setError(null);
    try {
      const parsed = parseMeterJson(meterJsonText);
      await openIngest(
        {
          id: Date.now(),
          namespace: parsed.namespace,
          path: parsed.path,
          keys: parsed.rows.map((r) => r.key),
          title: parsed.title || "Batch · JSON",
        },
        parsed.rows,
      );
      setShowMeterJson(false);
      setMeterJsonText("");
    } catch (e) {
      setError(String(e instanceof Error ? e.message : e));
    }
  }

  /** How a suggested passphrase is shaped. Kept in the window only: it is a
   *  preference, not a secret, and it is not written to config.json. */
  const [passOpts, setPassOpts] = useState({
    words: 6,
    separator: "-",
    capitalize: false,
    digits: false,
    symbol: false,
    compatible: false,
  });
  /** What the last suggestion was worth, shown next to the controls. */
  const [passNote, setPassNote] = useState("");
  /** False until a person picks the style once. Read from config.json. */
  const [passConfigured, setPassConfigured] = useState(true);
  /** Row waiting for the first-run style question, or null. */
  const [passFirstRun, setPassFirstRun] = useState<number | null>(null);
  /** Row whose star is pulsing right now. Generating is instant, so the form is
   *  a pulse and a flash — not the save button's ring, which means "wait". */
  const [pulseRow, setPulseRow] = useState<number | null>(null);

  /** Loads the stored style so the settings panel shows what is really saved. */
  useEffect(() => {
    let alive = true;
    void invoke<{
      words: number;
      separator: string;
      capitalize: boolean;
      digits: boolean;
      symbol: boolean;
      compatible: boolean;
      configured: boolean;
    }>("passphrase_config")
      .then((c) => {
        if (!alive) return;
        setPassOpts({
          words: c.words,
          separator: c.separator,
          capitalize: c.capitalize,
          digits: c.digits,
          symbol: c.symbol,
          compatible: c.compatible,
        });
        setPassConfigured(c.configured);
      })
      .catch(() => undefined);
    return () => {
      alive = false;
    };
  }, []);

  /** «✦» on a row: a Diceware phrase built inside the widget.
   *
   *  The value is born in Rust and lands in the row like a typed one. It does
   *  not travel over the local pipe and it is not logged, so no agent ever sees
   *  it — only this window does, the same way it already shows a secret read
   *  from the vault. */
  async function onSuggestPassphrase(idx: number) {
    // The first suggestion asks for the style first, once. Rust marks it as
    // configured when it is saved, so this question never appears twice.
    if (!passConfigured) {
      setPassFirstRun(idx);
      return;
    }
    await generateInto(idx);
  }

  /** The generation itself, split out so the first-run step can call it once the
   *  style has been saved. */
  async function generateInto(idx: number) {
    setPulseRow(idx);
    window.setTimeout(() => setPulseRow((cur) => (cur === idx ? null : cur)), 520);
    try {
      const made = await invoke<{
        value: string;
        entropyBits: number;
        words: number;
        advice: string | null;
      }>("make_passphrase", { options: passOpts });
      const next = [...batchRows];
      next[idx] = { ...next[idx]!, value: made.value };
      setBatchRows(next);
      // A value the person asked for is shown, not masked: they need to read it.
      setMaskedRows((prev) => {
        const s = new Set(prev);
        s.delete(idx);
        return s;
      });
      const bits = made.entropyBits.toFixed(1).replace(".", ",");
      setPassNote(
        `${bits} bits · ${made.words} ${t("palabras")}${made.advice ? " · " + made.advice : ""}`,
      );
    } catch (e) {
      setError(String(e instanceof Error ? e.message : e));
    }
  }

  /** Saves the passphrase style. `configured` is set by Rust, not here: the only
   *  way to reach that command is a person choosing, which is what it records. */
  async function onSavePassphrase() {
    try {
      await invoke("save_passphrase_config", {
        settings: { ...passOpts, configured: true },
      });
      setPassConfigured(true);
      setToast(t("Estilo de contraseñas guardado"));
    } catch (e) {
      setError(String(e instanceof Error ? e.message : e));
    }
  }

  /** Copy one Meter row: the value is already in the widget, so this is the
   *  clipboard, not a vault read. */
  async function onCopyRow(row: BatchRow) {
    if (!row.value) return;
    try {
      await invoke("copy_to_clipboard", { text: row.value });
      setToast(`Copiado · ${row.key.trim() || "valor"}`);
    } catch (e) {
      setError(String(e));
    }
  }

  /** The whole block as `KEY=valor` lines: what a `.env` wants, and what saves
   *  copying six passwords one by one. */
  async function onCopyBlock() {
    const pairs = batchRows
      .filter((r) => r.key.trim())
      .map((r) => `${r.key.trim()}=${r.value}`);
    if (!pairs.length) return;
    try {
      await invoke("copy_to_clipboard", { text: pairs.join("\n") });
      setToast(`Copiado · ${pairs.length} entradas (KEY=valor)`);
    } catch (e) {
      setError(String(e));
    }
  }

  function onToggleRowMask(idx: number) {
    setMaskedRows((prev) => {
      const next = new Set(prev);
      if (next.has(idx)) next.delete(idx);
      else next.add(idx);
      return next;
    });
  }

  function onToggleAllMask() {
    setMaskedRows((prev) => (prev.size ? new Set() : new Set(batchRows.map((_, i) => i))));
  }

  async function onSaveBatch() {
    if (!ingest) return;
    const path = ingest.path.trim().replace(/^\/+|\/+$/g, "");
    if (!path) {
      setError("path es requerido");
      return;
    }
    const fields = batchRows.filter((r) => r.key.trim() && r.value.length > 0);
    if (!fields.length) {
      setError("al menos una variable con valor");
      return;
    }
    setBusy(true);
    setError(null);
    setMeterSaveFlash(false);
    try {
      const label = await invoke<string>("upsert_secret_batch", {
        namespace: ingest.namespace || ".",
        path,
        fields,
        replaceAll: batchReplaceAll,
      });
      await invoke("take_pending_ingest");
      setMeterSaveFlash(true);
      setToast(label);
      await new Promise((r) => window.setTimeout(r, 520));
      setIngest(null);
      setBatchPage(0);
      setMeterSaveFlash(false);
      setPanel("list");
      setQuery(path);
      await runSearch(path);
      await refreshStatus();
      const cur = await getWindowLogic();
      await animateTo({
        x: cur.x,
        y: cur.y,
        w: 460,
        h: 600,
      });
    } catch (e) {
      setError(String(e));
      setMeterSaveFlash(false);
    } finally {
      setBusy(false);
    }
  }

  function onPickLoginCsv(file: File | null) {
    if (!file) return;
    setError(null);
    const reader = new FileReader();
    reader.onload = () => {
      try {
        const text = String(reader.result ?? "");
        const drafts = parseGooglePasswordCsv(text);
        if (!drafts.length) {
          setError("CSV vacío o sin filas útiles");
          setLoginDrafts([]);
          setLoginFileName(null);
          return;
        }
        setLoginDrafts(drafts);
        setLoginFileName(file.name);
        setToast(`${drafts.length} logins listos para importar`);
        sizeForBatchRows(Math.min(Math.max(drafts.length, BATCH_VISIBLE_ROWS), BATCH_VISIBLE_ROWS)).catch(
          () => undefined,
        );
      } catch (e) {
        setError(String(e));
        setLoginDrafts([]);
        setLoginFileName(null);
      }
    };
    reader.onerror = () => setError("No pude leer el CSV");
    reader.readAsText(file);
  }

  async function onImportLogins() {
    if (!loginDrafts.length) return;
    setBusy(true);
    setError(null);
    try {
      const result = await invoke<{
        imported: number;
        skipped: number;
        message: string;
      }>("import_logins", {
        rows: loginDrafts.map((d) => ({
          slug: d.slug,
          name: d.name,
          url: d.url,
          username: d.username,
          password: d.password,
          notes: d.notes || null,
        })),
        replaceAll: loginReplaceAll,
      });
      setToast(result.message);
      setLoginDrafts([]);
      setLoginFileName(null);
      if (loginFileRef.current) loginFileRef.current.value = "";
      setPanel("list");
      setQuery("personal/logins");
      await runSearch("personal/logins");
      await refreshStatus();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function onClearToken() {
    setBusy(true);
    setError(null);
    try {
      await invoke("clear_token");
      setToken("");
      setToast("Token borrado del keyring");
      await refreshStatus();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function openDiag() {
    setPanel("diag");
    setError(null);
    try {
      setDiag(await invoke<Diagnostics>("diagnostics"));
    } catch (e) {
      setError(String(e));
    }
  }

  /**
   * The Avisos panel. Opening it does **not** clear the counter: "I looked" and
   * "I have no more pending" are different things, and the widget is meant to
   * keep the ones you have not dealt with yet. Clearing is the explicit button.
   */
  async function openNotices() {
    setPanel("notices");
    setError(null);
    try {
      await refreshNotices();
    } catch (e) {
      setError(String(e));
    }
  }

  async function markNoticesSeen() {
    setError(null);
    try {
      // Rust returns the fresh list, so panel and counter move together.
      setNotices(await invoke<Notices>("notices_mark_seen"));
      setToast("Avisos marcados como vistos");
    } catch (e) {
      setError(String(e));
    }
  }

  async function copyReport() {
    if (!diag) return;
    try {
      // Goes through Rust: the clipboard plugin is already registered there,
      // so the frontend needs no extra binding.
      await invoke("copy_to_clipboard", { text: buildReport(diag) });
      setToast("Informe copiado al portapapeles");
    } catch (e) {
      setError(String(e));
    }
  }

  async function copyNoticePath(path: string) {
    try {
      await invoke("copy_to_clipboard", { text: path });
      setToast("Ruta copiada");
    } catch (e) {
      setError(String(e));
    }
  }

  const seal = status?.seal;
  const docked = dock !== null && !expanded;

  /** The header LEDs, straight from the backend: `config.toggles[]` plus what
   *  each probe sees. Nothing in this file decides what a LED *means*. */
  const leds = status?.toggles ?? [];
  const grantAsk = !!grant;
  const grantLeft = grant ? grantCountdown(grant.expiresAt, nowSecs) : "";

  /**
   * The tooltip: what the probe saw, what a click does, what the `alt` gesture
   * does. Every line comes from the published catalog, so a LED added from
   * Ajustes documents itself without a line of UI copy.
   */
  const ledTitle = (led: ToggleView) => {
    const dir: "on" | "off" = led.state.on ? "off" : "on";
    const lines = [
      `${led.label} · ${led.state.detail}`,
      `Click = ${dir} · ${dir === "on" ? led.docs.on : led.docs.off}`,
    ];
    if (led.alt && led.docs.alt) lines.push(`Ctrl/clic derecho = ${led.docs.alt}`);
    if (led.state.ask && grant) {
      lines.push(`Allow pendiente · ${grant.actor} · ${grant.path} (${grantLeft})`);
    }
    return lines.join("\n");
  };

  const openPendingGrant = (e?: ReactMouseEvent) => {
    e?.stopPropagation();
    if (!expandedRef.current) void expandPanel();
  };

  const grantChip = (variant: "bar" | "dock") =>
    grant ? (
      <button
        type="button"
        className={`grant-chip grant-chip-${variant}`}
        onClick={openPendingGrant}
        title={`${grant.actor} · ${grant.namespace}/${grant.path}\n${grant.cmd.join(" ")}\nQuedan ${grantLeft} para Allow / Deny`}
      >
        <span className="grant-chip-dot" aria-hidden />
        <span className="grant-chip-label">{t("Allow")}</span>
        <span className="grant-chip-time">{grantLeft}</span>
      </button>
    ) : null;

  const ledToggles = (variant: "bar" | "dock") => (
    <div
      className={[
        "led-toggles",
        `led-toggles-${variant}`,
        // From four LEDs up the row has to give a little to still fit the
        // floating bar / top dock (see `.led-toggles-compact`).
        leds.length >= 4 ? "led-toggles-compact" : "",
      ]
        .filter(Boolean)
        .join(" ")}
    >
      {leds.map((led) => (
        <motion.button
          key={led.id}
          type="button"
          className={[
            "led-toggle",
            led.state.on ? "on" : "off",
            led.state.warn ? "warn" : "",
            led.state.ask ? "ask" : "",
          ]
            .filter(Boolean)
            .join(" ")}
          disabled={busy || toggleBusy !== null || !led.state.live}
          whileTap={{ scale: 0.96 }}
          onClick={(e) => {
            e.stopPropagation();
            const mod = e.ctrlKey || e.metaKey || e.altKey || e.shiftKey;
            void fireToggle(led, mod && led.alt ? "alt" : "flip");
          }}
          onContextMenu={
            led.alt
              ? (e) => {
                  e.preventDefault();
                  e.stopPropagation();
                  void fireToggle(led, "alt");
                }
              : undefined
          }
          title={ledTitle(led)}
          aria-label={`${led.label} · ${led.docs.probe}`}
          aria-pressed={led.state.on}
        >
          <span className="led" aria-hidden />
          <span className="led-label">{led.label}</span>
        </motion.button>
      ))}
    </div>
  );

  const dragHandle = (variant: "bar" | "dock") => (
    <div
      className={`drag-handle drag-handle-${variant}`}
      data-tauri-drag-region
      title={t("Arrastrar widget")}
      aria-label={t("Mover widget")}
    >
      <span className="drag-grip" data-tauri-drag-region aria-hidden />
    </div>
  );

  /**
   * The counter, as a badge on the mark.
   *
   * On the mark and not beside the LEDs because this is the one spot that is
   * identical in the three shapes (docked rail, collapsed pill, open header),
   * and because it costs **zero layout**: the rail and the pill are sized for
   * the chrome they already had, so a counter that took room would resize the
   * widget — and a widget that grows when the committee writes is a widget that
   * breathes, which this one must not.
   *
   * It is decorative (`aria-hidden`, `pointer-events: none`): it cannot be a
   * button inside the brand button, and the count is announced in that button's
   * own `title` (`unseenHint`). The list itself is the Avisos tab.
   */
  const noticesBadge = () => {
    const unseen = notices?.unseen ?? 0;
    if (!unseen) return null;
    return (
      <span className="notice-badge" aria-hidden>
        {unseen > 99 ? "99+" : unseen}
      </span>
    );
  };

  /** What the brand's tooltip adds when there is something to read. */
  const unseenHint = () => {
    const unseen = notices?.unseen ?? 0;
    if (!unseen) return "";
    return ` · ${unseen} ${unseen === 1 ? "aviso" : "avisos"} sin ver`;
  };

  /**
   * The inline expanded row: one credential's fields, either new or being
   * edited. The password starts empty even when editing — it is never read into
   * the form, and leaving it empty means "leave the stored one alone".
   */
  const loginRowForm = (editing: boolean) => (
    <div className="login-row-form">
      <div className="login-form">
        <label>{t("Nombre")}<input
            value={loginForm.name}
            onChange={(e) => setLoginForm({ ...loginForm, name: e.target.value })}
            placeholder={t("GitHub")}
            spellCheck={false}
            autoFocus={!editing}
          />
        </label>
        <label>{t("URL / dominio")}<input
            value={loginForm.url}
            onChange={(e) => setLoginForm({ ...loginForm, url: e.target.value })}
            placeholder={t("github.com")}
            spellCheck={false}
          />
        </label>
        <label>{t("Usuario")}<input
            value={loginForm.username}
            onChange={(e) => setLoginForm({ ...loginForm, username: e.target.value })}
            spellCheck={false}
            autoComplete="off"
          />
        </label>
        <label>
          Password
          <input
            type="password"
            value={loginForm.password}
            onChange={(e) => setLoginForm({ ...loginForm, password: e.target.value })}
            placeholder={editing ? "(sin tocar)" : ""}
            spellCheck={false}
            autoComplete="new-password"
          />
        </label>
        <label>
          Slug (path)
          <input
            value={loginForm.slug}
            onChange={(e) => setLoginForm({ ...loginForm, slug: e.target.value })}
            placeholder={slugifyLogin(loginForm.name, loginForm.url) || "auto"}
            spellCheck={false}
          />
        </label>
        <label>{t("Notas")}<input
            value={loginForm.notes}
            onChange={(e) => setLoginForm({ ...loginForm, notes: e.target.value })}
            spellCheck={false}
          />
        </label>
      </div>
      <div className="settings-row">
        <motion.button
          type="button"
          className="primary"
          disabled={busy}
          whileTap={tap}
          onClick={onSaveLoginForm}
        >
          {editing ? "Guardar" : "Agregar a la lista"}
        </motion.button>
        <button type="button" className="ghost" disabled={busy} onClick={closeLoginRow}>{t("Cancelar")}</button>
      </div>
    </div>
  );

  // Nothing renders until the remembered spot is in place (see the restore
  // effect): the window is revealed from there, and a floating pill inside a
  // dock-sized window — even for a single frame — reads as a glitch.
  if (!ready) return null;

  return (
    <motion.div
      className={[
        "shell",
        expanded ? "expanded" : "collapsed",
        docked ? `docked dock-${dock}` : "floating",
        grantAsk ? "has-grant" : "",
      ].join(" ")}
      transition={spring.move}
      onPointerEnter={clearCollapseTimer}
      onPointerLeave={armCollapseTimer}
    >
      {/* One slot, two layers: the pill and the panel cross-fade in place, so
          one can leave while the other arrives. AnimatePresence "wait" used to
          queue them, which made the expand read as two separate steps. */}
      <div className="stage">
        <AnimatePresence initial={false}>
          {docked ? (
            <motion.div
              key="dock"
              ref={dockTabRef}
              className={["dock-tab", dock !== "top" && dockIsland ? "island" : ""]
                .filter(Boolean)
                .join(" ")}
              initial={{ opacity: 0, scale: 0.96 }}
              animate={{ opacity: 1, scale: 1 }}
              exit={{ opacity: 0, scale: 0.97 }}
              transition={spring.sheet}
            >
              {dragHandle("dock")}
              <button
                type="button"
                className="dock-open"
                // El escudo —y la palabra `edgePass` cuando está horizontal— es
                // la puerta al menú: un clic lo abre y otro lo cierra. Antes la
                // isla se abría con sólo pasar el puntero, y eso se comía el clic
                // de los LEDs, que es justo para lo que sirve la barra compacta.
                // En el dock superior no hay isla: abre el panel y listo.
                onClick={() => {
                  if (dock === "top") {
                    void expandPanel();
                    return;
                  }
                  traceUi(`isla → ${dockIsland ? "cerrar" : "abrir"}`);
                  setDockIsland((open) => !open);
                }}
                title={
                  dock === "top"
                    ? `Abrir edgePass${unseenHint()}`
                    : `Menú${dockIsland ? " (cerrar)" : ""}${unseenHint()}`
                }
              >
                <LogoMark className="mark" size={22} />
                {noticesBadge()}
                {dock === "top" ? (
                  <span className="dock-label dock-label-top">edgePass</span>
                ) : (
                  <span className="dock-label dock-label-side" aria-label={t("edgePass")}>
                    {Array.from("edgePass").map((ch, i) => (
                      <span key={`${ch}-${i}`} className="dock-letter">
                        {ch}
                      </span>
                    ))}
                  </span>
                )}
              </button>
              {grantChip("dock")}
              {ledToggles("dock")}
              {/* La isla no es sólo una barra más ancha: es el menú. Con el
                  puntero encima aparecen las pestañas del panel, así que ir a
                  Batch desde la pastilla es un clic y no "abrir y buscar". */}
              {dock !== "top" && dockIsland && (
                <div className="dock-menu">
                  {(
                    [
                      ["list", tabLabels().secrets],
                      ["batch", tabLabels().batch],
                      ["webPassword", tabLabels().credentials],
                      ["settings", tabLabels().config],
                      ["mcp", tabLabels().mcp],
                      ["notices", tabLabels().notices],
                      ["diag", tabLabels().logs],
                    ] as [Panel, string][]
                  )
                    // Without the two writer scripts the panel has nothing to
                    // show, so the menu does not offer it either.
                    .filter(([id]) => id !== "notices" || notices?.available !== false)
                    .map(([id, label]) => (
                    <button
                      key={id}
                      type="button"
                      className="dock-menu-item"
                      onClick={(e) => {
                        e.stopPropagation();
                        void openTab(id);
                      }}
                      title={label}
                    >
                      {label}
                      {id === "notices" && !!notices?.unseen && (
                        <span className="tab-count">{notices.unseen}</span>
                      )}
                    </button>
                  ))}
                </div>
              )}
            </motion.div>
          ) : (
            <motion.div
              key="main"
              className="main-col"
              initial={{ opacity: 0, y: 4 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0, y: -3 }}
              transition={{ ...spring.sheet, delay: 0.04 }}
            >
            <header className={`bar ${expanded ? "" : "bar-compact"}`}>
              {dragHandle("bar")}
              <button
                className="brand"
                onClick={toggle}
                type="button"
                aria-expanded={expanded}
                title={`${expanded ? "Colapsar" : "Expandir"}${unseenHint()}`}
              >
                {/* Same mark in both states: the header used to say "eP" while
                    the pill showed the shield, which reads as two apps. */}
                <LogoMark className="mark" size={22} />
                {noticesBadge()}
                <span className="name">edgePass</span>
              </button>

              <div className="actions" ref={actionsRef}>
                {expanded && !status?.service?.running && status?.service?.installed && (
                  <motion.button
                    className="pill-btn warn"
                    type="button"
                    disabled={busy}
                    whileTap={{ scale: 0.96 }}
                    onClick={async () => {
                      setBusy(true);
                      try {
                        await invoke("openbao_service_start");
                        setToast("Servicio OpenBao arrancado");
                        await bootstrap();
                      } catch (e) {
                        setError(String(e));
                      } finally {
                        setBusy(false);
                      }
                    }}
                    title={t("Iniciar servicio Windows OpenBao")}
                  >
                    Svc
                  </motion.button>
                )}
                {grantChip("bar")}
                {ledToggles("bar")}
                <motion.button
                  className="icon-btn"
                  type="button"
                  whileTap={{ scale: 0.9 }}
                  onClick={toggle}
                  title={expanded ? "Colapsar" : "Expandir / cerca de un borde se ancla"}
                >
                  {expanded ? "▾" : "▸"}
                </motion.button>
              </div>
            </header>

            <div className={`drawer ${expanded ? "open" : ""}`}>
              <nav className="tabs">
                {/* The strip scrolls instead of shrinking: a squeezed tab is a
                    bitten label, and the reindex ↻ beside it got cut too. */}
                <div
                  className="tabs-strip"
                  ref={stripRef}
                  // La rueda sobre la tira la mueve a lo ancho: con siete
                  // pestañas y una ventana de 460 no entran todas, y la barra
                  // nativa está oculta a propósito (a estética). Sin esto, las
                  // últimas sólo se alcanzaban con shift+rueda o trackpad.
                  onWheel={(e) => {
                    const el = e.currentTarget;
                    if (Math.abs(e.deltaY) <= Math.abs(e.deltaX)) return;
                    el.scrollLeft += e.deltaY;
                  }}
                >
                  <button
                    type="button"
                    className={panel === "list" ? "active" : ""}
                    onClick={() => setPanel("list")}
                    title={tabTitles().secrets}
                  >
                    {tabLabels().secrets}
                  </button>
                  <button
                    type="button"
                    className={panel === "batch" ? "active" : ""}
                    onClick={() => {
                      if (!ingest) {
                        void openPathInMeter("", "", { isNew: true });
                      } else {
                        setPanel("batch");
                        sizeForBatchRows(Math.max(batchRows.length, BATCH_VISIBLE_ROWS)).catch(
                          () => undefined,
                        );
                      }
                    }}
                    title={tabTitles().batch}
                  >
                    {tabLabels().batch}
                  </button>
                  <button
                    type="button"
                    className={panel === "webPassword" ? "active" : ""}
                    onClick={() => {
                      setPanel("webPassword");
                      loadLoginRows().catch(() => undefined);
                      sizeForBatchRows(BATCH_VISIBLE_ROWS).catch(() => undefined);
                    }}
                    title={tabTitles().credentials}
                  >
                    {tabLabels().credentials}
                  </button>
                  <button
                    type="button"
                    className={panel === "settings" ? "active" : ""}
                    onClick={openSettings}
                    title={tabTitles().config}
                  >
                    {tabLabels().config}
                  </button>
                  <button
                    type="button"
                    className={panel === "mcp" ? "active" : ""}
                    onClick={openMcp}
                    title={tabTitles().mcp}
                  >
                    {tabLabels().mcp}
                  </button>
                  {notices?.available !== false && (
                    <button
                      type="button"
                      className={panel === "notices" ? "active" : ""}
                      onClick={() => void openNotices()}
                      title={tabTitles().notices}
                    >
                      {tabLabels().notices}
                      {!!notices?.unseen && <span className="tab-count">{notices.unseen}</span>}
                    </button>
                  )}
                  <button
                    type="button"
                    className={panel === "diag" ? "active" : ""}
                    onClick={openDiag}
                    title={tabTitles().logs}
                  >
                    {tabLabels().logs}
                  </button>
                </div>
                <motion.button
                  className="icon-btn"
                  type="button"
                  disabled={busy}
                  whileTap={tap}
                  onClick={onSync}
                  title={t("Reindexar")}
                >
                  ↻
                </motion.button>
              </nav>

              <AnimatePresence initial={false}>
                {error && (
                  <motion.div
                    className="error"
                    role="alert"
                    initial={{ opacity: 0, height: 0 }}
                    animate={{ opacity: 1, height: "auto" }}
                    exit={{ opacity: 0, height: 0 }}
                    transition={spring.content}
                  >
                    {error}
                  </motion.div>
                )}
              </AnimatePresence>

              <AnimatePresence mode="wait">
                {panel === "list" && (
                  <motion.section
                    key="list"
                    className="body"
                    initial={{ opacity: 0, y: 6 }}
                    animate={{ opacity: 1, y: 0 }}
                    exit={{ opacity: 0, y: -4 }}
                    transition={spring.content}
                  >
                      <>
                        <div className="search-wrap">
                          <svg
                            className="search-glyph"
                            viewBox="0 0 16 16"
                            fill="none"
                            stroke="currentColor"
                            strokeWidth="1.6"
                            strokeLinecap="round"
                            aria-hidden="true"
                          >
                            <circle cx="7" cy="7" r="4.6" />
                            <path d="M10.5 10.5L14 14" />
                          </svg>
                          <input
                            ref={inputRef}
                            className="search"
                            value={query}
                            onChange={(e) => setQuery(e.target.value)}
                            onKeyDown={onSearchKey}
                            placeholder={t("M2M · infisi cb2 · path variable…")}
                            spellCheck={false}
                            autoComplete="off"
                            role="combobox"
                            aria-expanded={hits.length > 0}
                            aria-controls="hits"
                            aria-autocomplete="list"
                          />
                          {query.length > 0 && (
                            <button
                              type="button"
                              className="search-clear"
                              title={t("Limpiar búsqueda")}
                              aria-label={t("Limpiar búsqueda")}
                              onClick={() => {
                                setQuery("");
                                inputRef.current?.focus();
                              }}
                            >
                              ✕
                            </button>
                          )}
                        </div>
                        <div className="meta">
                          <span>{hits.length} visibles</span>
                          <span className="dot">·</span>
                          <span>{status?.indexed ?? 0} indexadas</span>
                          <span className="dot">·</span>
                          <span>click copiar · ✎ editar · ⌫ borrar</span>
                        </div>
                        <ul className="hits" id="hits" role="listbox" aria-label={t("Secretos M2M")}>
                          <AnimatePresence initial={false}>
                            {hits.length === 0 && (
                              <motion.li
                                className="empty"
                                initial={{ opacity: 0 }}
                                animate={{ opacity: 1 }}
                                transition={spring.content}
                              >
                                {!seal?.reachable
                                  ? bootMsg || "OpenBao offline — arranca el servicio OpenBao"
                                  : "Sin secretos indexados aún"}
                              </motion.li>
                            )}
                            {hits.map((hit, i) => (
                              <motion.li
                                key={`${hit.namespace}/${hit.path}/${hit.key}`}
                                ref={i === selected ? selectedRef : undefined}
                                className="hit-li"
                                role="presentation"
                                initial={{ opacity: 0, y: 4 }}
                                animate={{ opacity: 1, y: 0 }}
                                exit={{ opacity: 0, scale: 0.98 }}
                                transition={{
                                  ...spring.content,
                                  delay: Math.min(i, 8) * 0.015,
                                }}
                              >
                                <div className={`hit-row ${i === selected ? "selected" : ""}`}>
                                  <motion.button
                                    type="button"
                                    className="hit"
                                    disabled={busy}
                                    role="option"
                                    aria-selected={i === selected}
                                    whileTap={{ scale: 0.985, transition: spring.press }}
                                    onClick={() => onCopy(hit)}
                                    title={t("Copiar valor")}
                                  >
                                    <div className="hit-top">
                                      <span className="hit-key">{hit.key}</span>
                                      {hit.useCount > 0 && (
                                        <span className="badge">{hit.useCount}×</span>
                                      )}
                                      {hit.notes > 0 && (
                                        <span className="badge notes-badge" title={`${hit.notes} nota(s)`}>
                                          🗒 {hit.notes}
                                        </span>
                                      )}
                                    </div>
                                    <div className="hit-sub">
                                      <span className="ns">{displayNs(hit.namespace)}</span>
                                      <span className="path">{hit.path}</span>
                                    </div>
                                  </motion.button>
                                  <button
                                    type="button"
                                    className="hit-edit"
                                    title={t("Notas de este secreto (por qué se pidió, para qué)")}
                                    onClick={() => void openNotes(hit)}
                                  >
                                    🗒
                                  </button>
                                  <button
                                    type="button"
                                    className="hit-edit"
                                    disabled={busy}
                                    title={t("Editar en Batch (sobrescribir)")}
                                    onClick={() => void openPathInMeter(hit.namespace, hit.path)}
                                  >
                                    ✎
                                  </button>
                                  <button
                                    type="button"
                                    className="hit-edit hit-del"
                                    disabled={busy}
                                    title={t("Borrar esta variable")}
                                    onClick={() => void onDeleteSecret(hit)}
                                  >
                                    ⌫
                                  </button>
                                </div>
                                {noteTarget === hitKey(hit) && (
                                  <div className="hit-notes">
                                    <div className="hit-notes-list">
                                      {noteRows.length === 0 && (
                                        <span className="hint">{t("Sin notas todavía.")}</span>
                                      )}
                                      {noteRows.map((n) => (
                                        <div className="hit-note" key={n.id}>
                                          <span className={`hit-note-kind ${n.kind}`}>
                                            {n.kind === "auto" ? "auto" : n.actor || "nota"}
                                          </span>
                                          <span className="hit-note-body">{n.body}</span>
                                          <span className="hit-note-when">
                                            {n.createdAt.slice(0, 16).replace("T", " ")}
                                          </span>
                                        </div>
                                      ))}
                                    </div>
                                    <div className="hit-notes-add">
                                      <input
                                        value={noteDraft}
                                        placeholder={t("¿por qué se pidió? ¿para qué sirve?")}
                                        onChange={(e) => setNoteDraft(e.target.value)}
                                        onKeyDown={(e) => {
                                          if (e.key === "Enter") void saveNote(hit);
                                        }}
                                      />
                                      <button
                                        type="button"
                                        className="pill-btn"
                                        disabled={busy || !noteDraft.trim()}
                                        onClick={() => void saveNote(hit)}
                                      >{t("Guardar nota")}</button>
                                    </div>
                                  </div>
                                )}
                              </motion.li>
                            ))}
                          </AnimatePresence>
                        </ul>
                      </>
                  </motion.section>
                )}

                {panel === "batch" && ingest && (
                  <motion.section
                    key="batch"
                    className="body batch"
                    initial={{ opacity: 0, y: 8 }}
                    animate={{ opacity: 1, y: 0 }}
                    exit={{ opacity: 0, y: -5 }}
                    transition={spring.content}
                  >
                    <div className="batch-head">
                      <div style={{ flex: 1, minWidth: 0 }}>
                        <div className="batch-title">{ingest.title || "Meter · nuevo secreto"}</div>
                        <div className="path-edit-meta" style={{ marginTop: 6 }}>
                          <input
                            className="batch-key"
                            value={ingest.namespace === "" ? "." : ingest.namespace}
                            onChange={(e) => {
                              const v = e.target.value.trim() === "." ? "" : e.target.value;
                              setIngest({ ...ingest, namespace: v });
                            }}
                            placeholder={t("ns · .")}
                            spellCheck={false}
                            title={t("Namespace")}
                          />
                          <input
                            className="batch-key"
                            value={ingest.path}
                            onChange={(e) => setIngest({ ...ingest, path: e.target.value })}
                            placeholder={t("path")}
                            spellCheck={false}
                            title="Path"
                          />
                        </div>
                      </div>
                      <div className="meter-head-actions">
                        <span className="badge">{batchRows.length} {batchRows.length === 1 ? "variable" : "variables"}</span>
                        <motion.button
                          type="button"
                          className={[
                            "meter-save",
                            showMeterJson ? "load" : "",
                            busy ? "busy" : "",
                            meterSaveFlash ? "ok" : "",
                          ]
                            .filter(Boolean)
                            .join(" ")}
                          disabled={
                            busy || (showMeterJson && !meterJsonText.trim())
                          }
                          whileTap={{ scale: 0.92 }}
                          onClick={() => {
                            if (showMeterJson) void onApplyMeterJson();
                            else void onSaveBatch();
                          }}
                          title={
                            showMeterJson
                              ? "Cargar JSON → filas editables (aún no guarda en vault)"
                              : "Guardar en vault (Enter) · no auto-save"
                          }
                          aria-label={
                            showMeterJson
                              ? "Cargar JSON en Batch"
                              : "Guardar en vault"
                          }
                        >
                          <AnimatePresence mode="wait" initial={false}>
                            {showMeterJson ? (
                              <motion.svg
                                key="refresh"
                                className="meter-save-svg"
                                viewBox="0 0 36 36"
                                aria-hidden
                                initial={{ opacity: 0, rotate: -40 }}
                                animate={{ opacity: 1, rotate: 0 }}
                                exit={{ opacity: 0, rotate: 40 }}
                                transition={{ duration: 0.18 }}
                              >
                                <circle
                                  className="meter-save-ring"
                                  cx="18"
                                  cy="18"
                                  r="14"
                                />
                                <path
                                  className="meter-save-refresh"
                                  d="M24.5 13.2a7.2 7.2 0 1 0 1.1 7.3"
                                  fill="none"
                                  strokeLinecap="round"
                                />
                                <path
                                  className="meter-save-refresh"
                                  d="M24.5 10.2v4h-4"
                                  fill="none"
                                  strokeLinecap="round"
                                  strokeLinejoin="round"
                                />
                              </motion.svg>
                            ) : (
                              <motion.svg
                                key="check"
                                className="meter-save-svg"
                                viewBox="0 0 36 36"
                                aria-hidden
                                initial={{ opacity: 0, scale: 0.85 }}
                                animate={{ opacity: 1, scale: 1 }}
                                exit={{ opacity: 0, scale: 0.85 }}
                                transition={{ duration: 0.15 }}
                              >
                                <circle
                                  className="meter-save-ring"
                                  cx="18"
                                  cy="18"
                                  r="14"
                                />
                                <motion.path
                                  className="meter-save-check"
                                  d="M11 18.5 L16 23.5 L25 13.5"
                                  fill="none"
                                  strokeLinecap="round"
                                  strokeLinejoin="round"
                                  initial={false}
                                  animate={
                                    meterSaveFlash
                                      ? { pathLength: 1, opacity: 1 }
                                      : {
                                          pathLength: 1,
                                          opacity: busy ? 0 : 0.55,
                                        }
                                  }
                                  transition={
                                    meterSaveFlash
                                      ? {
                                          duration: 0.32,
                                          ease: [0.32, 0.72, 0, 1],
                                        }
                                      : { duration: 0.15 }
                                  }
                                />
                              </motion.svg>
                            )}
                          </AnimatePresence>
                        </motion.button>
                      </div>
                    </div>

                    {showMeterJson ? (
                      <div className="meter-json">
                        <textarea
                          className="meter-json-input"
                          value={meterJsonText}
                          onChange={(e) => setMeterJsonText(e.target.value)}
                          onKeyDown={(e) => {
                            if ((e.ctrlKey || e.metaKey) && e.key === "Enter") {
                              e.preventDefault();
                              void onApplyMeterJson();
                            }
                          }}
                          placeholder={`{\n  "kvpath": "projects/…",\n  "API_KEY": "…",\n  "DB_URL": "…"\n}`}
                          spellCheck={false}
                          rows={7}
                          aria-label={t("Pegar JSON en Batch")}
                          autoFocus
                        />
                        <p className="meter-json-hint">
                          ↻ carga filas temporales · ✓ guarda en OpenBao
                        </p>
                      </div>
                    ) : (
                      <>
                    <div className="batch-table" role="table">
                      <div className="batch-row head" role="row">
                        <span>{t("Variable")}</span>
                        <span>{t("Valor")}</span>
                      </div>
                      {batchRows
                        .slice(
                          batchPage * BATCH_VISIBLE_ROWS,
                          batchPage * BATCH_VISIBLE_ROWS + BATCH_VISIBLE_ROWS,
                        )
                        .map((row, i) => {
                          const idx = batchPage * BATCH_VISIBLE_ROWS + i;
                          return (
                            <motion.div
                              key={`${ingest.id}-${row.key}-${idx}`}
                              className="batch-row"
                              role="row"
                              initial={{ opacity: 0, y: 6 }}
                              animate={{ opacity: 1, y: 0 }}
                              transition={{ ...spring.content, delay: i * 0.025 }}
                            >
                              <input
                                className="batch-key"
                                value={row.key}
                                onChange={(e) => {
                                  const next = [...batchRows];
                                  next[idx] = { ...next[idx]!, key: e.target.value };
                                  setBatchRows(next);
                                }}
                                onKeyDown={(e) => {
                                  if (e.key === "Enter") {
                                    e.preventDefault();
                                    void onSaveBatch();
                                  }
                                }}
                                spellCheck={false}
                              />
                              <input
                                className="batch-val"
                                type={maskedRows.has(idx) ? "password" : "text"}
                                autoComplete="off"
                                value={row.value}
                                placeholder={t("valor / placeholder… · Enter guarda")}
                                onChange={(e) => {
                                  const next = [...batchRows];
                                  next[idx] = { ...next[idx]!, value: e.target.value };
                                  setBatchRows(next);
                                }}
                                onKeyDown={(e) => {
                                  if (e.key === "Enter") {
                                    e.preventDefault();
                                    void onSaveBatch();
                                  }
                                }}
                                spellCheck={false}
                              />
                              <button
                                type="button"
                                className="ghost batch-act"
                                disabled={!row.value}
                                onClick={() => onToggleRowMask(idx)}
                                title={maskedRows.has(idx) ? "Mostrar valor" : "Ocultar valor"}
                                aria-label={
                                  maskedRows.has(idx)
                                    ? `Mostrar ${row.key.trim() || "valor"}`
                                    : `Ocultar ${row.key.trim() || "valor"}`
                                }
                              >
                                {maskedRows.has(idx) ? "ver" : "ocultar"}
                              </button>
                              <button
                                type="button"
                                className="ghost batch-act"
                                disabled={!row.value}
                                onClick={() => void onCopyRow(row)}
                                title={t("Copiar valor")}
                                aria-label={`Copiar ${row.key.trim() || "valor"}`}
                              >
                                ⧉
                              </button>
                              <motion.button
                                type="button"
                                className="ghost batch-act"
                                whileTap={tap}
                                animate={
                                  pulseRow === idx
                                    ? { scale: [1, 1.32, 1], color: ["inherit", "#8fd6ff", "inherit"] }
                                    : { scale: 1 }
                                }
                                transition={
                                  pulseRow === idx
                                    ? { duration: 0.5, times: [0, 0.35, 1] }
                                    : spring.content
                                }
                                onClick={() => void onSuggestPassphrase(idx)}
                                title={t("Sugerir contraseña")}
                                aria-label={`${t("Sugerir contraseña")} para ${row.key.trim() || "la variable"}`}
                              >
                                ✦
                              </motion.button>
                            </motion.div>
                          );
                        })}
                    </div>

                    {batchRows.length > BATCH_VISIBLE_ROWS && (
                      <div className="pager">
                        <button
                          type="button"
                          className="ghost"
                          disabled={batchPage <= 0}
                          onClick={() => setBatchPage((p) => Math.max(0, p - 1))}
                        >
                          ←
                        </button>
                        <span>
                          {batchPage + 1}/{Math.ceil(batchRows.length / BATCH_VISIBLE_ROWS)} ·{" "}
                          {batchRows.length} {batchRows.length === 1 ? "variable" : "variables"}
                        </span>
                        <button
                          type="button"
                          className="ghost"
                          disabled={(batchPage + 1) * BATCH_VISIBLE_ROWS >= batchRows.length}
                          onClick={() => setBatchPage((p) => p + 1)}
                        >
                          →
                        </button>
                      </div>
                    )}
                      </>
                    )}

                    <div className="settings-row meter-footer">
                      <button
                        type="button"
                        className="ghost"
                        onClick={() => {
                          const next = !showMeterJson;
                          setShowMeterJson(next);
                          sizeForBatchRows(
                            Math.max(batchRows.length, BATCH_VISIBLE_ROWS),
                            { jsonPaste: next },
                          ).catch(() => undefined);
                        }}
                        title={t("Pegar objeto JSON con kvpath + campos")}
                      >
                        {showMeterJson ? "Ocultar JSON" : "Pegar JSON"}
                      </button>
                      <button
                        type="button"
                        className="ghost"
                        disabled={!batchRows.some((r) => r.key.trim())}
                        onClick={() => void onCopyBlock()}
                        title={t("Copiar todas las entradas como líneas KEY=valor")}
                      >
                        ⧉ copiar todo
                      </button>
                      <button
                        type="button"
                        className="ghost"
                        disabled={!batchRows.length}
                        onClick={onToggleAllMask}
                        title={t("Mostrar u ocultar todos los valores")}
                      >
                        {maskedRows.size ? "ver todo" : "ocultar todo"}
                      </button>
                      <button
                        type="button"
                        className="ghost"
                        onClick={() => {
                          setBatchRows([...batchRows, { key: "", value: "" }]);
                          setBatchPage(Math.floor(batchRows.length / BATCH_VISIBLE_ROWS));
                        }}
                      >
                        + variable
                      </button>
                      <label className="check">
                        <input
                          type="checkbox"
                          checked={batchReplaceAll}
                          onChange={(e) => setBatchReplaceAll(e.target.checked)}
                        />{t("Reemplazar path")}</label>
                      
                      <button
                        type="button"
                        className="ghost"
                        onClick={() => {
                          setPanel("list");
                          setIngest(null);
                          setBatchPage(0);
                          setMeterSaveFlash(false);
                        }}
                      >{t("Descartar")}</button>
                    </div>

                    {passFirstRun !== null && (
                      <div
                        role="dialog"
                        aria-modal="true"
                        aria-label={t("Contraseñas sugeridas")}
                        style={{
                          position: "absolute",
                          inset: 0,
                          background: "var(--material-strong, rgba(20,20,22,0.86))",
                          display: "flex",
                          alignItems: "center",
                          justifyContent: "center",
                          padding: 18,
                          zIndex: 40,
                        }}
                      >
                        <div className="group" style={{ maxWidth: 380 }}>
                          <p className="group-title">{t("Contraseñas sugeridas")}</p>
                          <p className="hint">
                            {t("Primera vez. Elegí cómo se arma la frase y queda guardado. Después se cambia en Ajustes.")}
                          </p>
                          <label>
                            {t("Palabras")}
                            <select
                              value={passOpts.words}
                              onChange={(e) =>
                                setPassOpts({ ...passOpts, words: Number(e.target.value) })
                              }
                            >
                              {[3, 4, 5, 6, 7, 8, 9, 10].map((n) => (
                                <option key={n} value={n}>{n}</option>
                              ))}
                            </select>
                          </label>
                          <label>
                            {t("Separador")}
                            <select
                              value={passOpts.separator}
                              onChange={(e) =>
                                setPassOpts({ ...passOpts, separator: e.target.value })
                              }
                            >
                              <option value="-">-</option>
                              <option value="_">_</option>
                              <option value=".">.</option>
                              <option value=" ">{t("espacio")}</option>
                              <option value="">{t("sin separador")}</option>
                            </select>
                          </label>
                          <label className="check">
                            <input
                              type="checkbox"
                              checked={passOpts.compatible}
                              onChange={(e) =>
                                setPassOpts({ ...passOpts, compatible: e.target.checked })
                              }
                            />{t("Solo letras y números")}</label>
                          <p className="hint">
                            {(() => {
                              const bits = passOpts.words * Math.log2(7411);
                              return `${t("Fuerza aproximada")}: ${bits.toFixed(0)} bits`;
                            })()}
                          </p>
                          <div className="settings-row">
                            <motion.button
                              type="button"
                              className="primary"
                              whileTap={tap}
                              onClick={() => {
                                void (async () => {
                                  await onSavePassphrase();
                                  const target = passFirstRun;
                                  setPassFirstRun(null);
                                  if (target !== null) await generateInto(target);
                                })();
                              }}
                            >{t("Guardar y usar")}</motion.button>
                            <button
                              type="button"
                              className="ghost"
                              onClick={() => setPassFirstRun(null)}
                            >{t("Cancelar")}</button>
                          </div>
                        </div>
                      </div>
                    )}
                  </motion.section>
                )}

                {panel === "webPassword" && (
                  <motion.section
                    key="webPassword"
                    className="body form logins"
                    initial={{ opacity: 0, y: 6 }}
                    animate={{ opacity: 1, y: 0 }}
                    exit={{ opacity: 0, y: -4 }}
                    transition={spring.content}
                  >
                    <p className="hint">
                      Credenciales web (usuario/password) en{" "}
                      <code>personal/logins/&#123;slug&#125;</code>. Busca por sitio, usuario o dominio; click
                      en user/•••• copia.
                    </p>

                    <div className="login-toolbar">
                      <input
                        className="login-search"
                        value={loginQuery}
                        onChange={(e) => setLoginQuery(e.target.value)}
                        placeholder={t("Buscar sitio, usuario o dominio…")}
                        spellCheck={false}
                        aria-label={t("Buscar credencial")}
                      />
                      <motion.button
                        type="button"
                        className={loginOpen && !loginEditPath ? "ghost" : "primary"}
                        whileTap={tap}
                        disabled={busy}
                        onClick={() =>
                          loginOpen && !loginEditPath ? closeLoginRow() : openLoginRow(null)
                        }
                        title={t("Agregar una credencial a la lista")}
                      >
                        + Agregar
                      </motion.button>
                    </div>

                    {loginOpen && !loginEditPath && loginRowForm(false)}

                    {(() => {
                      if (loginBusy && !loginRows.length) {
                        return <p className="hint">{t("Leyendo credenciales del vault…")}</p>;
                      }
                      if (!loginRows.length) {
                        return (
                          <p className="hint">{t("Sin credenciales aún. «+ Agregar» crea la primera (o importa un CSV abajo).")}</p>
                        );
                      }
                      const shown = loginRows.filter((r) => loginMatches(r, loginQuery));
                      if (!shown.length) {
                        return <p className="hint">Nada coincide con «{loginQuery.trim()}».</p>;
                      }
                      return (
                        <ul className="login-list">
                          {shown.map((row) =>
                            loginEditPath === row.path ? (
                              <li key={row.path} className="login-item login-item-editing">
                                {loginRowForm(true)}
                              </li>
                            ) : (
                              <li key={row.path} className="login-item">
                                <div className="login-item-main">
                                  <div className="login-item-id">
                                    <strong>{row.name || row.slug}</strong>
                                    {row.url && <span className="login-meta">{row.url}</span>}
                                    {row.username && (
                                      <span className="login-meta login-user">{row.username}</span>
                                    )}
                                  </div>
                                  <div className="login-item-actions">
                                    {row.username && (
                                      <button
                                        type="button"
                                        className="ghost"
                                        disabled={busy}
                                        title={t("Copiar usuario")}
                                        onClick={() =>
                                          void onCopy({
                                            namespace: "",
                                            path: row.path,
                                            key: "username",
                                            useCount: 0,
                                            lastUsedAt: null,
                                            notes: 0,
                                            score: 0,
                                          })
                                        }
                                      >
                                        user
                                      </button>
                                    )}
                                    {row.hasPassword && (
                                      <button
                                        type="button"
                                        className="ghost login-pass"
                                        disabled={busy}
                                        title={t("Copiar password")}
                                        onClick={() =>
                                          void onCopy({
                                            namespace: "",
                                            path: row.path,
                                            key: "password",
                                            useCount: 0,
                                            lastUsedAt: null,
                                            notes: 0,
                                            score: 0,
                                          })
                                        }
                                      >
                                        ••••••••
                                      </button>
                                    )}
                                    <button
                                      type="button"
                                      className="hit-edit"
                                      disabled={busy}
                                      title={t("Editar")}
                                      onClick={() => void onEditLogin(row.path)}
                                    >
                                      ✎
                                    </button>
                                  </div>
                                </div>
                              </li>
                            ),
                          )}
                        </ul>
                      );
                    })()}

                    <div className="login-csv">
                      <p className="group-title">{t("Importar CSV")}</p>
                      <input
                        ref={loginFileRef}
                        type="file"
                        accept=".csv,text/csv"
                        hidden
                        onChange={(e) => onPickLoginCsv(e.target.files?.[0] ?? null)}
                      />
                      <div className="settings-row">
                        <motion.button
                          type="button"
                          className="ghost"
                          whileTap={tap}
                          disabled={busy}
                          onClick={() => loginFileRef.current?.click()}
                        >{t("Elegir CSV…")}</motion.button>
                        {loginFileName && <span className="hint">{loginFileName}</span>}
                      </div>
                      {loginDrafts.length > 0 && (
                        <>
                          <p className="hint">{loginDrafts.length} listos · se importan todos</p>
                          <label className="check">
                            <input
                              type="checkbox"
                              checked={loginReplaceAll}
                              onChange={(e) => setLoginReplaceAll(e.target.checked)}
                            />{t("Reemplazar path si ya existe")}</label>
                          <div className="settings-row">
                            <motion.button
                              type="button"
                              className="primary"
                              disabled={busy}
                              whileTap={tap}
                              onClick={onImportLogins}
                            >
                              Importar {loginDrafts.length}
                            </motion.button>
                            <button
                              type="button"
                              className="ghost"
                              onClick={() => {
                                setLoginDrafts([]);
                                setLoginFileName(null);
                                if (loginFileRef.current) loginFileRef.current.value = "";
                              }}
                            >{t("Limpiar")}</button>
                          </div>
                        </>
                      )}
                    </div>
                  </motion.section>
                )}

                {panel === "mcp" && (
                  <motion.div
                    key="mcp"
                    className="body mcp"
                    initial={{ opacity: 0 }}
                    animate={{ opacity: 1 }}
                    exit={{ opacity: 0 }}
                  >
                    <div className="batch-head">
                      <div className="batch-title">{t("Servidores MCP")}</div>
                      <span className="badge">
                        {mcpLive?.running ?? 0}/{mcpLive?.total ?? mcpDrafts.length}
                      </span>
                      <button
                        className="icon-btn"
                        type="button"
                        onClick={() => void refreshMcp()}
                        title={t("Releer el estado de los procesos")}
                      >
                        ↻
                      </button>
                    </div>
                    <p className="hint">
                      {mcpLive?.detail ??
                        "Los que tengan «auto» arrancan con edgePass — y edgePass arranca al iniciar sesión, así que eso es la persistencia."}
                    </p>

                    <div className="mcp-list">
                      {mcpDrafts.map((row, i) => {
                        const live = mcpLive?.servers.find((s) => s.id === row.id);
                        const url = live?.url || (row.publicUrl ? `${row.publicUrl}${row.path}` : "");
                        return (
                          <div className="mcp-row" key={row.id || `nuevo-${i}`}>
                            <div className="mcp-line">
                              <span className={live?.running ? "mcp-dot on" : "mcp-dot"} />
                              <input
                                className="mcp-field"
                                value={row.label}
                                placeholder={t("etiqueta del servidor (ej. mi-servidor)")}
                                onChange={(e) => editMcp(i, { label: e.target.value })}
                              />
                              <label
                                className="mcp-auto"
                                title={t("Arrancar con edgePass, y edgePass arranca al iniciar sesión")}
                              >
                                <input
                                  type="checkbox"
                                  checked={row.autostart}
                                  onChange={(e) => editMcp(i, { autostart: e.target.checked })}
                                />
                                auto
                              </label>
                              <button
                                className="icon-btn"
                                type="button"
                                disabled={busy}
                                title={t("Arrancar")}
                                onClick={() => void mcpControl(row.id, "on")}
                              >
                                ▶
                              </button>
                              <button
                                className="icon-btn"
                                type="button"
                                disabled={busy}
                                title={t("Detener")}
                                onClick={() => void mcpControl(row.id, "off")}
                              >
                                ■
                              </button>
                              <button
                                className="icon-btn"
                                type="button"
                                disabled={busy}
                                title={t("Reiniciar")}
                                onClick={() => void mcpControl(row.id, "restart")}
                              >
                                ↻
                              </button>
                              <button
                                className="icon-btn"
                                type="button"
                                title={t("Quitar de la lista (se aplica al guardar)")}
                                onClick={() => setMcpDrafts((rows) => rows.filter((_, j) => j !== i))}
                              >
                                ×
                              </button>
                            </div>
                            {live?.detail && <div className="mcp-detail">{live.detail}</div>}
                            {/* Tres formas de ejecutar, y el panel nombra las tres.
                                Antes eran cinco campos sueltos sin etiqueta: el
                                modelo solo lo entendía quien lo escribió. */}
                            <div className="mcp-labelled">
                              <span className="mcp-label">{t("Ejecutable")}</span>
                              <input
                                className="mcp-field"
                                value={row.command}
                                placeholder="C:\\ruta\\servidor-mcp.exe"
                                onChange={(e) => editMcp(i, { command: e.target.value })}
                              />
                            </div>
                            <div className="mcp-labelled">
                              <span className="mcp-label">{t("Directorio")}</span>
                              <input
                                className="mcp-field"
                                value={row.cwd}
                                placeholder={t("directorio de trabajo (opcional)")}
                                onChange={(e) => editMcp(i, { cwd: e.target.value })}
                              />
                            </div>
                            <div className="mcp-labelled">
                              <span className="mcp-label">{t("Argumentos")}</span>
                              <input
                                className="mcp-field"
                                value={row.args.join(" ")}
                                placeholder={t("separados por espacio")}
                                onChange={(e) =>
                                  editMcp(i, { args: e.target.value.split(/\s+/).filter(Boolean) })
                                }
                              />
                            </div>
                            <div className="mcp-labelled">
                              <span className="mcp-label">{t("Imagen de contenedor")}</span>
                              <input
                                className="mcp-field"
                                value={row.image}
                                placeholder={t("ghcr.io/usuario/servidor:tag (opcional, en vez del ejecutable)")}
                                onChange={(e) => editMcp(i, { image: e.target.value })}
                              />
                            </div>
                            <div className="mcp-labelled">
                              <span className="mcp-label">{t("URL pública")}</span>
                              <input
                                className="mcp-field"
                                value={row.publicUrl}
                                placeholder={t("https://host:8443")}
                                onChange={(e) => editMcp(i, { publicUrl: e.target.value })}
                              />
                            </div>
                            {/* El nombre del túnel, no su URL: el routing y las
                                credenciales viven en ~/.cloudflared, que es el
                                contrato de Cloudflare. Con nombre acá, edgePass
                                arranca y para el conector junto con el server. */}
                            <div className="mcp-labelled">
                              <span className="mcp-label">{t("Túnel nombrado")}</span>
                              <input
                                className="mcp-field"
                                value={row.tunnel}
                                placeholder={t("opcional (cloudflared tunnel run …)")}
                                title="Nombre del túnel de Cloudflare que publica este servidor. edgePass lo arranca y lo para junto con el servidor, y el LED MCP sólo dice «encendido» si el conector está conectado."
                                onChange={(e) => editMcp(i, { tunnel: e.target.value })}
                              />
                            </div>
                            {url !== "" && (
                              <div className="mcp-line">
                                <code className="mcp-url">{url}</code>
                                <button
                                  className="icon-btn"
                                  type="button"
                                  title={t("Copiar la URL que va en el cliente")}
                                  onClick={() => void invoke("copy_to_clipboard", { text: url })}
                                >
                                  ⧉
                                </button>
                                {row.credentialCommand.trim() !== "" && (
                                  <button
                                    className="pill-btn"
                                    type="button"
                                    disabled={busy}
                                    title="Correr el comando que acuña el client id: solo el servidor puede emitirlo"
                                    onClick={() => void mcpCredential(row.id)}
                                  >
                                    client id
                                  </button>
                                )}
                              </div>
                            )}
                            {(live?.envKeys.length ?? 0) > 0 && (
                              <div className="mcp-detail">
                                env: {live?.envKeys.join(", ")}
                                {live != null && live.secretEnvKeys.length > 0
                                  ? ` · desde el vault: ${live.secretEnvKeys.join(", ")}`
                                  : ""}
                              </div>
                            )}
                            {/* Variables que el server recibe por entorno, leídas
                                del vault al arrancar. La fila declara *dónde*
                                vive el valor; «Regenerar» lo acuña ahí. */}
                            {row.secretEnv.map((entry, j) => (
                              <div className="mcp-line mcp-env" key={`${row.id}-env-${j}`}>
                                <input
                                  className="mcp-field mcp-field-senv"
                                  value={entry.key}
                                  placeholder={t("VARIABLE")}
                                  spellCheck={false}
                                  onChange={(e) => editMcpSecret(i, j, { key: e.target.value })}
                                />
                                <input
                                  className="mcp-field mcp-field-senv"
                                  value={entry.namespace}
                                  placeholder={t("ns")}
                                  spellCheck={false}
                                  onChange={(e) => editMcpSecret(i, j, { namespace: e.target.value })}
                                />
                                <input
                                  className="mcp-field"
                                  value={entry.path}
                                  placeholder={t("ruta en el vault (ej. personal/servidor)")}
                                  spellCheck={false}
                                  onChange={(e) => editMcpSecret(i, j, { path: e.target.value })}
                                />
                                <input
                                  className="mcp-field mcp-field-senv"
                                  value={entry.field}
                                  placeholder={t("CAMPO")}
                                  spellCheck={false}
                                  onChange={(e) => editMcpSecret(i, j, { field: e.target.value })}
                                />
                                <button
                                  className="pill-btn"
                                  type="button"
                                  disabled={
                                    busy ||
                                    !row.id.trim() ||
                                    !entry.key.trim() ||
                                    !entry.path.trim() ||
                                    !entry.field.trim()
                                  }
                                  title="Acuñar un valor nuevo, guardarlo en OpenBao (donde dice esta fila) y mostrarlo para copiar"
                                  onClick={() => void mcpRegenerate(row, entry.key.trim())}
                                >{t("Regenerar")}</button>
                                <button
                                  className="icon-btn"
                                  type="button"
                                  title={t("Quitar esta variable de la fila")}
                                  onClick={() => editMcpSecret(i, j, null)}
                                >
                                  ×
                                </button>
                              </div>
                            ))}
                            <div className="mcp-line">
                              <button
                                className="ghost"
                                type="button"
                                onClick={() =>
                                  editMcpSecret(i, -1, {
                                    key: "",
                                    namespace: "",
                                    path: "",
                                    field: "",
                                  })
                                }
                              >
                                + Variable desde el vault
                              </button>
                              {row.secretEnv.length > 0 && (
                                <span className="mcp-detail">
                                  «Regenerar» acuña, guarda en OpenBao y lo muestra una sola vez
                                </span>
                              )}
                            </div>
                            {mcpSecret != null && mcpSecret.id === row.id && (
                              <div className="mcp-line mcp-secret">
                                <span className="mcp-detail">
                                  {mcpSecret.key}: copialo ahora — no se vuelve a mostrar
                                </span>
                                <code className="mcp-url">{mcpSecret.value}</code>
                                <button
                                  className="icon-btn"
                                  type="button"
                                  title={t("Copiar el valor")}
                                  onClick={() =>
                                    void invoke("copy_to_clipboard", { text: mcpSecret.value })
                                  }
                                >
                                  ⧉
                                </button>
                              </div>
                            )}
                          </div>
                        );
                      })}
                      {mcpDrafts.length === 0 && (
                        <p className="hint">{t("Todavía no hay servidores MCP. Alcanza con un ejecutable y sus argumentos, o una imagen, o una URL pública. Con «auto» el servidor arranca junto con edgePass, y eso es la persistencia.")}</p>
                      )}
                    </div>

                    <div className="quit-row">
                      <button
                        className="pill-btn"
                        type="button"
                        onClick={() => setMcpDrafts((rows) => [...rows, blankServer()])}
                      >
                        + Agregar servidor
                      </button>
                      <button
                        className="pill-btn"
                        type="button"
                        onClick={() => {
                          const next = !showMcpJson;
                          setShowMcpJson(next);
                          // Opening it pre-fills with what is on screen: copy out
                          // is the other half of paste in.
                          if (next && !mcpJsonText.trim()) {
                            setMcpJsonText(JSON.stringify(mcpDrafts, null, 2));
                          }
                        }}
                        title={t("Pegar o copiar servidores como JSON")}
                      >
                        {showMcpJson ? t("Ocultar JSON") : t("JSON")}
                      </button>
                      <button
                        className="pill-btn"
                        type="button"
                        disabled={busy}
                        onClick={() => void saveMcp()}
                      >{t("Guardar")}</button>
                    </div>
                    {showMcpJson && (
                      <div className="meter-json">
                        <textarea
                          className="meter-json-input"
                          value={mcpJsonText}
                          onChange={(e) => setMcpJsonText(e.target.value)}
                          spellCheck={false}
                          placeholder={'[{"label":"mi-servidor","command":"C:\\\\ruta\\\\mcp.exe","args":["--port","9000"]}]'}
                          rows={8}
                        />
                        <div className="settings-row">
                          <button
                            className="pill-btn"
                            type="button"
                            disabled={!mcpJsonText.trim()}
                            onClick={() => {
                              try {
                                const parsed = parseMcpJson(mcpJsonText);
                                setMcpDrafts((rows) => [...rows, ...parsed]);
                                setMcpJsonText("");
                                setShowMcpJson(false);
                                setMcpMsg(
                                  `${parsed.length} ${parsed.length === 1 ? "servidor agregado" : "servidores agregados"}. Guardá para aplicarlos.`,
                                );
                              } catch (e) {
                                setMcpMsg(`JSON inválido: ${e instanceof Error ? e.message : String(e)}`);
                              }
                            }}
                          >{t("Agregar a la lista")}</button>
                          <button
                            className="ghost"
                            type="button"
                            disabled={!mcpDrafts.length}
                            onClick={() => {
                              void invoke("copy_to_clipboard", {
                                text: JSON.stringify(mcpDrafts, null, 2),
                              });
                              setMcpMsg(
                                `${mcpDrafts.length} ${mcpDrafts.length === 1 ? "servidor copiado" : "servidores copiados"} como JSON.`,
                              );
                            }}
                          >{t("Copiar JSON")}</button>
                        </div>
                        <p className="meter-json-hint">
                          {t("Un objeto o una lista. Campos: label, command, args, cwd, image, env, secretEnv, publicUrl, tunnel, path, autostart. Lo que falte toma el valor vacío.")}
                        </p>
                      </div>
                    )}
                    {mcpMsg && <pre className="mcp-msg">{mcpMsg}</pre>}
                  </motion.div>
                )}

                {panel === "settings" && (
                  <motion.section
                    key="settings"
                    className="body form"
                    initial={{ opacity: 0, y: 6 }}
                    animate={{ opacity: 1, y: 0 }}
                    exit={{ opacity: 0, y: -4 }}
                    transition={spring.content}
                  >
                    <div className="group">
                      <p className="group-title">{t("Vault")}</p>
                      <label>
                        OpenBao URL
                        <input value={baoAddr} onChange={(e) => setBaoAddr(e.target.value)} spellCheck={false} />
                      </label>
                      <label>
                        Mount
                        <input value={mount} onChange={(e) => setMount(e.target.value)} spellCheck={false} />
                      </label>
                      <label>
                        Namespaces
                        <input value={namespaces} onChange={(e) => setNamespaces(e.target.value)} spellCheck={false} />
                      </label>
                      <p className="hint">
                        <code>.</code> = root + todos los namespaces hijos · lista explícita acota (ej.{" "}
                        <code>dokploy</code>). Solo loopback (127.0.0.1 · localhost · ::1).
                      </p>
                    </div>

                    <div className="group">
                      <p className="group-title">{t("Acceso")}</p>
                      <label>
                        Init JSON
                        <input value={initPath} onChange={(e) => setInitPath(e.target.value)} spellCheck={false} />
                      </label>
                      <label>{t("Token manual")}<input
                          type="password"
                          value={token}
                          onChange={(e) => setToken(e.target.value)}
                          spellCheck={false}
                          placeholder={status?.hasToken ? "ya hay un token guardado" : "sin token"}
                        />
                      </label>
                      <label className="check">
                        <input type="checkbox" checked={autoUnseal} onChange={(e) => setAutoUnseal(e.target.checked)} />{t("Auto-unseal al usar")}</label>
                      <label className="check">
                        <input
                          type="checkbox"
                          checked={autoLoadToken}
                          onChange={(e) => setAutoLoadToken(e.target.checked)}
                        />{t("Cargar token del init")}</label>
                      <div className="settings-row">
                        <button
                          type="button"
                          className="ghost"
                          disabled={busy || !status?.hasToken}
                          onClick={onClearToken}
                        >{t("Borrar token guardado")}</button>
                      </div>
                    </div>

                    <div className="group">
                      <p className="group-title">{t("Inicio")}</p>
                      <p className="hint">{t("No es un servicio de Windows (la UI no puede vivir en Session 0): tarea al logon que espera al servicio OpenBao y lanza el widget una vez.")}</p>
                      <div className="settings-row">
                        <button
                          type="button"
                          className={autostart?.installed ? "ghost" : "primary"}
                          disabled={busy || autostartBusy}
                          onClick={async () => {
                            setAutostartBusy(true);
                            setError(null);
                            try {
                              if (autostart?.installed) {
                                const s = await invoke<{
                                  installed: boolean;
                                  message: string;
                                  exe: string;
                                  taskName: string;
                                }>("autostart_remove");
                                setAutostart(s);
                                setToast(s.message);
                              } else {
                                const s = await invoke<{
                                  installed: boolean;
                                  message: string;
                                  exe: string;
                                  taskName: string;
                                }>("autostart_install");
                                setAutostart(s);
                                setToast(s.message);
                              }
                            } catch (e) {
                              setError(String(e));
                            } finally {
                              setAutostartBusy(false);
                            }
                          }}
                        >
                          {autostartBusy
                            ? "…"
                            : autostart?.installed
                              ? "Quitar arranque al iniciar sesión"
                              : "Ligar arranque al iniciar sesión"}
                        </button>
                      </div>
                      <p className="hint">
                        {autostart
                          ? `${autostart.installed ? "Activo" : "Inactivo"} · ${autostart.message}`
                          : "…"}
                      </p>
                    </div>

                    <div className="group">
                      <p className="group-title">{t("Panel")}</p>
                      <label>{t("Auto-colapsar sin mouse (s · 0 = nunca)")}<input
                          type="number"
                          min={0}
                          value={autoCollapseIdleSecs}
                          onChange={(e) => setAutoCollapseIdleSecs(Number(e.target.value) || 0)}
                        />
                      </label>
                      <p className="hint">{t("Si el puntero sale del widget expandido, vuelve a la píldora tras N segundos.")}</p>
                    </div>

                    <div className="group">
                      <p className="group-title">{t("Toggles del header")}</p>
                      <p className="hint">{t("Cada botón del header es una fila: qué")}<strong>mira</strong> (estado) y qué{" "}
                        <strong>hace</strong> al click. Todo vive en <code>config.json</code> y el
                        catálogo lo publica Rust: un agente puede leerlo o reescribirlo por el pipe (
                        <code>{'{"op":"toggles"}'}</code>), sin tocar código.
                      </p>
                      {toggleDrafts.length === 0 && (
                        <p className="hint">{t("Sin toggles: el header queda sin botones.")}</p>
                      )}
                      {toggleDrafts.map((row, i) => (
                        <div className="toggle-row" key={`row-${i}`}>
                          <div className="toggle-row-top">
                            <input
                              className="toggle-row-label"
                              value={row.label}
                              maxLength={status?.catalog.maxLabelChars ?? 4}
                              placeholder={t("KV")}
                              spellCheck={false}
                              onChange={(e) => patchToggle(i, { label: e.target.value })}
                              title={t("Qué dice el botón (1-4 caracteres)")}
                            />
                            <select
                              value={row.probe}
                              title={probeOf(row.probe)?.doc ?? ""}
                              onChange={(e) => setDraftProbe(i, e.target.value)}
                            >
                              {(status?.catalog.probes ?? []).map((p) => (
                                <option key={p.id} value={p.id}>
                                  {p.id}
                                </option>
                              ))}
                            </select>
                            <button
                              type="button"
                              className="ghost toggle-row-btn"
                              disabled={i === 0}
                              onClick={() => moveToggle(i, -1)}
                              title={t("Subir")}
                            >
                              ↑
                            </button>
                            <button
                              type="button"
                              className="ghost toggle-row-btn"
                              disabled={i === toggleDrafts.length - 1}
                              onClick={() => moveToggle(i, 1)}
                              title={t("Bajar")}
                            >
                              ↓
                            </button>
                            <button
                              type="button"
                              className="ghost toggle-row-btn"
                              onClick={() => removeToggle(i)}
                              title={t("Quitar este toggle")}
                            >
                              ×
                            </button>
                          </div>
                          <div className="toggle-row-sides">
                            <label>{t("Al encender")}<select
                                value={row.on}
                                onChange={(e) => patchToggle(i, { on: e.target.value })}
                              >
                                {(status?.catalog.actions ?? []).map((a) => (
                                  <option key={a.id} value={a.id}>
                                    {a.id}
                                  </option>
                                ))}
                              </select>
                            </label>
                            <label>{t("Al apagar")}<select
                                value={row.off}
                                onChange={(e) => patchToggle(i, { off: e.target.value })}
                              >
                                {(status?.catalog.actions ?? []).map((a) => (
                                  <option key={a.id} value={a.id}>
                                    {a.id}
                                  </option>
                                ))}
                              </select>
                            </label>
                            <label>{t("Ctrl / clic derecho")}<select
                                value={row.alt ?? ""}
                                onChange={(e) => patchToggle(i, { alt: e.target.value || null })}
                              >
                                <option value="">— sin extra —</option>
                                {(status?.catalog.actions ?? []).map((a) => (
                                  <option key={a.id} value={a.id}>
                                    {a.id}
                                  </option>
                                ))}
                              </select>
                            </label>
                          </div>
                          <p className="hint">
                            {probeOf(row.probe)?.doc}
                            {row.on ? ` · enciende: ${actionDoc(row.on)}` : ""}
                            {row.off ? ` · apaga: ${actionDoc(row.off)}` : ""}
                          </p>
                        </div>
                      ))}
                      <div className="settings-row">
                        <button
                          type="button"
                          className="ghost"
                          disabled={
                            !status || toggleDrafts.length >= (status.catalog.maxToggles ?? 5)
                          }
                          onClick={addToggle}
                        >
                          ＋ Agregar un toggle
                        </button>
                        <span className="hint">
                          {toggleDrafts.length}/{status?.catalog.maxToggles ?? 5} · se aplican al
                          guardar
                        </span>
                      </div>
                    </div>

                    <div className="group">
                      <p className="group-title">{t("Sincronización")}</p>
                      <label>{t("Auto-sellar tras idle (s · 0 = nunca)")}<input
                          type="number"
                          min={0}
                          value={autoSealIdleSecs}
                          onChange={(e) => setAutoSealIdleSecs(Number(e.target.value) || 0)}
                        />
                      </label>
                      <p className="hint warn">
                        0 por defecto: sellar corta el vault a <strong>todos</strong> sus clientes, así que solo
                        debería pasar cuando tú lo pides.
                      </p>
                      <label className="check">
                        <input type="checkbox" checked={autoSync} onChange={(e) => setAutoSync(e.target.checked)} />{t("Reindexar al abrir")}</label>
                    </div>

                    <div className="group">
                      <p className="group-title">{t("Contraseñas sugeridas")}</p>
                      <p className="hint">
                        {t("Cómo se arma la frase que sugiere el botón ✦ del Meter. Se pregunta una vez y se guarda.")}
                      </p>
                      <label>
                        {t("Palabras")}
                        <select
                          value={passOpts.words}
                          onChange={(e) =>
                            setPassOpts({ ...passOpts, words: Number(e.target.value) })
                          }
                        >
                          {[3, 4, 5, 6, 7, 8, 9, 10].map((n) => (
                            <option key={n} value={n}>{n}</option>
                          ))}
                        </select>
                      </label>
                      <label>
                        {t("Separador")}
                        <select
                          value={passOpts.separator}
                          onChange={(e) => setPassOpts({ ...passOpts, separator: e.target.value })}
                        >
                          <option value="-">-</option>
                          <option value="_">_</option>
                          <option value=".">.</option>
                          <option value=" ">{t("espacio")}</option>
                          <option value="">{t("sin separador")}</option>
                        </select>
                      </label>
                      <label className="check">
                        <input
                          type="checkbox"
                          checked={passOpts.capitalize}
                          onChange={(e) => setPassOpts({ ...passOpts, capitalize: e.target.checked })}
                        />{t("Mayúsculas")}</label>
                      <label className="check">
                        <input
                          type="checkbox"
                          checked={passOpts.digits}
                          onChange={(e) => setPassOpts({ ...passOpts, digits: e.target.checked })}
                        />{t("Dígitos")}</label>
                      <label className="check">
                        <input
                          type="checkbox"
                          checked={passOpts.symbol}
                          disabled={passOpts.compatible}
                          onChange={(e) => setPassOpts({ ...passOpts, symbol: e.target.checked })}
                        />{t("Símbolo")}</label>
                      <label className="check">
                        <input
                          type="checkbox"
                          checked={passOpts.compatible}
                          onChange={(e) => setPassOpts({ ...passOpts, compatible: e.target.checked })}
                        />{t("Solo letras y números")}</label>
                      <p className="hint">
                        {(() => {
                          // Must match `passphrase.rs`: 7.411 palabras y 14 símbolos.
                          const bits =
                            passOpts.words * Math.log2(7411) +
                            (passOpts.digits ? 2 * Math.log2(10) : 0) +
                            (!passOpts.compatible && passOpts.symbol ? Math.log2(14) : 0);
                          return `${t("Fuerza aproximada")}: ${bits.toFixed(0)} bits`;
                        })()}
                        {passNote ? ` · ${passNote}` : ""}
                      </p>
                      <div className="settings-row">
                        <motion.button
                          type="button"
                          className="primary"
                          whileTap={tap}
                          onClick={() => void onSavePassphrase()}
                        >{passConfigured ? t("Guardar estilo") : t("Guardar y usar")}</motion.button>
                      </div>
                    </div>

                    {/* Pegado al pie: en una app que se usa en ráfagas, la acción
                        primaria no puede quedar tres pantallas abajo. */}
                    <div className="settings-row sticky-save">
                      <motion.button
                        type="button"
                        className="primary"
                        disabled={busy}
                        whileTap={tap}
                        onClick={onSaveSettings}
                      >{t("Guardar")}</motion.button>
                      <button type="button" className="ghost" disabled={busy} onClick={bootstrap}>{t("Reconectar")}</button>
                    </div>
                  </motion.section>
                )}

                {panel === "notices" && (
                  <motion.section
                    key="notices"
                    className="body form notices"
                    initial={{ opacity: 0, y: 6 }}
                    animate={{ opacity: 1, y: 0 }}
                    exit={{ opacity: 0, y: -4 }}
                    transition={spring.content}
                  >
                    <p className="hint">{t("Lo que el comité dejó en disco: el hook del servidor MCP (lo que se escribe por el board) y el vigilante de la carpeta (una nota copiada a mano, un archivo restaurado). El contador son las que todavía no marcaste como vistas.")}</p>
                    <div className="settings-row notices-head">
                      <span className="badge">{notices?.unseen ?? 0} sin ver</span>
                      <span className="notice-total">
                        de {notices?.total ?? 0}
                        {notices && notices.total > notices.items.length
                          ? ` · últimas ${notices.items.length}`
                          : ""}
                      </span>
                      <motion.button
                        type="button"
                        className="ghost"
                        whileTap={tap}
                        disabled={!notices?.unseen}
                        onClick={() => void markNoticesSeen()}
                      >{t("Marcar todo como visto")}</motion.button>
                      <button type="button" className="ghost" onClick={() => void openNotices()}>{t("Actualizar")}</button>
                    </div>
                    <ul className="notice-rows">
                      {(notices?.items ?? []).map((notice, i) => (
                        <li
                          key={`${notice.at}-${notice.path}`}
                          className={`notice-row ${i < (notices?.unseen ?? 0) ? "new" : ""}`}
                        >
                          <div className="notice-main">
                            <span className="notice-title">
                              <span
                                className={`notice-kind ${notice.kind === "board" ? "board" : ""}`}
                              >
                                {notice.kind || "?"}
                              </span>
                              <span className="notice-name" title={notice.path}>
                                {notice.name || notice.path}
                              </span>
                            </span>
                            <span className="notice-sub">
                              {noticeProject(notice.root)} · {notice.source || "?"}
                              {notice.tool ? ` · ${notice.tool}` : ""} · {noticeWhen(notice.at)}
                            </span>
                          </div>
                          <button
                            type="button"
                            className="icon-btn"
                            title={notice.path}
                            aria-label={t("Copiar la ruta")}
                            onClick={() => void copyNoticePath(notice.path)}
                          >
                            ⧉
                          </button>
                        </li>
                      ))}
                      {!notices?.items.length && (
                        <li className="notice-empty">{t("Todavía no hay avisos. Aparecen solos cuando el comité escribe.")}</li>
                      )}
                    </ul>
                  </motion.section>
                )}

                {panel === "diag" && (
                  <motion.section
                    key="diag"
                    className="body form diag"
                    initial={{ opacity: 0, y: 6 }}
                    animate={{ opacity: 1, y: 0 }}
                    exit={{ opacity: 0, y: -4 }}
                    transition={spring.content}
                  >
                    <p className="hint">{t("Qué binario corre, qué log escribe y qué ve del vault. Es lo primero que hace falta cuando algo no arranca.")}</p>
                    {diag ? (
                      <>
                        <dl className="diag-rows">
                          <div className="diag-row">
                            <dt>edgePass</dt>
                            <dd>{diag.version}</dd>
                          </div>
                          <div className="diag-row">
                            <dt>build</dt>
                            <dd>{diag.buildTime ?? "-"}</dd>
                          </div>
                          <div className="diag-row">
                            <dt>exe</dt>
                            <dd>{diag.exe}</dd>
                          </div>
                          <div className="diag-row">
                            <dt>webview2</dt>
                            <dd>{diag.webview2}</dd>
                          </div>
                          <div className="diag-row">
                            <dt>vault</dt>
                            <dd>{diag.seal.message}</dd>
                          </div>
                          <div className="diag-row">
                            <dt>runs</dt>
                            <dd>
                              {(diag.runs ?? []).length === 0
                                ? "(none)"
                                : (diag.runs ?? [])
                                    .map((r) => `${r.runId} ${r.state} exit=${r.code ?? "-"}`)
                                    .join(" · ")}
                            </dd>
                          </div>
                          <div className="diag-row">
                            <dt>servicio</dt>
                            <dd>{diag.service.state}</dd>
                          </div>
                          <div className="diag-row">
                            <dt>indexadas</dt>
                            <dd>{diag.indexed}</dd>
                          </div>
                          <div className="diag-row">
                            <dt>token</dt>
                            <dd>{diag.hasToken ? "cargado" : "sin token"}</dd>
                          </div>
                          <div className="diag-row">
                            <dt>config</dt>
                            <dd>{diag.configPath}</dd>
                          </div>
                          <div className="diag-row">
                            <dt>db</dt>
                            <dd>{diag.dbPath}</dd>
                          </div>
                          <div className="diag-row">
                            <dt>log</dt>
                            <dd>{diag.logPath}</dd>
                          </div>
                        </dl>
                        <p className="group-title">{t("Últimas líneas del log")}</p>
                        <label className="check">
                          <input
                            type="checkbox"
                            checked={showLogNoise}
                            onChange={(e) => setShowLogNoise(e.target.checked)}
                          />{t("Incluir el vigilante de acople")}</label>
                        <pre className="diag-log">
                          {(showLogNoise ? diag.logTail : withoutDockNoise(diag.logTail)) ||
                            "(vacío)"}
                        </pre>
                      </>
                    ) : (
                      <p className="hint">{t("Leyendo…")}</p>
                    )}
                    <div className="settings-row">
                      <motion.button
                        type="button"
                        className="primary"
                        disabled={!diag}
                        whileTap={tap}
                        onClick={copyReport}
                      >{t("Copiar informe")}</motion.button>
                      <button type="button" className="ghost" disabled={busy} onClick={openDiag}>{t("Refrescar")}</button>
                    </div>

                    {/* Two steps on purpose: the widget has no taskbar button and
                        no window frame, so a single stray click would close the
                        only handle on it. Closing never seals the vault — that
                        is its own button — so this stays a plain "salir". */}
                    <div className="quit-row">
                      <button
                        type="button"
                        className={`ghost ${confirmQuit ? "danger armed" : ""}`}
                        disabled={busy}
                        onClick={async () => {
                          if (!confirmQuit) {
                            setConfirmQuit(true);
                            return;
                          }
                          try {
                            await invoke("close_app");
                          } catch (e) {
                            setError(String(e));
                            setConfirmQuit(false);
                          }
                        }}
                      >
                        {confirmQuit ? "¿Seguro? Salir ahora" : "Salir de edgePass"}
                      </button>
                      <span className="hint">
                        {confirmQuit
                          ? "Esc para cancelar."
                          : "Cierra el widget. El vault sigue como esté."}
                      </span>
                    </div>
                  </motion.section>
                )}
              </AnimatePresence>
            </div>
          </motion.div>
        )}
        </AnimatePresence>
      </div>

      <AnimatePresence>
        {toast && (
          <motion.div
            className="toast"
            role="status"
            aria-live="polite"
            initial={{ opacity: 0, y: 10, scale: 0.96, filter: "blur(6px)" }}
            animate={{ opacity: 1, y: 0, scale: 1, filter: "blur(0px)" }}
            exit={{ opacity: 0, y: 8, scale: 0.98, filter: "blur(4px)" }}
            transition={spring.material}
          >
            {toast}
          </motion.div>
        )}
      </AnimatePresence>

      <AnimatePresence>
        {grant && expanded && (
          <motion.div
            className="openbao-prompt-overlay grant-prompt-overlay"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: 0.2 }}
          >
            <motion.div
              className="openbao-prompt grant-prompt"
              role="alertdialog"
              aria-modal="true"
              aria-labelledby="grant-prompt-title"
              initial={{ opacity: 0, y: 16, scale: 0.97, filter: "blur(6px)" }}
              animate={{ opacity: 1, y: 0, scale: 1, filter: "blur(0px)" }}
              exit={{ opacity: 0, y: 8, scale: 0.98, filter: "blur(4px)" }}
              transition={spring.material}
            >
              <div className="openbao-prompt-icon">
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                  <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" />
                </svg>
              </div>
              <div className="openbao-prompt-text">
                <div className="openbao-prompt-title" id="grant-prompt-title">
                  {grant.actor} quiere ejecutar un comando
                </div>
                <div className="openbao-prompt-desc">
                  {grant.namespace}/{grant.path}
                  {grant.keys.length ? ` · ${grant.keys.join(", ")}` : " · todas las keys"}
                  {" · "}queda {grantLeft}
                </div>
                <div className="grant-prompt-cmd">{grant.cmd.join(" ")}</div>
                {grant.reason ? (
                  <div className="grant-prompt-reason">{grant.reason}</div>
                ) : null}
              </div>
              <div className="openbao-prompt-actions grant-prompt-actions">
                <motion.button
                  className="openbao-prompt-btn ghost"
                  type="button"
                  disabled={grantBusy}
                  whileHover={{ scale: grantBusy ? 1 : 1.02 }}
                  whileTap={{ scale: grantBusy ? 1 : 0.98 }}
                  onClick={() => void onGrantDecision("deny")}
                >{t("Rechazar")}</motion.button>
                <motion.button
                  className="openbao-prompt-btn ghost"
                  type="button"
                  disabled={grantBusy}
                  whileHover={{ scale: grantBusy ? 1 : 1.02 }}
                  whileTap={{ scale: grantBusy ? 1 : 0.98 }}
                  onClick={() => void onGrantDecision("allow-2h")}
                >
                  2 horas
                </motion.button>
                <motion.button
                  className="openbao-prompt-btn primary"
                  type="button"
                  disabled={grantBusy}
                  whileHover={{ scale: grantBusy ? 1 : 1.02 }}
                  whileTap={{ scale: grantBusy ? 1 : 0.98 }}
                  onClick={() => void onGrantDecision("allow-once")}
                >
                  {grantBusy ? "…" : "Una vez"}
                </motion.button>
              </div>
            </motion.div>
          </motion.div>
        )}
      </AnimatePresence>

      {/* OpenBao start prompt */}
      <AnimatePresence>
        {showOpenBaoPrompt && (
          <motion.div
            className="openbao-prompt-overlay"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: 0.2 }}
          >
            <motion.div
              className="openbao-prompt"
              role="alertdialog"
              aria-modal="true"
              aria-labelledby="openbao-prompt-title"
              initial={{ opacity: 0, y: 16, scale: 0.97, filter: "blur(6px)" }}
              animate={{ opacity: 1, y: 0, scale: 1, filter: "blur(0px)" }}
              exit={{ opacity: 0, y: 8, scale: 0.98, filter: "blur(4px)" }}
              transition={spring.material}
            >
              <div className="openbao-prompt-icon">
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                  <rect x="3" y="11" width="18" height="11" rx="2" ry="2" />
                  <path d="M7 11V7a5 5 0 0 1 10 0v4" />
                </svg>
              </div>
              <div className="openbao-prompt-text">
                <div className="openbao-prompt-title" id="openbao-prompt-title">{t("OpenBao no está corriendo")}</div>
                <div className="openbao-prompt-desc">{t("El servicio de OpenBao está detenido. ¿Quieres que edgePass lo arranque?")}</div>
              </div>
              <div className="openbao-prompt-actions">
                <motion.button
                  className="openbao-prompt-btn primary"
                  type="button"
                  disabled={busyStartingOpenBao}
                  whileHover={{ scale: busyStartingOpenBao ? 1 : 1.02 }}
                  whileTap={{ scale: busyStartingOpenBao ? 1 : 0.98 }}
                  onClick={onOpenBaoStart}
                >
                  {busyStartingOpenBao ? "Arrancando…" : "Arrancar OpenBao"}
                </motion.button>
                <motion.button
                  className="openbao-prompt-btn ghost"
                  type="button"
                  onClick={() => setShowOpenBaoPrompt(false)}
                  whileHover={{ scale: 1.02 }}
                  whileTap={{ scale: 0.98 }}
                >{t("No, gracias")}</motion.button>
              </div>
            </motion.div>
          </motion.div>
        )}
      </AnimatePresence>
    </motion.div>
  );
}
