import {
  getCurrentWindow,
  LogicalPosition,
  LogicalSize,
  availableMonitors,
  currentMonitor,
  cursorPosition,
  monitorFromPoint,
  primaryMonitor,
  type Monitor,
} from "@tauri-apps/api/window";
import { invoke } from "@tauri-apps/api/core";

/* ─────────────────────────────────────────────────────────────────────────
 * Traza del pipeline de geometría
 *
 * Todo lo que decide el tamaño de la ventana —la medición del contenido, la
 * conversión CSS→lógico y los frames del spring— ocurre en el tiempo y no se
 * puede diagnosticar con una captura. `traceUi` acumula las líneas y las manda
 * al log de la app en tandas de 250 ms: barato para la UI, completo al leer.
 * ───────────────────────────────────────────────────────────────────────── */
const traceBuffer: string[] = [];
let traceTimer: number | null = null;

/** Una línea de traza con el reloj del webview (ms desde el arranque). */
export function traceUi(msg: string) {
  traceBuffer.push(`${performance.now().toFixed(0)}ms ${msg}`);
  if (traceTimer !== null) return;
  traceTimer = window.setTimeout(() => {
    traceTimer = null;
    const lines = traceBuffer.splice(0).join("\n");
    void invoke("ui_log", { msg: lines }).catch(() => undefined);
  }, 250);
}

const fmtRect = (r: Rect) =>
  `${Math.round(r.w)}x${Math.round(r.h)} @${Math.round(r.x)},${Math.round(r.y)}`;

export type DockEdge = "left" | "right" | "top";
export type Rect = { x: number; y: number; w: number; h: number };

/**
 * Where the pill was left, as persisted in `placement.json`.
 *
 * Relative on purpose: the monitor it lived on plus its centre as a fraction of
 * that monitor's work area. Absolute pixels would park the pill off screen the
 * moment a resolution or DPI changed, and this widget has no taskbar entry to
 * drag it back from.
 */
export type Placement = {
  monitor: string;
  /** `null` = free-floating pill. */
  dock: DockEdge | null;
  /** Centre of the collapsed pill, 0..1 across the work area. */
  x: number;
  y: number;
};

export const SNAP_PX = 56;
/** Pull farther than this to leave a dock (avoids instant re-stick). */
export const UNDOCK_PX = 96;

export const FLOAT_COLLAPSED = { w: 410, h: 56 };
/**
 * Width of the collapsed floating pill for the LEDs it has to hold.
 *
 * 410 is the chrome (grip, brand, arrow) plus three LEDs. From a fourth on the
 * row is compact but each pill still costs ~44 px, and the row never wraps —
 * wrapping would change the height — so the window grows with it and shrinks
 * back when a LED is removed. This is what makes "collapse to the LEDs present
 * or grow for the eight" true without a fixed guess in the middle.
 */
export function floatCollapsedWidth(leds: number): number {
  return FLOAT_COLLAPSED.w + Math.max(0, leds - 3) * 50;
}

export function floatCollapsedSize(leds: number): { w: number; h: number } {
  return { w: floatCollapsedWidth(leds), h: FLOAT_COLLAPSED.h };
}
export const FLOAT_EXPANDED = { w: 460, h: 600 };
/**
 * Floor for the side rail, **in logical window px** — not in CSS px, and those
 * are not the same number. That was the bug: on a 150 % display the window kept
 * 52 logical of *device* width, so the webview laid the rail out in ~35 CSS px,
 * `SYNC`/`AUTO` wrapped, the LED column outgrew the height and `KV` ended up on
 * top of the `edgePass` letters. The rail is therefore sized from what its
 * content asks for (see [`sizeDockTo`]); this is only the floor, the "looks like
 * a pill" width that a one-LED header still gets.
 */
