"""Isolated executed-assertion checks for Flow R, the staged resolution of an interrupted handoff.

Design 6.4.2 in docs/GATE4-AGENT-1-DESIGN.md. Each entry removes one guard Flow R adds and names
the test assertion that must fail for it. Run only after normal regressions pass, and with
RUSTFLAGS='-D warnings' as CI builds: every mutant keeps every item used, so a dead-code error
can never pass for a detection. Source bytes are restored exactly after every mutation;
compilation failures, zero-test filters and incidental panics never count as detection.
Optional command-line names select a subset for diagnosis without changing the CI default.
"""
from pathlib import Path
import os
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
STORE = "store::epoch_studio::tests::rotation::overlay::handoff::resolution::"
RECEIVER = "studio::receiver::catchup::tests::"
RESOLUTION = "crates/catcoms-app/src/store/epoch_studio/resolution.rs"
INVENTORY = "crates/catcoms-app/src/store/epoch_recovery/inventory.rs"
RUNTIME = "crates/catcoms-app/src/studio/receiver/handoff.rs"
ROTATION = "crates/catcoms-app/src/studio/receiver/catchup/rotation.rs"
CATCHUP = "crates/catcoms-app/src/studio/receiver/catchup.rs"
REGISTRY = "crates/catcoms-app/src/studio/receiver/catchup/registry_runtime.rs"
REPLAY = "crates/catcoms-app/src/studio/receiver/replay.rs"
COMMAND = ["cargo", "test", "--locked", "-j", "4", "--config",
           "profile.test.package.catcoms-app.debug=0", "-p", "catcoms-app", "--lib"]
