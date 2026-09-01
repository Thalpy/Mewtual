import assert from "node:assert/strict";
import test from "node:test";
import {
  MAX_NEW_VAULT_SECRET_BYTES,
  newVaultSecretError,
  vaultSecretChangeNotice,
} from "./vault-secret-change.ts";

test("new vault secrets are bounded by encoded UTF-8 bytes", () => {
  assert.equal(newVaultSecretError("a".repeat(MAX_NEW_VAULT_SECRET_BYTES)), null);
  assert.match(
    newVaultSecretError("a".repeat(MAX_NEW_VAULT_SECRET_BYTES + 1)) ?? "",
    /4096 UTF-8 bytes.*4097/i,
  );
  assert.equal(newVaultSecretError("🐈".repeat(1024)), null);
  assert.match(newVaultSecretError("🐈".repeat(1025)) ?? "", /4100/i);
});

test("a committed-but-not-durable rewrap identifies the new secret as active", () => {
  const notice = vaultSecretChangeNotice({
    changed: true,
    durabilityConfirmed: false,
    warning: "The new secret is active, but crash durability is uncertain. Keep the new secret; do not treat the old secret as current.",
  });
  assert.equal(notice.kind, "committed-uncertain");
  assert.match(notice.message, /new (?:vault )?secret is active/i);
  assert.match(notice.message, /keep the new secret/i);
  assert.doesNotMatch(notice.message, /was not changed/i);
});

test("a durable rewrap uses the ordinary success copy", () => {
  const notice = vaultSecretChangeNotice({
    changed: true,
    durabilityConfirmed: true,
    warning: null,
  });
  assert.equal(notice.kind, "confirmed");
  assert.match(notice.message, /secret changed/i);
});