export const DOCK_SIDE = { w: 52, h: 360 };
export const DOCK_TOP = { w: 280, h: 44 };
/** Widest the side rail becomes while the pointer rests on it (the island). */
export const DOCK_ISLAND_W = 148;
/** The size the side rail was last given, so snapped geometry stays in step. */
let sideSize: { w: number; h: number } | null = null;
/** Same for the top dock: it is a row, so it grows with the LEDs too. */
let topSize: { w: number; h: number } | null = null;
export const BATCH_BASE = { w: 560, h: 220 };
export const BATCH_ROW_H = 44;
/** Visible rows before pagination — window grows to fit these, not the full list. */
export const BATCH_VISIBLE_ROWS = 4;
/** Header + tabs + path + pager + footer (save lives in head). */
export const BATCH_CHROME = 268;
/** Extra height when Meter shows the JSON paste pane (textarea + hint). */
export const METER_JSON_EXTRA = 200;
/** Grow batch geometry a bit so footer/actions aren't clipped. */
export const BATCH_SIZE_PAD = 1.12;

/* ─────────────────────────────────────────────────────────────────────────
 * Geometry — pure functions, no side effects.
 * ───────────────────────────────────────────────────────────────────────── */

function clamp(v: number, min: number, max: number) {
  return Math.min(max, Math.max(min, v));
}

function monitorWorkArea(mon: Monitor): Rect {
  const scale = mon.scaleFactor || 1;
  const pos = mon.workArea.position;
  const size = mon.workArea.size;
  return {
    x: pos.x / scale,
    y: pos.y / scale,
    w: size.width / scale,
    h: size.height / scale,
  };
}

export async function getWindowLogic(): Promise<{
  x: number;
  y: number;
  w: number;
  h: number;
  scale: number;
  work: Rect;
  /** Name of the monitor the window sits on; "" when it cannot be resolved. */
  monitor: string;
}> {
  const win = getCurrentWindow();
  const scale = await win.scaleFactor();
  const pos = await win.outerPosition();
  const size = await win.outerSize();
  const lx = pos.x / scale;
  const ly = pos.y / scale;
  const lw = size.width / scale;
  const lh = size.height / scale;
  const cx = Math.round(pos.x + size.width / 2);
  const cy = Math.round(pos.y + size.height / 2);
  const mon = (await monitorFromPoint(cx, cy)) ?? (await currentMonitor()) ?? null;
  if (!mon) {
    return {
      x: lx,
      y: ly,
      w: lw,
      h: lh,
      scale,
      work: { x: 0, y: 0, w: 1920, h: 1080 },
      monitor: "",
    };
  }
  return {
    x: lx,
    y: ly,
    w: lw,
    h: lh,
    scale,
    work: monitorWorkArea(mon),
    monitor: mon.name ?? "",
  };
}

/**
 * How far the *pointer* is from a dock edge, in logical px — `Infinity` when the
 * OS cannot say.
 *
 * The shape of the problem this exists for: while the user drags, Windows runs
 * a native move loop that keeps repositioning the window from the offset it
 * captured when the drag began, so `outerPosition()` reports a place the pill is
 * no longer at (worse: the pill dropped *past* the screen edge reports as "far
 * from the edge" and would come back floating). The pointer is never stale, so
 * the drop is decided with it.
 */
export async function pointerEdgeDistance(
  edge: DockEdge,
  work: Rect,
  scale: number,
): Promise<number> {
  try {
    const c = await cursorPosition();
    const cx = c.x / (scale || 1);
    const cy = c.y / (scale || 1);
    if (edge === "left") return Math.max(0, cx - work.x);
    if (edge === "right") return Math.max(0, work.x + work.w - cx);
    return Math.max(0, cy - work.y);
  } catch {
    return Number.POSITIVE_INFINITY;
  }
}

/** Grow the window so the header row and the tab strip fit whole.
 *
 *  Why this has to ask: `.actions` (the LEDs) and `.tabs-strip` are
 *  `min-width: 0` with `overflow: hidden` / `auto`, and `App.css` squeezes the
 *  LEDs from four up. So their content never forces a width — a sixth LED or a
 *  sixth tab gets *bitten* instead of widening the window, which is exactly what
 *  the header and the tab strip were doing. `needW` is measured from the live
 *  DOM by the caller, so this stays right for any number of rows instead of
 *  hardcoding a number that goes stale on the next tab.
 *
 *  Height is untouched (`FLOAT_EXPANDED.h`): this only moves the right edge. */
