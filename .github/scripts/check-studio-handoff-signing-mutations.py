"""Execute isolated handoff authority mutants and byte-exact restored regressions."""
from pathlib import Path
import os
import subprocess

ROOT = Path(__file__).resolve().parents[2]
PREFIX = "studio::epoch::owner::tests::handoff::"
MUTATIONS = [
    ("mls", "crates/catcoms-replication/src/studio/overlay/handoff/preparation.rs",
     b"            || group.epoch() != self.mls_epoch\n",
     b"            || { let _ = self.mls_epoch; false }\n",
     "studio_handoff_preparation_mls_change_rejects_next_signature_with_same_owner",
     "changed MLS epoch authorized another prepared signature"),
    ("owner", "crates/catcoms-replication/src/studio/epoch/handoff.rs",
     b"            || self.gate.owner()\n                != DeviceId::from_public_key_bytes(&overlay.receipt().owner_public_key)\n",
     b"            || { let _ = self.gate.owner(); false }\n",
     "studio_handoff_preparation_source_owner_must_match_verified_authority",
     "captured source owner bypassed verified handoff authority"),
    # SIGN-TEST-002: the captured authority's receipt must be the branch's own.
    ("receipt", "crates/catcoms-replication/src/studio/overlay/handoff/preparation.rs",
     b"        if self.target != authority.target || active.receipt() != &authority.receipt {\n",
     b"        if self.target != authority.target || { let _ = (active.receipt(), &authority.receipt); false } {\n",
     "studio_handoff_preparation_refuses_authority_captured_for_another_receipt",
     "authority captured for one receipt prepared a branch opened on another"),
    # SIGN-TEST-001: pre-sign typed admission is the handoff path's only editor-cap check.
    ("local-policy", "crates/catcoms-replication/src/studio/epoch/handoff/preparation.rs",
     b"            source\n                .target\n                .local_policy(&projection, domain, &source.actor)?;\n",
     b"            let _ = source\n                .target\n                .local_policy(&projection, domain, &source.actor);\n",
     "studio_handoff_preparation_refuses_a_vault_decoded_branch_over_the_local_cap",
     "an over-cap branch reached signing past the local editor cap"),
    # SIGN-TEST-001, aggregate half: the probe gate's admission before any signature.
    ("probe-gate", "crates/catcoms-replication/src/studio/epoch/handoff/preparation.rs",
     b"            if probe.admit_local(crate::AdmittedOperation {\n",
     b"            if false && probe.admit_local(crate::AdmittedOperation {\n",
     "studio_handoff_preparation_refuses_an_honest_branch_over_the_successor_gate",
     "a branch over the successor's epoch budget reached signing"),
]


def run(test):
    env = os.environ.copy()
    env["CARGO_INCREMENTAL"] = "0"
    if os.name == "nt":
        env["_LINK_"] = "/DEBUG:NONE"
    return subprocess.run([
        "cargo", "test", "--locked", "-j", os.environ.get("CARGO_BUILD_JOBS", "4"),
        "--config", "profile.test.package.catcoms-replication.debug=0",
        "-p", "catcoms-replication", "--lib", PREFIX + test, "--", "--exact", "--nocapture",
    ], cwd=ROOT, env=env, text=True, encoding="utf-8", errors="replace",
       stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=900, check=False)


def check(name, path, anchor, replacement, test, assertion):
    source = ROOT / path
    original = source.read_bytes()
    if original.count(anchor) != 1:
        raise AssertionError(f"mutation anchor is not unique: {name}")
    # Keep the field referenced so strict CI warnings cannot mask the executed assertion.
    changed = original.replace(anchor, replacement, 1)
    logs = ROOT / "logs"
    logs.mkdir(exist_ok=True)
    try:
        source.write_bytes(changed)
        result = run(test)
        (logs / f"gate4-handoff-signing-{name}-mutated.log").write_text(result.stdout, encoding="utf-8")
        if not (result.returncode != 0 and assertion in result.stdout
                and "test result: FAILED. 0 passed; 1 failed;" in result.stdout):
            print(result.stdout, flush=True)
            raise AssertionError(f"mutation did not fail at its intended executed assertion: {name}")
        print(f"DETECTED {name}: {assertion}", flush=True)
    finally:
        if source.read_bytes() != changed:
            raise AssertionError("signing source changed concurrently")
        source.write_bytes(original)
        assert source.read_bytes() == original
        print(f"RESTORED {name} source byte-for-byte", flush=True)
    result = run(test)
    (logs / f"gate4-handoff-signing-{name}-restored.log").write_text(result.stdout, encoding="utf-8")
    if result.returncode != 0 or "test result: ok. 1 passed; 0 failed;" not in result.stdout:
        print(result.stdout, flush=True)
        raise AssertionError(f"restored regression failed: {name}")
    print(f"PASS restored {name} regression", flush=True)


def main():
    for mutation in MUTATIONS:
        check(*mutation)


if __name__ == "__main__":
    main()
