"""Isolated executed-assertion checks for the local Studio handoff transaction.

Run only after normal regressions pass. Restore exact source bytes after every mutation;
compilation failures, zero-test filters and incidental panics never count as detection.
Optional command-line names select a subset for diagnosis without changing the CI default.
"""
from pathlib import Path
import os
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
PREFIX = "store::epoch_studio::tests::rotation::overlay::handoff::"
CORE = "crates/catcoms-replication/src/studio/overlay/handoff.rs"
STORE = "crates/catcoms-app/src/store/epoch_studio/handoff.rs"
COMMAND = ["cargo", "test", "--locked", "-j", "4", "--config",
           "profile.test.package.catcoms-app.debug=0", "-p", "catcoms-app", "--lib"]
MUTATIONS = [
    ("completed-target", CORE,
     "self.check_target(target)?;\n        let Some(done) = &self.completed else {",
     "let _ = target;\n        let Some(done) = &self.completed else {",
     "studio_overlay_handoff_completed_retry_keeps_channel_after_real_receipt_retirement",
     "completed retry acknowledged a different channel"),
    ("signed-digest", CORE,
     "Ok(Some(actual)) if &actual == expected => count += 1,",
     "Ok(Some(actual)) => { let _ = (actual, expected); count += 1; },",
     "evidence::studio_overlay_handoff_full_signed_digest_prevents_false_completion",
     "handoff accepted a different complete signed operation"),
    ("source-version", STORE,
     ".is_none_or(|a| blake3::hash(&a.plain) != capability.before)",
     ".is_none_or(|a| { let _ = (&a.plain, capability.before); false })",
     "evidence::studio_overlay_handoff_rechecks_source_after_prepared_before_candidate_write",
     "handoff overwrote source changed after Prepared"),
    ("replacement-fence", STORE,
     ".preserves_vault_source(snapshot, &protected)",
     ".preserves_vault_source(snapshot, &protected).map(|_| true)",
     "fences::studio_overlay_handoff_publication_and_shared_replacement_fences_survive_restart",
     "Prepared source was replaced"),
    ("publication-fence", STORE,
     'if metadata.is_prepared() {\n                return Err(invalid("prepared overlay handoff is not publishable"));',
     'if metadata.is_prepared() && false {\n                return Err(invalid("prepared overlay handoff is not publishable"));',
     "fences::studio_overlay_handoff_publication_and_shared_replacement_fences_survive_restart",
     "called `Result::unwrap_err()` on an `Ok` value: Page"),
    ("required-intents", STORE, "if linked {", "if linked && false {",
     "fences::studio_overlay_handoff_missing_metadata_cannot_become_publishable_after_restart",
     "missing Prepared record exposed its source"),
    ("retry-floor", CORE, "if closed < floor {", "if closed < floor && false {",
     "metadata::studio_overlay_handoff_rollover_floor_rejects_forgotten_retry_after_rewind",
     "forgotten overlay retry crossed persisted floor"),
    ("acceptance-order", "crates/catcoms-replication/src/studio/epoch/handoff.rs",
     "for (intent, ts) in overlay.ordered(ledger)? {",
     "for (intent, ts) in overlay.ordered(ledger)?.into_iter().rev() {",
     "eligibility::studio_overlay_handoff_replays_dependency_order_and_retains_all_pixel_references",
     "handoff lost accepted operation order"),
    ("later-peak", "crates/catcoms-app/src/store/epoch_budget.rs",
     "for step in steps {", "for step in steps.iter().take(0) {",
     "metadata::studio_overlay_handoff_preflights_later_source_peak_before_prepared_write",
     "handoff wrote Prepared before checking the later source peak"),
    ("reference-dependency", "crates/catcoms-app/src/store/epoch_recovery/inventory.rs",
     "collected.check_dependencies()?;", "let _ = collected.check_dependencies();",
     "references::studio_overlay_handoff_reference_scan_keeps_overlay_only_pixels_when_metadata_is_missing",
     "missing handoff metadata completed a reference scan"),
]


def run(test):
    env = os.environ.copy()
    env["CARGO_INCREMENTAL"] = "0"
    if os.name == "nt":
        env["_LINK_"] = "/DEBUG:NONE"
    return subprocess.run(COMMAND + [PREFIX + test, "--", "--exact", "--nocapture"],
                          cwd=ROOT, env=env, text=True, encoding="utf-8", errors="replace",
                          stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=900, check=False)


def main():
    selected = set(sys.argv[1:])
    if selected - {m[0] for m in MUTATIONS}:
        raise ValueError("unknown mutation selector")
    mutations = [m for m in MUTATIONS if not selected or m[0] in selected]
    logs = ROOT / "logs"
    logs.mkdir(exist_ok=True)
    for name, path, before, after, test, assertion in mutations:
        source = ROOT / path
        original = source.read_bytes()
        if original.count(before.encode()) != 1:
            raise AssertionError(f"mutation anchor is not unique: {name}")
        changed = original.replace(before.encode(), after.encode(), 1)
        try:
            source.write_bytes(changed)
            result = run(test)
            (logs / f"gate4-handoff-mutation-{name}.log").write_text(result.stdout, encoding="utf-8")
            if not (result.returncode != 0 and assertion in result.stdout
                    and "test result: FAILED. 0 passed; 1 failed;" in result.stdout):
                print(result.stdout, flush=True)
                raise AssertionError(f"mutation did not fail at its intended assertion: {name}")
            print(f"DETECTED {name}: {assertion}", flush=True)
        finally:
            if source.read_bytes() != changed:
                raise AssertionError(f"source changed concurrently: {path}")
            source.write_bytes(original)
            assert source.read_bytes() == original
            print(f"RESTORED {path} byte-for-byte", flush=True)
    for name, _, _, _, test, _ in mutations:
        result = run(test)
        (logs / f"gate4-handoff-restored-{name}.log").write_text(result.stdout, encoding="utf-8")
        if result.returncode != 0 or "test result: ok. 1 passed; 0 failed;" not in result.stdout:
            print(result.stdout, flush=True)
            raise AssertionError(f"restored regression failed: {name}")
        print(f"PASS restored {name}", flush=True)


if __name__ == "__main__":
    main()