/** Último ancho pedido para el panel: dos llamadas seguidas con la misma medida
 *  (el efecto y `expandPanel` la piden la una a la otra) se resuelven con una
 *  sola animación. */
let lastChromeW = 0;
/** Última medición de la chrome (CSS px): la necesita `sizeForBatchRows` para
 *  pedir el tamaño completo en **una** animación en vez de dos que se pisan. */
let lastChromeNeed = 0;
/** Alto que el panel se fijó a sí mismo (Batch y Credenciales, en CSS px). */
let panelHeightCss = 0;

/** Fija el alto que el panel quiere para sí (`sizeForBatchRows`). */
export function setPanelHeight(hCss: number) {
  panelHeightCss = Math.round(hCss);
}

export async function sizeForChrome(needW: number, opts: { hCss?: number } = {}) {
  const cur = await getWindowLogic();
  if (needW > 0) lastChromeNeed = needW;
  // `needW` viene del DOM en **CSS px** y `setSize` fija px de **cliente**: la
  // conversión es `dpr / escala` y nada más. Pedirlo por delta contra el rect
  // outer (que trae el marco invisible) hacía crecer la ventana en cada ajuste —
  // y con ella los LEDs y las pestañas se veían recortados aunque el número
  // "creciera".
  const k = (window.devicePixelRatio || 1) / (cur.scale || 1);
  const w = Math.min(
    Math.max(Math.round(needW * k), FLOAT_EXPANDED.w),
    Math.max(cur.work.w - 24, 320),
  );
  // El alto: el que el panel se fijó (Batch/Credenciales) o el genérico del
  // panel. Un solo dueño del alto evita que las dos rutas animen en el mismo
  // frame y la ventana quede con el alto del que llegó último.
  const h = Math.min(
    Math.round((opts.hCss ?? (panelHeightCss || FLOAT_EXPANDED.h)) * k),
    cur.work.h - 24,
  );
  // Ya está en el tamaño que se pide: no se anima. Se comparan **los dos ejes**
  // contra el viewport real, porque pasar de Batch (más alto) a Secrets tiene que
  // devolver el alto aunque el ancho no cambie.
  const curCss = document.documentElement.clientWidth || 0;
  const curCssH = document.documentElement.clientHeight || 0;
  if (
    w === lastChromeW &&
    Math.abs(curCss - w) < 2 &&
    Math.abs(curCssH - h) < 2 &&
    !isMorphing()
  ) {
    traceUi(`chrome skip (${w}x${h})`);
    return;
  }
  lastChromeW = w;
  traceUi(`chrome need=${Math.round(needW)} k=${k.toFixed(3)} → ${w}x${h}`);
  const x = clamp(cur.x, cur.work.x + 8, cur.work.x + cur.work.w - w - 8);
  const y = clamp(cur.y, cur.work.y + 8, cur.work.y + cur.work.h - h - 8);
  await animateTo({ x, y, w, h });
}

/**
 * Sizes the docked rail so its content is painted whole, keeping the edge it is
 * glued to.
 *
 * The units are the whole point: `need` comes from the DOM in **CSS px** and the
 * window is set in **logical px**, and on a scaled display those differ by the
 * monitor's factor (150 % on the display this was found on). So the conversion is
 * measured — the ratio between the webview's viewport and the window's own
 * logical size — instead of assumed, which also heals the case that caused the
 * bug: a rail sized on one display and left there after the window moved to a
 * display at another scale. It grows to fit and stops; a size that already fits
 * is a no-op, so it can never breathe on a poll.
 */
