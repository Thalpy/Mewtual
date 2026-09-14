"""Require Closing overlay regressions to catch isolated guard removals.

Run from a Rust test environment; no native/UI build is needed. Logs stay local under logs/.
Restoration is byte-exact and refuses to overwrite an unrelated concurrent source change.
"""
from pathlib import Path
import os
import subprocess


ROOT = Path(__file__).resolve().parents[2]
PREFIX = "store::epoch_studio::tests::rotation::overlay::studio_overlay_store_"
COMMAND = [
    "cargo", "test", "--locked", "-j", "4", "--config",
    "profile.test.package.catcoms-app.debug=0", "-p", "catcoms-app", "--lib",
]
MUTATIONS = [
    (
        "source-version-first", "crates/catcoms-replication/src/studio/overlay.rs",
        "source_version,\n            receipt: plan.receipt().clone(),",
        "source_version: source_version.map(|_| 0),\n            receipt: plan.receipt().clone(),",
        "changed_closing_source_refuses_first_acceptance",
        "overlay basis ignored changed persisted Closing source version",
    ),
    (
        "source-version-append", "crates/catcoms-replication/src/studio/overlay.rs",
        "source_version,\n            receipt: plan.receipt().clone(),",
        "source_version: source_version.map(|_| 0),\n            receipt: plan.receipt().clone(),",
        "changed_closing_source_refuses_append_but_keeps_exact_retry",
        "overlay basis ignored changed persisted Closing source version",
    ),
    (
        "ordinary-apply", "crates/catcoms-app/src/store/epoch_intents.rs",
        "if state.is_overlay(&operation.id(&device.device_id())) {", "if false {",
        "failed_ordinary_intent_is_not_acceptance_and_ordinary_apply_cannot_promote",
        "ordinary Apply admitted an annotated local draft",
    ),
    (
        "sequence", "crates/catcoms-replication/src/studio/overlay.rs",
        "|| entry.sequence != index as u64 + 1", "|| (index == usize::MAX)",
        "codec_binds_annotations_envelopes_and_sequence", "overlay accepted reordered sequence",
    ),
    (
        "seed-refs", "crates/catcoms-app/src/store/epoch_recovery/inventory.rs",
        "overlay.base_blob_cids().map_err(invalid)?", "{ let _ = overlay; Vec::<[u8; 32]>::new() }",
        "seed_only_references_survive_without_the_source", "overlay seed-only pixel reference was lost",
    ),
]


def run(test):
    env = os.environ.copy()
    env["CARGO_INCREMENTAL"] = "0"
    if os.name == "nt":
        env["_LINK_"] = "/DEBUG:NONE"
    return subprocess.run(
        COMMAND + [PREFIX + test, "--", "--exact", "--nocapture"], cwd=ROOT,
        env=env, text=True, encoding="utf-8", errors="replace",
        stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=900, check=False,
    )


def main():
    log_dir = ROOT / "logs"
    log_dir.mkdir(exist_ok=True)
    for name, path, before, after, test, assertion in MUTATIONS:
        source = ROOT / path
        original = source.read_bytes()
        before, after = before.encode(), after.encode()
        if original.count(before) != 1:
            raise AssertionError(f"mutation anchor is not unique: {name}")
        changed = original.replace(before, after, 1)
        try:
            source.write_bytes(changed)
            result = run(test)
            (log_dir / f"gate4-overlay-mutation-{name}.log").write_text(result.stdout, encoding="utf-8")
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
    # Recompile the restored sources once, then require each exercised regression to pass.
    for name, _, _, _, test, _ in MUTATIONS:
        result = run(test)
        (log_dir / f"gate4-overlay-restored-{name}.log").write_text(result.stdout, encoding="utf-8")
        if result.returncode != 0 or "test result: ok. 1 passed; 0 failed;" not in result.stdout:
            print(result.stdout, flush=True)
            raise AssertionError(f"restored regression failed: {name}")
        print(f"PASS restored {name}", flush=True)


if __name__ == "__main__":
    main()
