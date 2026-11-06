/** Parse Meter paste JSON: kvpath + flat fields → path + editable rows. */

export type MeterJsonParsed = {
  namespace: string;
  path: string;
  title: string | null;
  rows: Array<{ key: string; value: string }>;
};

const META = new Set([
  "kvpath",
  "path",
  "namespace",
  "ns",
  "kvns",
  "title",
]);

function coerceValue(v: unknown): string {
  if (v == null) return "";
  if (typeof v === "string") return v;
  if (typeof v === "number" || typeof v === "boolean") return String(v);
  return JSON.stringify(v);
}

export function parseMeterJson(raw: string): MeterJsonParsed {
  const text = raw.trim();
  if (!text) throw new Error("JSON vacío");

  let data: unknown;
  try {
    data = JSON.parse(text);
  } catch {
    throw new Error("JSON inválido");
  }

  if (!data || typeof data !== "object" || Array.isArray(data)) {
    throw new Error("se espera un objeto { kvpath, …campos }");
  }

  const obj = data as Record<string, unknown>;
  const pathRaw = obj.kvpath ?? obj.path;
  const path =
    typeof pathRaw === "string" ? pathRaw.trim().replace(/^\/+|\/+$/g, "") : "";
  if (!path) throw new Error("falta 'kvpath' (path OpenBao)");

  const nsRaw = obj.namespace ?? obj.ns ?? obj.kvns ?? "";
  const namespace =
    typeof nsRaw === "string" && nsRaw.trim() && nsRaw.trim() !== "."
      ? nsRaw.trim()
      : "";

  const titleRaw = obj.title;
  const title =
    typeof titleRaw === "string" && titleRaw.trim() ? titleRaw.trim() : null;

  const rows: Array<{ key: string; value: string }> = [];
  for (const [key, val] of Object.entries(obj)) {
    if (META.has(key)) continue;
    const k = key.trim();
    if (!k) continue;
    rows.push({ key: k, value: coerceValue(val) });
  }

  if (!rows.length) throw new Error("sin campos (solo meta kvpath/ns/title)");

  return { namespace, path, title, rows };
}