export async function sizeDockTo(
  edge: DockEdge,
  needCssW: number,
  needCssH: number,
  opts: { remember?: boolean } = {},
) {
  const cur = await getWindowLogic();
  const vw = document.documentElement.clientWidth || 0;
  const vh = document.documentElement.clientHeight || 0;
  if (vw <= 0 || vh <= 0) return;
  const k = (window.devicePixelRatio || 1) / (cur.scale || 1);
  if (edge === "top") {
    // The top dock is a row: it grows with the LEDs the same way the pill does,
    // and the height is the dock's own (`DOCK_TOP.h`). Centred on the point it
    // already occupies so a new LED opens the row symmetrically.
    const wantW = Math.max(needCssW, DOCK_TOP.w) * k;
    const w = Math.round(Math.min(wantW, cur.work.w - 8));
    const h = Math.round(DOCK_TOP.h * k);
    const center = cur.x + cur.w / 2;
    const x = Math.round(
      clamp(center - w / 2, cur.work.x, cur.work.x + cur.work.w - w),
    );
    const y = cur.work.y;
    const clientW = vw * k;
    const clientH = vh * k;
    if (Math.abs(clientW - w) < 2 && Math.abs(clientH - h) < 2) return;
    if (opts.remember !== false) topSize = { w, h };
    traceUi(`dockTo top need=${Math.round(needCssW)} k=${k.toFixed(3)} → ${w}x${h}`);
    await animateTo({ x, y, w, h });
    return;
  }

  // CSS px → logical window px, **as a delta**. The two are different units on
  // this machine and neither is 1:1 with the other: the webview reports
  // `devicePixelRatio` 2 while the window is scaled by 1.5, so a rail laid out
  // for one unit and sized in the other came out clipped. A delta also cancels
  // tao's invisible shadow frame (it is the same on both sides) and converges in
  // one step — an absolute size built from these ratios chases its own tail,
  // because resizing changes the very ratio it is computed from.
  // CSS px → px de **cliente** de la ventana. Se pide absoluto y no por delta:
  // `setSize` fija el tamaño de cliente, pero `getWindowLogic` lee el rect
  // *outer*, que arrastra el marco invisible de tao (~15 lógicos acá). Usar ese
  // rect como base sumaba el marco en cada ajuste: la ventana crecía un poco,
  // la verificación lo veía ancho, volvía a pedir… y el widget se redimensionaba
  // solo cada 5 s. `k` es la única constante que hace falta (dpr / escala).
  const wantW = Math.max(needCssW, DOCK_SIDE.w) * k;
  const wantH = needCssH * k;
  const w = Math.round(Math.min(Math.max(wantW, DOCK_SIDE.w), cur.work.w - 8));
  const h = Math.round(Math.min(Math.max(wantH, 120), cur.work.h - 16));
  traceUi(
    `dockTo edge=${edge} need=${Math.round(needCssW)}x${Math.round(needCssH)} vp=${vw}x${vh} ` +
      `k=${k.toFixed(3)} → ${w}x${h}${opts.remember === false ? " (isla)" : ""}`,
  );
  const x = edge === "left" ? cur.work.x : cur.work.x + cur.work.w - w;
  const y = clamp(
    cur.y,
    cur.work.y + 8,
    Math.max(cur.work.y + 8, cur.work.y + cur.work.h - h - 8),
  );
  // Sólo la forma **fina** se recuerda: `dockSize()` es lo que usa el snap al
  // colapsar, y si guardara el tamaño de la isla, minimizar dejaría el rail
  // ancho (compactado a medias). La isla es un estado, no un tamaño de reposo.
  if (opts.remember !== false) sideSize = { w, h };
  // ¿Ya está en ese tamaño? Se compara contra el tamaño de **cliente** (el
  // viewport, convertido), no contra `cur.w`: ese rect trae el marco invisible y
  // nunca coincide, así que cada poll de 5 s volvía a pedir el mismo tamaño y el
  // widget se redimensionaba solo ("resize a cada rato").
  const clientW = vw * k;
  const clientH = vh * k;
  if (
    Math.abs(clientW - w) < 2 &&
    Math.abs(clientH - h) < 2 &&
    Math.abs(cur.y - y) < 2
  ) {
    traceUi(`dockTo skip (ya cabe) client=${Math.round(clientW)}x${Math.round(clientH)}`);
    return;
  }
  await animateTo({ x, y, w, h });
}

/** Snapshot of the collapsed pill for `placement.json`.
 *
 *  The centre is taken from the size the *restore* will set (`dockSize` /
 *  `FLOAT_COLLAPSED`), not from the live `rect.w/h`: the window's outer rect
 *  carries tao's invisible shadow frame, which is ~15x9 logical px bigger than
 *  what `setSize` actually applies. Building the centre out of it would push the
 *  pill by that margin on *every* restart (it crawls across the screen). */
