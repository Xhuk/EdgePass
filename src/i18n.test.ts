import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { TRANSLATIONS, setLocale, t } from "./i18n";

/**
 * Every `t("literal")` in the UI must have an English entry.
 *
 * `t()` falls back to the Spanish key, so a missing translation is invisible in
 * a Spanish install and shows Spanish in an English one. This is the guard that
 * makes the omission a red test instead of a surprise on a stranger's screen.
 */
function sourceFiles(dir: string): string[] {
  const out: string[] = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) out.push(...sourceFiles(path));
    else if (/\.(ts|tsx)$/.test(entry.name) && !entry.name.endsWith(".test.ts")) out.push(path);
  }
  return out;
}

function usedKeys(): Set<string> {
  const keys = new Set<string>();
  for (const file of sourceFiles(join(process.cwd(), "src"))) {
    const source = readFileSync(file, "utf8");
    for (const match of source.matchAll(/\bt\(\s*"((?:[^"\\]|\\.)*)"\s*,?\s*\)/g)) {
      keys.add(match[1].replace(/\\"/g, '"').replace(/\\\\/g, "\\"));
    }
  }
  return keys;
}

describe("i18n coverage", () => {
  it("translates every literal passed to t()", () => {
    const missing = [...usedKeys()].filter((key) => !(key in TRANSLATIONS)).sort();
    expect(missing, `missing EN entries:\n${missing.join("\n")}`).toEqual([]);
  });

  it("translates interpolated status lines segment by segment", () => {
    setLocale("es-ES");
    expect(t("1 of 1 running · tunnel edgepass-mcp up")).toBe(
      "1 de 1 corriendo · túnel edgepass-mcp arriba",
    );
    expect(t("Sealed (1/5)")).toBe("Sellado (1/5)");
    expect(t("binary missing: C:\\tools\\x.exe")).toBe("falta el binario: C:\\tools\\x.exe");
    setLocale("en-US");
    expect(t("1 of 1 running")).toBe("1 of 1 running");
  });
});
