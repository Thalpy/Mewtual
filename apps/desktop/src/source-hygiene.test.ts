import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { extname, join, relative } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const sourceDir = fileURLToPath(new URL(".", import.meta.url));

function sourceFiles(dir: string): string[] {
  const files: string[] = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) files.push(...sourceFiles(path));
    else if ([".ts", ".svelte", ".js", ".css"].includes(extname(entry.name))) files.push(path);
  }
  return files;
}

/** Tab, newline and carriage return are the only control characters source legitimately holds. */
const ALLOWED_CONTROLS = new Set([9, 10, 13]);

/**
 * Deliberately expressed as arithmetic rather than as a regular expression character class.
 * Writing this rule needs the very characters the rule forbids, and a class built from them is a
 * guard that trips over its own subject.
 */
function isRawControl(code: number): boolean {
  return (code < 32 && !ALLOWED_CONTROLS.has(code)) || code === 127;
}

/**
 * No source file may contain a raw control character.
 *
 * Written for a real one. A domain separator in the redaction hash was a literal NUL byte in the
 * source rather than the two-character escape, which produced the right string at runtime and an
 * unreadable file everywhere else: grep, and every tool sharing its heuristic, classified the
 * whole file as binary and silently skipped it. A search that quietly returns nothing is worse
 * than one that fails, because the answer looks like "not present".
 *
 * The separator is also load-bearing. Anything that strips an invisible byte on the way through
 * (an editor normalising on save, a patch tool, a copy through a terminal) would join the salt,
 * the kind and the value into one string with no boundary between them, and two different kinds
 * of identifier could then hash to the same alias. As an escape it is ordinary ASCII that no tool
 * has an opinion about.
 */
test("no source file carries a raw control character where an escape belongs", () => {
  const offenders: string[] = [];
  for (const path of sourceFiles(sourceDir)) {
    const text = readFileSync(path, "utf8");
    let line = 1;
    for (const ch of text) {
      const code = ch.codePointAt(0) ?? 0;
      if (code === 10) line += 1;
      else if (isRawControl(code)) {
        const hex = code.toString(16).padStart(4, "0").toUpperCase();
        offenders.push(`${relative(sourceDir, path)}:${line} contains U+${hex}`);
      }
    }
  }
  assert.deepEqual(offenders, [], `write these as escapes instead:\n${offenders.join("\n")}`);
});