export function placementFrom(
  rect: Rect,
  work: Rect,
  dock: DockEdge | null,
  monitor: string,
  floatW: number = FLOAT_COLLAPSED.w,
): Placement {
  const size = dock ? dockSize(dock) : { w: floatW, h: FLOAT_COLLAPSED.h };
  return {
    monitor,
    dock,
    x: clamp((rect.x + size.w / 2 - work.x) / (work.w || 1), 0, 1),
    y: clamp((rect.y + size.h / 2 - work.y) / (work.h || 1), 0, 1),
  };
}

/**
 * The rect to open at, from a stored placement — `null` only when no monitor
 * can be resolved at all.
 *
 * The monitor is looked up by name (the display the pill lived on), falling
 * back to the primary one: a laptop undocked from its external screen then gets
 * the pill back on the built-in display instead of off screen.
 */
export async function geometryFromPlacement(
  p: Placement,
  floatW: number = FLOAT_COLLAPSED.w,
): Promise<{ dock: DockEdge | null; rect: Rect } | null> {
  const monitors = await availableMonitors().catch(() => [] as Monitor[]);
  const named = p.monitor ? monitors.find((m) => m.name === p.monitor) : undefined;
  const mon = named ?? (await primaryMonitor().catch(() => null)) ?? monitors[0] ?? null;
  if (!mon) return null;
  const work = monitorWorkArea(mon);
  // `dock` arrives from a JSON file: treat anything this version does not know
  // as "floating" rather than trusting the string.
  const dock: DockEdge | null =
    p.dock === "left" || p.dock === "right" || p.dock === "top" ? p.dock : null;

  if (dock) {
    const size = dockSize(dock);
    // The stored centre (x, y) is what the pill keeps when the edge is
    // remembered: only the axis across that edge can move, the other is pinned.
    const anchorY = work.y + p.y * work.h - size.h / 2;
    const anchorX = work.x + p.x * work.w - size.w / 2;
    const pos = dockPosition(dock, work, anchorY, anchorX);
    return { dock, rect: { ...pos, ...size } };
  }

  const w = Math.min(floatW, work.w);
  const h = Math.min(FLOAT_COLLAPSED.h, work.h);
  const rect = {
    x: clamp(work.x + p.x * work.w - w / 2, work.x + 8, work.x + work.w - w - 8),
    y: clamp(work.y + p.y * work.h - h / 2, work.y + 8, work.y + work.h - h - 8),
    w,
    h,
  };
  // Mirror the widget's own rule (see `trySnapEdge`): a pill left close to an
  // edge belongs to that edge. Deciding it *here* rather than letting the next
  // move event re-dock means the first frame on screen is already the final one
  // — otherwise the widget visibly glues itself to the edge right after
  // appearing, which is exactly the jump the hidden window exists to avoid.
  const edge = nearestEdge(rect.x, rect.y, rect.w, rect.h, work);
  if (!edge) return { dock: null, rect };
  const size = dockSize(edge);
  return { dock: edge, rect: { ...dockPosition(edge, work, rect.y, rect.x), ...size } };
}

/** Distance from window to each work-area edge (0 = touching / overlapping). */
export function edgeDistances(
  x: number,
  y: number,
  w: number,
  h: number,
  work: Rect,
): Record<DockEdge, number> {
  void h;
  return {
    left: Math.max(0, x - work.x),
    right: Math.max(0, work.x + work.w - (x + w)),
    top: Math.max(0, y - work.y),
  };
}

/** Prefer left/right; top only if clearly closer. */
export function nearestEdge(
  x: number,
  y: number,
  w: number,
  h: number,
  work: Rect,
  threshold = SNAP_PX,
): DockEdge | null {
  const d = edgeDistances(x, y, w, h, work);
  const candidates: { edge: DockEdge; d: number }[] = [
    { edge: "left", d: d.left },
    { edge: "right", d: d.right },
    { edge: "top", d: d.top },
  ];
  candidates.sort((a, b) => a.d - b.d);
  const best = candidates[0];
  if (!best || best.d > threshold) return null;
  // Prefer side docks when similar distance to top
  if (best.edge === "top") {
    const side = candidates.find((c) => c.edge !== "top");
    if (side && side.d <= threshold + 12) return side.edge;
  }
  return best.edge;
}

