import assert from "node:assert/strict";
import test from "node:test";
import { JamSourceChannelRegistry } from "./jam-channel.ts";

test("only the exact newest authenticated channel capability is current", () => {
  const registry = new JamSourceChannelRegistry();
  const first = registry.open("alice");
  const second = registry.open("alice");
  assert.equal(registry.isCurrent(first), false);
  assert.equal(registry.isCurrent(second), true);
  assert.equal(registry.close("alice"), true);
  assert.equal(registry.isCurrent(second), false);
  assert.notEqual(registry.open("alice"), second);
});
