import { test } from "node:test";
import assert from "node:assert/strict";
import { memberRole, identityForRoleLabel } from "./member-role.ts";

test("role authority distinguishes devices with the same display label", () => {
  const a = "12345678" + "a".repeat(56);
  const twin = "12345678" + "b".repeat(56);
  const roles = { [a]: "admin", "12345678": "owner" };
  assert.equal(memberRole(roles, a), "admin");
  assert.equal(memberRole(roles, twin), "member");
  const members = [{ fingerprint: "12345678", identity: a }, { fingerprint: "12345678", identity: twin }];
  assert.equal(identityForRoleLabel(members, "12345678"), "");
  assert.equal(identityForRoleLabel(members.slice(0, 1), "12345678"), a);
});