MUTATIONS = [
    # R3's intent comparison, as a branch and as a digest. Without either, a record a fence has
    # already resolved is no longer recognised, and R3 refuses instead of standing aside.
    ("intent-changed-branch", RESOLUTION,
     "        if let StampedRecords::IntentChanged = records {",
     "        if matches!(records, StampedRecords::IntentChanged) && false {",
     STORE + "flow_r_stands_aside_when_a_fence_resolved_first",
     "R3 refused a record a fence had already resolved"),
    ("intent-digest", RESOLUTION,
     "!= Some(stamp.intent) {",
     "!= Some(stamp.intent) && false {",
     STORE + "flow_r_stands_aside_when_a_fence_resolved_first",
     "R3 refused a record a fence had already resolved"),
    # HIGH-1: the source comparison and the synchronous fallback it selects. Without either, a
    # received operation on the Prepared destination makes R3 refuse and back off.
    ("source-digest", RESOLUTION,
     "!= Some(stamp.source) {",
     "!= Some(stamp.source) && false {",
     STORE + "flow_r_resolves_at_once_when_a_received_operation_changed_the_source",
     "a changed source must resolve at once, not refuse and back off"),
    ("source-fallback", RESOLUTION,
     "        if matches!(records, StampedRecords::SourceChanged) || owner_moved {",
     "        if false && (matches!(records, StampedRecords::SourceChanged) || owner_moved) {",
     STORE + "flow_r_resolves_at_once_when_a_received_operation_changed_the_source",
     "a changed source must resolve at once, not refuse and back off"),
    # M-2: R3 checks the worker's accounting for the source rather than trusting it. The whole
    # call goes: `verify_record` also invalidates the budget on a mismatch, so a mutant that only
    # ignored its result would still make the later write refuse, and survive for that reason.
    ("verify-record", RESOLUTION,
     "        budget\n"
     "            .storage\n"
     "            .verify_record(\n"
     "                &StorageScope::new(server, &document.server_id).map_err(invalid)?,\n"
     "                *blake3::hash(&source_scope).as_bytes(),\n"
     "                Some(observed),\n"
     "            )\n"
     "            .map_err(invalid)?;\n",
     "",
     STORE + "flow_r_refuses_a_plan_whose_accounting_disagrees_with_the_inventory",
     "R3 trusted the worker's accounting for the source"),
    # M-3: the shape of the carried next state, per arm.
    ("shape-returned", RESOLUTION,
     "if !returned_shape(metadata, &next) {",
     "if !returned_shape(metadata, &next) && false {",
     STORE + "flow_r_refuses_a_next_state_built_for_the_other_outcome",
     "R3 wrote a next state shaped for the other outcome"),
    ("shape-completed", RESOLUTION,
     "if !completed_shape(metadata, &next, target, &unit)? {",
     "if !completed_shape(metadata, &next, target, &unit)? && false {",
     STORE + "flow_r_refuses_a_next_state_built_for_the_other_outcome",
     "R3 wrote a next state shaped for the other outcome"),
    # R1's framing-only Hold exit: without it a Hold record reaches a worker and its restore.
    ("hold-exit", RESOLUTION,
     "            == StudioHandoffEvidence::Hold\n        {",
     "            == StudioHandoffEvidence::Hold\n            && false\n        {",
     STORE + "flow_r_holds_at_r1_with_no_restore",
     "R1 did not stop a Hold record before a worker"),
    # The re-review's M-1: R1 evicts a stale cached version, or IfVacant refuses the warm install
    # and R3's scan restores the source inline. The mutant evicts under the wrong family.
    ("stale-eviction", INVENTORY,
     "        self.inventory_cache.evict_mismatch(\n"
     "            (EpochRecordKind::Studio,",
     "        self.inventory_cache.evict_mismatch(\n"
     "            (EpochRecordKind::Registry,",
     STORE + "flow_r_restores_nothing_under_custody_from_a_stale_cache",
     "a stale cached version refused the warm install"),
    # HIGH-2: the runtime warms the inventory with R2's validation before R3's budget.
    ("warm-install", RUNTIME,
     "            store.warm_studio_resolution(plan);\n",
     "            let _ = plan;\n",
     RECEIVER + "flow_r_resolves_an_interrupted_handoff_through_scheduled_turns",
     "a custody turn restored the source while resolving"),
    # The probe routes a Prepared branch to Flow R; without it H1 resolves it synchronously,
    # restoring the source under custody.
    ("probe-route", RUNTIME,
     "        if prepared {\n            let started",
     "        if false && prepared {\n            let started",
     RECEIVER + "flow_r_resolves_an_interrupted_handoff_through_scheduled_turns",
     "a custody turn restored the source while resolving"),
    # Owner rotation skips a Prepared target instead of letting the read-only service path's
    # refusal pause all of receive.
    ("rotation-skip", ROTATION,
     "        if prepared {\n            self.owner_target = None;",
     "        if false && prepared {\n            self.owner_target = None;",
     RECEIVER + "owner_rotation_skips_a_prepared_document_instead_of_pausing_receive",
     "a background turn failed while a watched document's handoff was Prepared"),
    # The same skip on the other two rails that read a watched target through the refusing
    # service path (Flow R's implementation review, HIGH-1): the client pass, which needs a proven
    # peer to run, and Registry maintenance.
    ("client-pass-skip", CATCHUP,
     "        if Self::handoff_prepared(server, store, id, watch.target) {",
     "        if false && Self::handoff_prepared(server, store, id, watch.target) {",
     RECEIVER + "the_client_pass_skips_a_prepared_document_instead_of_pausing_receive",
     "a background turn failed while a watched document's handoff was Prepared"),
    ("registry-skip", REGISTRY,
     "        if Self::handoff_prepared(server, store, id, target) {",
     "        if false && Self::handoff_prepared(server, store, id, target) {",
     RECEIVER + "registry_maintenance_skips_a_prepared_document_instead_of_pausing_receive",
     "a background turn failed while a watched document's handoff was Prepared"),
    # The re-review's MEDIUM-1: replay is a fourth such rail, through Apply rather than a read. Its
    # skip has two halves, the new-pass filter and the active-pass check, so the mutant removes
    # both in one span.
    ("replay-skip", REPLAY,
     "                        && !super::catchup::CatchupRuntime::handoff_prepared(server, store, id, b.0)\n"
     "                })\n"
     "            });\n"
     "        let Some((target, epoch)) = target else {\n"
     "            return Ok(None);\n"
     "        };\n"
     "        if super::catchup::CatchupRuntime::handoff_prepared(server, store, id, target) {\n",
     "                        && !(false && super::catchup::CatchupRuntime::handoff_prepared(server, store, id, b.0))\n"
     "                })\n"
     "            });\n"
     "        let Some((target, epoch)) = target else {\n"
     "            return Ok(None);\n"
     "        };\n"
     "        if false && super::catchup::CatchupRuntime::handoff_prepared(server, store, id, target) {\n",
     RECEIVER + "replay_waits_for_a_prepared_document_to_be_resolved",
     "replay took up a document whose handoff was Prepared"),
    # MEDIUM-1: a resolve job whose budget will not build releases its permit and admission.
    ("budget-release", RUNTIME,
     "            if matches!(\n"
     "                self.handoff.job.as_ref().map(|job| &job.stage),\n"
     "                Some(HandoffStage::ResolveReady(..))\n",
     "            if false && matches!(\n"
     "                self.handoff.job.as_ref().map(|job| &job.stage),\n"
     "                Some(HandoffStage::ResolveReady(..))\n",
     RECEIVER + "a_resolve_job_whose_budget_will_not_build_releases_its_permit",
     "a resolve job kept its permit through a budget failure"),
    # A resolve job carries no transfer authority, so the authority check leaves it alone.
    ("resolve-authority", RUNTIME,
     "                        authority: None,",
     "                        authority: Some((0, 0)),",
     RECEIVER + "a_resolve_job_is_not_abandoned_by_the_transfer_authority_check",
     "the transfer authority check abandoned a resolve job"),
]


def run(test):
    env = os.environ.copy()
    env["CARGO_INCREMENTAL"] = "0"
    if os.name == "nt":
        env["_LINK_"] = "/DEBUG:NONE"
    return subprocess.run(COMMAND + [test, "--", "--exact", "--nocapture"],
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
            (logs / f"gate4-resolution-mutation-{name}.log").write_text(result.stdout,
                                                                         encoding="utf-8")
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
        (logs / f"gate4-resolution-restored-{name}.log").write_text(result.stdout,
                                                                     encoding="utf-8")
        if result.returncode != 0 or "test result: ok. 1 passed; 0 failed;" not in result.stdout:
            print(result.stdout, flush=True)
            raise AssertionError(f"restored regression failed: {name}")
        print(f"PASS restored {name}", flush=True)


if __name__ == "__main__":
    main()
