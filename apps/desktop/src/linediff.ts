// A small line-level diff for the wiki's history and review surfaces: old body vs new body
// as kept/removed/added lines, the familiar unified-diff reading. LCS via the standard
// O(N*D) Myers walk on lines; wiki pages are small documents, so no windowing is needed,
// but a hard cap keeps a pathological page from freezing the UI thread.

/// One diff row. `same` lines appear in both bodies; `del` only in the old; `add` only in
/// the new.
export type DiffLine = { kind: "same" | "del" | "add"; text: string };

/// Beyond this many lines per side, fall back to whole-body replace (never freeze the UI).
const MAX_DIFF_LINES = 4000;

/// Diff two page bodies line-by-line, old to new.
export function diffLines(oldBody: string, newBody: string): DiffLine[] {
  if (oldBody === newBody) {
    return oldBody === "" ? [] : oldBody.split("\n").map((text) => ({ kind: "same" as const, text }));
  }
  const a = oldBody === "" ? [] : oldBody.split("\n");
  const b = newBody === "" ? [] : newBody.split("\n");
  if (a.length > MAX_DIFF_LINES || b.length > MAX_DIFF_LINES) {
    return [
      ...a.map((text) => ({ kind: "del" as const, text })),
      ...b.map((text) => ({ kind: "add" as const, text })),
    ];
  }

  // Myers: furthest-reaching x per diagonal k, with a trace for backtracking.
  const max = a.length + b.length;
  const offset = max;
  let v = new Int32Array(2 * max + 1);
  const trace: Int32Array[] = [];
  let dFound = -1;
  outer: for (let d = 0; d <= max; d++) {
    trace.push(v.slice());
    const next = v.slice();
    for (let k = -d; k <= d; k += 2) {
      let x: number;
      if (k === -d || (k !== d && v[offset + k - 1] < v[offset + k + 1])) {
        x = v[offset + k + 1]; // down: an added line
      } else {
        x = v[offset + k - 1] + 1; // right: a removed line
      }
      let y = x - k;
      while (x < a.length && y < b.length && a[x] === b[y]) {
        x++;
        y++;
      }
      next[offset + k] = x;
      if (x >= a.length && y >= b.length) {
        dFound = d;
        v = next;
        break outer;
      }
    }
    v = next;
  }

  // Backtrack from the end to recover the edit script, then reverse it.
  const rev: DiffLine[] = [];
  let x = a.length;
  let y = b.length;
  for (let d = dFound; d > 0; d--) {
    const vPrev = trace[d];
    const k = x - y;
    let prevK: number;
    if (k === -d || (k !== d && vPrev[offset + k - 1] < vPrev[offset + k + 1])) {
      prevK = k + 1; // came from a down-move (add)
    } else {
      prevK = k - 1; // came from a right-move (del)
    }
    const prevX = vPrev[offset + prevK];
    const prevY = prevX - prevK;
    while (x > prevX && y > prevY) {
      rev.push({ kind: "same", text: a[x - 1] });
      x--;
      y--;
    }
    if (prevK === k + 1) {
      rev.push({ kind: "add", text: b[y - 1] });
      y--;
    } else {
      rev.push({ kind: "del", text: a[x - 1] });
      x--;
    }
  }
  while (x > 0 && y > 0) {
    rev.push({ kind: "same", text: a[x - 1] });
    x--;
    y--;
  }
  return rev.reverse();
}

/// Counts for a compact "+N -M" summary chip.
export function diffStats(lines: DiffLine[]): { added: number; removed: number } {
  let added = 0;
  let removed = 0;
  for (const l of lines) {
    if (l.kind === "add") added++;
    else if (l.kind === "del") removed++;
  }
  return { added, removed };
}
