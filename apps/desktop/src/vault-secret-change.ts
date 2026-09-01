export type VaultSecretChangeResult = {
  changed: boolean;
  durabilityConfirmed: boolean;
  warning: string | null;
};

export type VaultSecretChangeNotice = {
  kind: "confirmed" | "committed-uncertain";
  message: string;
};

export const MAX_NEW_VAULT_SECRET_BYTES = 4096;

/** Validate the exact UTF-8 bytes sent to the native KDF boundary. */
export function newVaultSecretError(secret: string): string | null {
  if (!secret) return "Enter a vault secret.";
  // JavaScript string length counts UTF-16 code units, so it misreports emoji and other Unicode.
  const bytes = new TextEncoder().encode(secret).byteLength;
  return bytes > MAX_NEW_VAULT_SECRET_BYTES
    ? `Vault secrets can use at most ${MAX_NEW_VAULT_SECRET_BYTES} UTF-8 bytes (this entry uses ${bytes}).`
    : null;
}

/**
 * Turn the native commit classification into user-facing truth. A post-rename directory-sync
 * failure is not a rollback: the replacement wrapper is visible and the new secret is active.
 */
export function vaultSecretChangeNotice(result: VaultSecretChangeResult): VaultSecretChangeNotice {
  if (!result.changed) throw new Error("the vault did not report a committed secret change");
  if (result.durabilityConfirmed) {
    return {
      kind: "confirmed",
      message: "Vault secret changed. Existing backups still use their old secret.",
    };
  }
  return {
    kind: "committed-uncertain",
    message: result.warning ||
      "The new vault secret is active, but crash durability could not be confirmed. Keep the new secret; the old one must not be treated as current.",
  };
}
