/** Google Password Manager / Chrome CSV → OpenBao login rows. */

export type LoginDraft = {
  slug: string;
  name: string;
  url: string;
  username: string;
  password: string;
  notes: string;
};

const HEADER_ALIASES: Record<string, keyof Omit<LoginDraft, "slug">> = {
  name: "name",
  title: "name",
  url: "url",
  website: "url",
  origin: "url",
  username: "username",
  user: "username",
  login: "username",
  email: "username",
  password: "password",
  pass: "password",
  note: "notes",
  notes: "notes",
  comment: "notes",
};

/** Minimal CSV (RFC4180-ish) for Google exports. */
export function parseCsv(text: string): string[][] {
  const rows: string[][] = [];
  let row: string[] = [];
  let cell = "";
  let i = 0;
  let inQuotes = false;
  const src = text.replace(/^\uFEFF/, "");

  while (i < src.length) {
    const ch = src[i]!;
    if (inQuotes) {
      if (ch === '"') {
        if (src[i + 1] === '"') {
          cell += '"';
          i += 2;
          continue;
        }
        inQuotes = false;
        i += 1;
        continue;
      }
      cell += ch;
      i += 1;
      continue;
    }
    if (ch === '"') {
      inQuotes = true;
      i += 1;
      continue;
    }
    if (ch === ",") {
      row.push(cell);
      cell = "";
      i += 1;
      continue;
    }
    if (ch === "\n" || ch === "\r") {
      if (ch === "\r" && src[i + 1] === "\n") i += 1;
      row.push(cell);
      cell = "";
      if (row.some((c) => c.trim().length > 0)) rows.push(row);
      row = [];
      i += 1;
      continue;
    }
    cell += ch;
    i += 1;
  }
  row.push(cell);
  if (row.some((c) => c.trim().length > 0)) rows.push(row);
  return rows;
}

export function slugifyLogin(name: string, url: string): string {
  let base = name.trim() || hostnameOf(url) || "login";
  base = base
    .normalize("NFKD")
    .replace(/[\u0300-\u036f]/g, "")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 48);
  return base || "login";
}

function hostnameOf(url: string): string {
  try {
    return new URL(url).hostname.replace(/^www\./, "");
  } catch {
    return "";
  }
}

/**
 * Parse a Google/Chrome password CSV into login drafts under
 * `personal/logins/{slug}` (slug assigned uniquely).
 */
export function parseGooglePasswordCsv(text: string): LoginDraft[] {
  const table = parseCsv(text);
  if (table.length < 2) return [];

  const header = table[0]!.map((h) => h.trim().toLowerCase());
  const idx: Partial<Record<keyof Omit<LoginDraft, "slug">, number>> = {};
  header.forEach((h, i) => {
    const key = HEADER_ALIASES[h];
    if (key && idx[key] === undefined) idx[key] = i;
  });

  if (idx.password === undefined && idx.username === undefined && idx.url === undefined) {
    throw new Error(
      "CSV no parece de Google Password Manager (faltan columnas url/username/password)",
    );
  }

  const used = new Map<string, number>();
  const out: LoginDraft[] = [];

  for (const cells of table.slice(1)) {
    const get = (k: keyof Omit<LoginDraft, "slug">) => {
      const i = idx[k];
      return i === undefined ? "" : (cells[i] ?? "").trim();
    };
    const password = get("password");
    const username = get("username");
    const url = get("url");
    const name = get("name") || hostnameOf(url) || username || "login";
    const notes = get("notes");
    if (!password && !username) continue;

    let slug = slugifyLogin(name, url);
    const n = (used.get(slug) ?? 0) + 1;
    used.set(slug, n);
    if (n > 1) slug = `${slug}-${n}`;

    out.push({ slug, name, url, username, password, notes });
  }
  return out;
}

export function loginPath(slug: string): string {
  return `personal/logins/${slug}`;
}