export function dockSize(edge: DockEdge): { w: number; h: number } {
  if (edge === "top") return topSize ?? DOCK_TOP;
  // What the rail was actually given last time — see `sizeDockTo`. Without this,
  // every snap/restore would put the width back to the floor and the labels
  // would be clipped again.
  return sideSize ?? DOCK_SIDE;
}

export function dockPosition(
  edge: DockEdge,
  work: Rect,
  anchorY?: number,
  anchorX?: number,
): { x: number; y: number } {
  const size = dockSize(edge);
  if (edge === "left") {
    const y = clamp(
      anchorY ?? work.y + work.h / 2 - size.h / 2,
      work.y + 8,
      work.y + work.h - size.h - 8,
    );
    return { x: work.x, y };
  }
  if (edge === "right") {
    const y = clamp(
      anchorY ?? work.y + work.h / 2 - size.h / 2,
      work.y + 8,
      work.y + work.h - size.h - 8,
    );
    return { x: work.x + work.w - size.w, y };
  }
  // top
  const x = clamp(
    anchorX ?? work.x + work.w / 2 - size.w / 2,
    work.x + 8,
    work.x + work.w - size.w - 8,
  );
  return { x, y: work.y };
}

export function expandGeometry(
  edge: DockEdge | null,
  work: Rect,
  from: { x: number; y: number; w: number; h: number },
): Rect {
  const w = FLOAT_EXPANDED.w;
  const h = FLOAT_EXPANDED.h;
  if (edge === "left") {
    return {
      x: work.x + 6,
      y: clamp(from.y, work.y + 6, work.y + work.h - h - 6),
      w,
      h,
    };
  }
  if (edge === "right") {
    return {
      x: work.x + work.w - w - 6,
      y: clamp(from.y, work.y + 6, work.y + work.h - h - 6),
      w,
      h,
    };
  }
  if (edge === "top") {
    return {
      x: clamp(from.x, work.x + 6, work.x + work.w - w - 6),
      y: work.y + 6,
      w,
      h,
    };
  }
  return {
    x: clamp(from.x, work.x + 6, work.x + work.w - w - 6),
    y: clamp(from.y, work.y + 6, work.y + work.h - h - 6),
    w,
    h,
  };
}

/* ─────────────────────────────────────────────────────────────────────────
 * Motion.
 *
 * The *window* is what moves: the pill has to keep a real hit area, so an
 * oversized always-present transparent window is not an option (it would eat
 * clicks meant for the desktop). Moving a native window is the one thing the
 * webview cannot composite, so the spring is integrated here by hand:
 *
 *  - four independent 1-D springs (x, y, w, h). Apple's rule: never fold a
 *    2-D distance into a single spring — the axes drift apart.
 *  - critically damped (ratio 1.0). A window must never overshoot its bounds.
 *  - velocity survives a re-target, so toggling mid-flight reverses from where
 *    the window *is* instead of snapping, and nothing locks input out.
 *  - size and position are dispatched together each frame, instead of two
 *    awaited IPC round-trips: that serialisation is what made the old
 *    fixed-8-frame ease stutter.
 *  - `prefers-reduced-motion` collapses all of it to an instant jump.
 * ───────────────────────────────────────────────────────────────────────── */

const RESPONSE = 0.38; // seconds — Apple's "move / reposition" is 0.4
const STIFFNESS = (2 * Math.PI / RESPONSE) ** 2;
const DAMPING = 2 * Math.sqrt(STIFFNESS); // damping ratio 1.0
const SUBSTEP = 1 / 240;
const MAX_FRAME = 1 / 30;

let current: Rect | null = null;
let goal: Rect | null = null;
let velocity: Rect = { x: 0, y: 0, w: 0, h: 0 };
let frame = 0;
let stamp = 0;
/** Frames y arranque del vuelo actual, para la traza: cuántos frames costó y
 *  cuánto tardó de verdad una transformación. */
