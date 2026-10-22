import assert from "node:assert/strict";
import test from "node:test";
import { EventEmitter } from "node:events";
import { spawn } from "node:child_process";
import { childOutputClosed } from "./child-output.mjs";

test("an exit event cannot retire output before its close event", async () => {
  const child = new EventEmitter();
  let settled = false;
  const closed = childOutputClosed(child).then(value => { settled = true; return value; });
  child.emit("exit", 0); await Promise.resolve();
  assert.equal(settled, false);
  child.emit("close", 0, null);
  assert.deepEqual(await closed, { code: 0, signal: null });
});

test("a marker written immediately before a real child exits is captured at close", async () => {
  const child = spawn(process.execPath, ["-e", "process.stdout.write('PROBE.SHUTDOWN.DRAINED')"], { windowsHide: true, stdio: ["ignore", "pipe", "pipe"] });
  let output = "";
  child.stdout.on("data", bytes => { output += bytes; });
  const closed = childOutputClosed(child);
  const result = await closed;
  assert.equal(result.code, 0); assert.equal(child.stdout.readableEnded, true);
  assert.equal(output, "PROBE.SHUTDOWN.DRAINED");
});
