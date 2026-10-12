import assert from "node:assert/strict";
import test from "node:test";

import { startupChildEnv } from "./startup-env.mjs";

test("keeps RUST_LOG unset so the application installs its normal defaults", () => {
  const childEnv = startupChildEnv({ PATH: "example" });

  assert.equal("RUST_LOG" in childEnv, false);
  assert.equal(childEnv.PATH, "example");
});

test("treats a blank inherited RUST_LOG as unset", () => {
  assert.equal("RUST_LOG" in startupChildEnv({ RUST_LOG: "  " }), false);
});

test("appends a final target directive that makes the setup marker observable", () => {
  assert.equal(startupChildEnv({ RUST_LOG: "warn" }).RUST_LOG, "warn,catcoms_app=info");
  assert.equal(
    startupChildEnv({ RUST_LOG: "info,catcoms_app=off" }).RUST_LOG,
    "info,catcoms_app=off,catcoms_app=info",
  );
});

test("Windows removes a blank mixed-case RUST_LOG key", () => {
  const childEnv = startupChildEnv({ PATH: "example", RuSt_LoG: "  " }, "win32");

  assert.deepEqual(
    Object.keys(childEnv).filter((key) => key.toUpperCase() === "RUST_LOG"),
    [],
  );
});

test("Windows emits one canonical key for a mixed-case inherited filter", () => {
  const childEnv = startupChildEnv({ RuSt_LoG: "warn" }, "win32");

  assert.deepEqual(
    Object.keys(childEnv).filter((key) => key.toUpperCase() === "RUST_LOG"),
    ["RUST_LOG"],
  );
  assert.equal(childEnv.RUST_LOG, "warn,catcoms_app=info");
});

test("POSIX preserves differently cased environment names", () => {
  const childEnv = startupChildEnv({ rust_log: "trace" }, "linux");

  assert.equal(childEnv.rust_log, "trace");
  assert.equal("RUST_LOG" in childEnv, false);
});
