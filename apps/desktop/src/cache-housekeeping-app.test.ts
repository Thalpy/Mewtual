import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import ts from "typescript";
import { leaveLoss } from "./cache-status.ts";

// Cache housekeeping in App.svelte: the real leave-confirmation handlers, run against stand-ins
// for the state they read, plus static checks that every Leave entry point goes through them and
// that the footer and rename are in place. Function text comes from the production script.
const source = readFileSync(new URL("./App.svelte", import.meta.url), "utf8");
const script = source.slice(source.indexOf(">", source.indexOf("<script")) + 1, source.indexOf("</script>"));
const tree = ts.createSourceFile("App.ts", script, ts.ScriptTarget.ES2022, true, ts.ScriptKind.TS);
const names = new Set(["requestLeave", "confirmLeave", "checkedLeaveReport"]);
const handlers = tree.statements.filter((n) => ts.isFunctionDeclaration(n) && n.name && names.has(n.name.text));
assert.equal(handlers.length, names.size);
const production = ts.transpileModule(handlers.map((n) => n.getFullText(tree)).join("\n"), {
  compilerOptions: { target: ts.ScriptTarget.ES2022 },
}).outputText;

type State = {
  servers: { id: number; name: string }[];
  leaveConfirm: { id: number; name: string; loss: unknown } | null;
  showServerSettings: boolean;
  left: number[];
  calls: string[];
  invoke: (command: string, args: { server: number }) => Promise<unknown>;
  checkedPrivateDownloads: (raw: unknown) => unknown;
  leaveLoss: typeof leaveLoss;
  leaveServer: (id: number) => Promise<void>;
};

function load(state: State) {
  // eslint-disable-next-line no-new-func
  const make = new Function("state", `with (state) { ${production}; return { requestLeave, confirmLeave, checkedLeaveReport }; }`);
  return make(state) as {
    requestLeave: (id: number) => Promise<void>;
    confirmLeave: () => void;
    checkedLeaveReport: (raw: unknown) => unknown;
  };
}

function stateWith(responses: Record<string, unknown>): State {
  const state: State = {
    servers: [{ id: 1, name: "Lantern Room" }],
    leaveConfirm: null,
    showServerSettings: true,
    left: [],
    calls: [],
    invoke: async (command) => {
      state.calls.push(command);
      if (command in responses) return responses[command];
      throw new Error(`unexpected ${command}`);
    },
    checkedPrivateDownloads: (raw) => raw,
    leaveLoss,
    leaveServer: async (id) => {
      state.left.push(id);
    },
  };
  return state;
}

test("leaving asks first and names the Keep copies and unfinished downloads it deletes", async () => {
  const state = stateWith({
    get_kept_files: { files: [{ cid: "aa" }], transfers: [] },
    get_private_downloads: [{ cid: "bb", status: "paused" }],
    get_files: [{ cid: "aa", name: "song.mp3" }, { cid: "bb", name: "clip.mp4" }],
  });
  const app = load(state);
  const pending = app.requestLeave(1);
  assert.deepEqual(state.leaveConfirm, { id: 1, name: "Lantern Room", loss: null }, "the dialog opens at once, still loading");
  await pending;
  assert.deepEqual(state.leaveConfirm?.loss, { keep: ["song.mp3"], downloads: ["clip.mp4"], unknown: false });
  assert.deepEqual(state.left, [], "nothing leaves until confirmed");
  app.confirmLeave();
  assert.deepEqual(state.left, [1]);
  assert.equal(state.leaveConfirm, null);
  assert.equal(state.showServerSettings, false);
});

test("a dismissed confirmation is never resurrected by a late load", async () => {
  const state = stateWith({ get_kept_files: null, get_private_downloads: [], get_files: [] });
  const app = load(state);
  const pending = app.requestLeave(1);
  state.leaveConfirm = null; // the user pressed Stay
  await pending;
  assert.equal(state.leaveConfirm, null);
  app.confirmLeave();
  assert.deepEqual(state.left, []);
});

test("unreadable lists still ask, and say they could not check", async () => {
  const state = stateWith({});
  const app = load(state);
  await app.requestLeave(1);
  assert.deepEqual(state.leaveConfirm?.loss, { keep: [], downloads: [], unknown: true });
  // A Keep store that reports an error is just as unknown, even with empty lists.
  const faulted = stateWith({
    get_kept_files: { files: [], transfers: [], error: "Kept storage needs attention" },
    get_private_downloads: [],
    get_files: [],
  });
  await load(faulted).requestLeave(1);
  assert.equal((faulted.leaveConfirm?.loss as { unknown: boolean }).unknown, true);
  // Named as the headings name them, and "may": another entry for the same group keeps them.
  assert.match(source, /leaving\.loss\.unknown[\s\S]{0,200}Keep copies or unfinished downloads could not be checked\. Leaving may delete any/);
});

test("the leave report is validated before the toast claims anything", () => {
  const app = load(stateWith({}));
  assert.deepEqual(
    app.checkedLeaveReport({ freed_bytes: 5, kept_bytes: 0, failed: 0, cleaned: true }),
    { freed_bytes: 5, kept_bytes: 0, failed: 0, cleaned: true },
  );
  assert.equal(app.checkedLeaveReport({ freed_bytes: 5, kept_bytes: 0, failed: 0 }), null, "cleaned is required");
  assert.equal(app.checkedLeaveReport({ freed_bytes: -1, kept_bytes: 0, failed: 0, cleaned: true }), null);
  assert.equal(app.checkedLeaveReport(null), null);
});

test("every Leave entry point goes through the confirmation", () => {
  const direct = [...source.matchAll(/leaveServer\(/g)].length;
  // The definition and the one call from confirmLeave; nothing else may leave without asking.
  assert.equal(direct, 2, "leaveServer is reachable only from confirmLeave");
  assert.match(source, /label: "Leave server",[\s\S]{0,400}onSelect: \(\) => void requestLeave\(s\.id\)/);
  assert.match(source, /class="ghost leave" onclick=\{\(\) => \{ const id = activeServerId; if \(id !== null\) void requestLeave\(id\); \}\}/);
});

test("the footer segment opens the Storage page and the old label is gone", () => {
  assert.match(source, /class="seg sb-cache"[\s\S]{0,200}openServerSettings\(activeServerId, "storage"\)/);
  assert.match(source, /\{ id: "storage", label: "Storage", cat: "Content" \}/);
  assert.doesNotMatch(source, /"♾ Keep forever"/);
  assert.match(source, /"♾ Never expires"/);
});
