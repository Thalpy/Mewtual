import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import ts from "typescript";

const source = readFileSync(new URL("./App.svelte", import.meta.url), "utf8");
const start = source.indexOf("  function restoreReloaded(");
const end = source.indexOf("  async function unlock(", start);
assert.ok(start >= 0 && end > start);
const restore = ts.transpileModule(source.slice(start, end), {
  compilerOptions: { target: ts.ScriptTarget.ES2022 },
}).outputText;

function reopen(reloaded: object[]) {
  return new Function("reloaded", `
    let servers = [], locked = true, uiStateLoadGeneration = 0;
    const warnings = [];
    const sessionStorage = { removeItem() {} };
    const toast = (...args) => warnings.push(args);
    const loadUiContinuity = () => new Promise(() => {});
    const refreshAllDmRequests = () => {}, loadInbox = () => {}, refreshAllServerIcons = () => {};
    ${restore}
    restoreReloaded(reloaded);
    return { warnings, locked, servers };
  `)(reloaded) as { warnings: unknown[][]; locked: boolean; servers: { id: number }[] };
}

test("actual restored-server flow discloses missing outgoing routes without blocking the group", () => {
  const warning = "This group has no saved outgoing route. Reconnecting may depend on another member connecting to you or on discovery.";
  const result = reopen([
    { server: 1, name: "Chat", channel: "1", reconnect_warning: warning },
    { server: 2, name: "Other", channel: "1" },
  ]);
  assert.equal(result.locked, false);
  assert.deepEqual(result.servers.map((server) => server.id), [1, 2]);
  assert.deepEqual(result.warnings, [[`Chat: ${warning}`, "warn", 0]]);
});

test("legacy projections and empty founder vaults do not invent reconnect warnings", () => {
  assert.deepEqual(reopen([]).warnings, []);
  assert.deepEqual(reopen([{ server: 1, name: "Chat", channel: "1" }]).warnings, []);
});