let frameCount = 0;
let flightStart = 0;
/** Último `push` a la ventana (ver `pushedRecently`). */
let lastPushAt = 0;
let morphing = false;
let waiters: (() => void)[] = [];
let morphListener: ((active: boolean) => void) | null = null;

/** True while the window is moving. Replaces the old React `animating` flag,
 *  which locked the toggle out for the whole transition. */
export function isMorphing(): boolean {
  return morphing;
}

/** Lets the shell dim its content during a resize — a big surface shouldn't be
 *  legible while it is being reflowed. */
export function setMorphingListener(cb: ((active: boolean) => void) | null) {
  morphListener = cb;
}

export function prefersReducedMotion(): boolean {
  return (
    typeof window !== "undefined" &&
    typeof window.matchMedia === "function" &&
    window.matchMedia("(prefers-reduced-motion: reduce)").matches
  );
}

function push(rect: Rect) {
  const win = getCurrentWindow();
  // Independent calls: fire them together so a frame costs one round-trip
  // instead of two sequential ones.
  void win.setSize(new LogicalSize(Math.round(rect.w), Math.round(rect.h)));
  void win.setPosition(new LogicalPosition(Math.round(rect.x), Math.round(rect.y)));
  frameCount += 1;
  lastPushAt = performance.now();
}

/** `true` si acabamos de mover la ventana nosotros.
 *
 *  El `WM_MOVE` que provoca un resize programático puede llegar **después** de
 *  que el spring terminó, y entonces el watcher de `onMoved` lo tomaba por un
 *  arrastre del usuario y volvía a pegar el widget al borde: la isla se abría y
 *  saltaba de vuelta al rail. Con esta ventana de gracia, un movimiento nuestro
 *  nunca se confunde con uno suyo. */
export function pushedRecently(ms = 500): boolean {
  return performance.now() - lastPushAt < ms;
}

function settle() {
  frame = 0;
  if (frameCount > 0) {
    traceUi(
      `settle ${fmtRect(current ?? { x: 0, y: 0, w: 0, h: 0 })} en ${frameCount} frames / ${Math.round(
        performance.now() - flightStart,
      )}ms`,
    );
  }
  frameCount = 0;
  if (morphing) {
    morphing = false;
    morphListener?.(false);
  }
  const pending = waiters;
  waiters = [];
  for (const done of pending) done();
}

/** Semi-implicit Euler on a critically damped spring, sub-stepped so a dropped
 *  frame cannot make it explode. */
function advance(pos: number, vel: number, target: number, dt: number): [number, number] {
  let p = pos;
  let v = vel;
  let left = dt;
  while (left > 0) {
    const h = Math.min(SUBSTEP, left);
    const accel = -STIFFNESS * (p - target) - DAMPING * v;
    v += accel * h;
    p += v * h;
    left -= h;
  }
  return [p, v];
}

function tick(now: number) {
  frame = 0;
  const from = current;
  const to = goal;
  if (!from || !to) {
    settle();
    return;
  }

  const dt = Math.min(MAX_FRAME, Math.max(1 / 1000, (now - stamp) / 1000));
  stamp = now;

  const next: Rect = { ...from };
  let arrived = true;
  for (const axis of ["x", "y", "w", "h"] as const) {
    const [p, v] = advance(from[axis], velocity[axis], to[axis], dt);
    next[axis] = p;
    velocity[axis] = v;
    // Position to sub-pixel accuracy; size only until the webview stops
    // re-laying out, otherwise we keep firing one pointless resize.
    const slack = axis === "w" || axis === "h" ? 0.4 : 0.5;
    if (Math.abs(p - to[axis]) > slack || Math.abs(v) > slack * 12) arrived = false;
  }

  current = next;
  push(next);

  if (arrived) {
    current = { ...to };
    velocity = { x: 0, y: 0, w: 0, h: 0 };
    push(current);
    settle();
    return;
  }
  frame = requestAnimationFrame(tick);
}

