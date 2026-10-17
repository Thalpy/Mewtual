import process from "node:process";

/**
 * Build the startup probe's child environment without changing application logging defaults.
 *
 * The completion marker is emitted by `catcoms_app` at Info. An inherited filter can suppress it,
 * so the last directive restores just that target. If the filter is absent or blank, however,
 * passing a target-only filter would disable `catcoms-log`'s normal defaults for every other
 * target and hide useful startup diagnostics. Treat that case as genuinely unset instead.
 */
export function startupChildEnv(inheritedEnv, platform = process.platform) {
  const childEnv = { ...inheritedEnv };
  // Windows environment names are case-insensitive, but spreading `process.env` produces an
  // ordinary case-sensitive object. Find and remove the spelling Windows actually inherited so a
  // blank `RuSt_LoG` cannot survive beside the canonical key and suppress the application default.
  // POSIX names are case-sensitive, so an unrelated lowercase key must remain untouched there.
  const rustLogKeys = platform === "win32"
    ? Object.keys(childEnv).filter((key) => key.toUpperCase() === "RUST_LOG")
    : Object.hasOwn(childEnv, "RUST_LOG") ? ["RUST_LOG"] : [];
  const inheritedRustLog = rustLogKeys.length > 0
    ? String(childEnv[rustLogKeys[0]] ?? "").trim()
    : undefined;

  for (const key of rustLogKeys) delete childEnv[key];

  if (inheritedRustLog) {
    childEnv.RUST_LOG = `${inheritedRustLog},catcoms_app=info`;
  }

  return childEnv;
}
