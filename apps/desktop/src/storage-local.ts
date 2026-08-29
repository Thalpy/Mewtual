export type StorageRepairResult = {
  attempted_chunks: number;
  recovered_chunks: number;
  health: {
    missing_chunks: number;
    unreadable_chunks: number;
    invalid_manifests: number;
  };
};

/** Human-readable result for an explicit local-storage repair pass.
 *
 * Zero attempts does not necessarily mean healthy: contradictory exact references deliberately
 * fail closed because fetching the same content-addressed ciphertext cannot reconcile their keys.
 */
export function storageRepairNotice(result: StorageRepairResult): string {
  if (result.attempted_chunks) {
    return `Checked ${result.attempted_chunks} damaged or missing chunks; recovered ${result.recovered_chunks}.`;
  }
  const remaining = result.health.missing_chunks
    + result.health.unreadable_chunks
    + result.health.invalid_manifests;
  if (remaining) {
    return `Nothing could be repaired automatically; ${remaining} unreadable, missing, or invalid storage reference${remaining === 1 ? " remains" : "s remain"}.`;
  }
  return "Everything referenced by this server verifies.";
}