async function syncFromWindow() {
  const cur = await getWindowLogic();
  current = { x: cur.x, y: cur.y, w: cur.w, h: cur.h };
}

/**
 * Spring the window to `rect`. Interruptible by design: call it again
 * mid-flight and the motion continues from the live position and velocity.
 */
export function springTo(rect: Rect, opts: { instant?: boolean } = {}): Promise<void> {
  goal = { ...rect };
  flightStart = performance.now();
  traceUi(`spring→ ${fmtRect(rect)}${opts.instant ? " instant" : ""}`);

  if (opts.instant || prefersReducedMotion()) {
    if (frame) cancelAnimationFrame(frame);
    frame = 0;
    current = { ...rect };
    velocity = { x: 0, y: 0, w: 0, h: 0 };
    push(current);
    const pending = waiters;
    waiters = [];
    morphing = false;
    for (const done of pending) done();
    return Promise.resolve();
  }

  if (!morphing) {
    morphing = true;
    morphListener?.(true);
  }
  if (!frame) {
    stamp = performance.now();
    frame = requestAnimationFrame(tick);
  }
  return new Promise<void>((resolve) => {
    waiters.push(() => resolve());
  });
}

/**
 * Animate from the real on-screen geometry. Mid-flight we deliberately keep the
 * live value and velocity rather than re-reading — that is exactly what makes
 * an interruption continuous instead of a jump.
 */
export async function animateTo(rect: Rect, opts: { instant?: boolean } = {}): Promise<void> {
  if (!current || !morphing) {
    await syncFromWindow();
    velocity = { x: 0, y: 0, w: 0, h: 0 };
  }
  return springTo(rect, opts);
}

export async function snapToDock(edge: DockEdge, opts: { instant?: boolean } = {}): Promise<void> {
  const cur = await getWindowLogic();
  const size = dockSize(edge);
  const pos = dockPosition(edge, cur.work, cur.y, cur.x);
  const target = { ...pos, ...size };
  // Skip no-op snaps (sub-pixel drift / repeated glue) — they still spring
  // and make the dock look like it's breathing.
  const drift =
    Math.abs(cur.x - target.x) +
    Math.abs(cur.y - target.y) +
    Math.abs(cur.w - target.w) +
    Math.abs(cur.h - target.h);
  if (drift < 2) return;
  await animateTo(target, opts);
}

/** Grow the window to fit BATCH_VISIBLE_ROWS (or JSON paste pane) without clipping the footer. */
export async function sizeForBatchRows(
  _rows: number,
  opts?: { jsonPaste?: boolean },
) {
  const cur = await getWindowLogic();
  const visible = BATCH_VISIBLE_ROWS;
  const contentH = opts?.jsonPaste
    ? METER_JSON_EXTRA
    : visible * BATCH_ROW_H;
  const needH = Math.round((BATCH_CHROME + contentH + 48) * BATCH_SIZE_PAD);
  // El alto es de este panel; el ancho lo sigue llevando la chrome (header +
  // pestañas). Se guarda y se pide **una sola** animación con los dos ejes, en vez
  // de una por acá y otra por el efecto de la chrome pisándose en el mismo frame.
  setPanelHeight(needH);
  traceUi(`batch needH=${needH} chrome=${lastChromeNeed}`);
  if (lastChromeNeed > 0) {
    await sizeForChrome(lastChromeNeed);
    return;
  }
  // Todavía no hay medición de la chrome (panel abierto por primera vez): al
  // menos el alto, sin tocar el ancho actual.
  const k = (window.devicePixelRatio || 1) / (cur.scale || 1);
  const curCssW = document.documentElement.clientWidth || 0;
  const widthCss = Math.max(curCssW, BATCH_BASE.w);
  const w = Math.min(Math.round(widthCss * k), cur.work.w - 24);
  const h = Math.min(Math.max(Math.round(needH * k), Math.round(360 * k)), cur.work.h - 24);
  const x = clamp(cur.x, cur.work.x + 8, cur.work.x + cur.work.w - w - 8);
  const y = clamp(cur.y, cur.work.y + 8, cur.work.y + cur.work.h - h - 8);
  await animateTo({ x, y, w, h });
}
