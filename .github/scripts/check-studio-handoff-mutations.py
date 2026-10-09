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
# The receiver's actor-level tests live outside PREFIX; an entry names one in full from here.
RECEIVER = "studio::receiver::catchup::tests::"
CORE = "crates/catcoms-replication/src/studio/overlay/handoff.rs"
STORE = "crates/catcoms-app/src/store/epoch_studio/handoff.rs"
COMMAND = ["cargo", "test", "--locked", "-j", "4", "--config",
           "profile.test.package.catcoms-app.debug=0", "-p", "catcoms-app", "--lib"]
MUTATIONS = [
    # The channel guard on a completed-handoff retry. Since Flow S began carrying branch identity,
    # S1 classifies through `classify_request` FIRST, and its own `check_target` is what refuses a
    # retry for another channel. `completed_retry`'s check (the original anchor here) now runs only
    # as the Unmatched fallback, after classification has already passed on the same target, so
    # removing it changes nothing any Save can observe - this entry stopped detecting anything the
    # moment that ordering landed. The anchor is the guard that actually stands in the way.
    ("completed-target", CORE,
     "self.check_target(target)?;\n        if self.branch_id() == Some(branch) {",
     "let _ = target;\n        if self.branch_id() == Some(branch) {",
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
    # The test now proves the namespace and the floor separately: the forgotten retry is refused
    # by the namespace as stale, and a request prepared after the rewind - which the namespace
    # admits - is what the floor alone must stop. Disabling the floor fails that second half, with
    # the request accepted.
    ("retry-floor", CORE, "if closed < floor {", "if closed < floor && false {",
     "metadata::studio_overlay_handoff_rollover_floor_rejects_forgotten_retry_after_rewind",
     "a freshly prepared request on a rewound basis crossed the persisted floor"),
    ("acceptance-order", "crates/catcoms-replication/src/studio/epoch/handoff/preparation.rs",
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
    # Design 9.1: H5 resolves from the verified candidate, never by restoring the source it just
    # wrote. Forcing the restore back must be seen as a restore.
    ("verified-restore", STORE,
     "verified.into_checked(self, server, group, target, &mut budget.storage)?",
     "{ let _ = verified; self.checked_studio_source(server, group, target, device, false, &mut budget.storage)? }",
     "persisted::studio_overlay_handoff_commit_restores_nothing",
     "H5 restored a source"),
    # 9.1.1 A1: H5's header-only Index object check is the only thing between a referenced
    # Flipnote cleaned up after H1 and a durable Index entry naming nothing. The mutant empties
    # the check's loop from inside the helper. It used to replace the call site, which left
    # `check_index_objects_at_commit` dead, so under CI's `-D warnings` the build failed before
    # the test ran and this entry failed CI from 17dd54fc to fee9b993 without testing anything.
    ("index-commit", STORE,
     "        for object in objects {\n            let flipnote = StudioTarget::Flipnote { channel, object };\n            let logical = flipnote.document(&group.group_id()).map_err(invalid)?;\n            let scope = scope_bytes(server, &logical)?;\n            let record = self.read_studio_record(&scope)?.ok_or_else(unavailable)?;",
     "        for object in objects.into_iter().filter(|_| false) {\n            let flipnote = StudioTarget::Flipnote { channel, object };\n            let logical = flipnote.document(&group.group_id()).map_err(invalid)?;\n            let scope = scope_bytes(server, &logical)?;\n            let record = self.read_studio_record(&scope)?.ok_or_else(unavailable)?;",
     "eligibility::studio_overlay_handoff_rechecks_index_object_sources_at_commit_not_only_at_capture",
     "H5 committed an Index entry pointing at a source"),
    # 9.1.1 step 5: a proof whose evidence is not Complete must never be resolved, in particular
    # never returned to Active from a candidate.
    ("verified-evidence", STORE,
     "if from_candidate && evidence != StudioHandoffEvidence::Complete {",
     "if false && from_candidate && evidence != StudioHandoffEvidence::Complete {",
     "persisted::studio_overlay_handoff_verified_resolution_accepts_only_complete_evidence",
     "a proof with Absent evidence was resolved"),
    # Design 18.3 review, F7: of the proof's checks, the size-and-digest comparison is the only one
    # that sees the trailing link byte. A record with it dropped decodes unlinked with the
    # candidate's own snapshot, so the snapshot hash and the link check both pass. The mutant
    # removes the comparison. The record is then still refused, later, by resolve's flush-only
    # length fence ("retry file changed"), so the assertion this entry names is the one that pins
    # the refusal to the proof. The field checks themselves (channel, snapshot hash, link) have no
    # entries: each is covered by the digest, so removing one alone is undetectable.
    ("proof-digest", "crates/catcoms-app/src/store/epoch_studio/source/persisted.rs",
     "if landed.physical_bytes != version.bytes || blake3::hash(&landed.plain) != version.digest {",
     "if false && (landed.physical_bytes != version.bytes"
     " || blake3::hash(&landed.plain) != version.digest) {",
     "persisted::studio_overlay_handoff_refuses_a_persisted_source_whose_link_was_dropped",
     "refused by something other than the post-write proof"),
    # Design M1 and M2 (design 18.3 review, F8): the plan's currency check keeps only the size of
    # the intent, then of the source wrapper. Redundant by design with H5, so the observation is
    # the early one the design names: a stale plan would reach a signing turn. For the intent the
    # test asserts that gate only after driving the plan through H5, so this entry also pins H5's
    # step 6: with step 6 removed as well, the test fails earlier, at "H5 committed a plan", and
    # this entry reports the wrong assertion (verified 2026-10-09).
    ("plan-intent-digest", "crates/catcoms-app/src/store/epoch_studio/handoff_capture.rs",
     "if (blake3::hash(&intent.plain), intent.physical_bytes) != stamp.intent {",
     "if intent.physical_bytes != stamp.intent.1 {",
     "fences::studio_overlay_handoff_plan_is_stale_after_a_same_size_wrapper_replacement",
     "a stale plan reached a signing turn: the intent wrapper"),
    ("plan-source-digest", "crates/catcoms-app/src/store/epoch_studio/handoff_capture.rs",
     "Ok((blake3::hash(&source.plain), source.physical_bytes) == stamp.source)",
     "Ok(source.physical_bytes == stamp.source.1)",
     "fences::studio_overlay_handoff_plan_is_stale_after_a_same_size_wrapper_replacement",
     "a stale plan reached a signing turn: the source wrapper"),
    # Design M6 (design 18.3 review, F8, which found the probe unbuilt): H1's pristine-successor
    # probe answers "transferable" whatever the header says. Redundant by design with H2's
    # check_overlay_successor, so the observation is that H2 would start.
    ("successor-probe", "crates/catcoms-app/src/store/epoch_studio/handoff.rs",
     "StudioEpoch::overlay_successor_hold_in_vault(bytes, target, owner, overlay)",
     "StudioEpoch::overlay_successor_hold_in_vault(bytes, target, owner, overlay)"
     ".map(|hold| hold.filter(|_| false))",
     "fences::studio_overlay_handoff_h1_refuses_a_non_pristine_successor_before_capture",
     "H2 started for a non-pristine successor"),
    # 9.1.1 A3: a proof cannot be spent after a five-family write landed since verification.
    ("proof-generation", "crates/catcoms-app/src/store/epoch_studio/source/persisted.rs",
     "&& Arc::ptr_eq(&self.generation, &store.inventory_generation);",
     "&& { let _ = &self.generation; true };",
     "persisted::studio_overlay_handoff_verified_source_binds_target_candidate_and_generation",
     "a proof was spent after a write landed"),
    # Design 7.3's placement answer for H3 (design 18.3 review, F2, whose reviewer forced this
    # predicate to `false` and saw nothing fail). The test drives production turns while a second
    # member's checkpoint request waits, so these entries name the receiver's test in full.
    ("handoff-priority", "crates/catcoms-app/src/studio/receiver.rs",
     "        server.sync.has_epoch_service_interest()\n"
     "            || self\n"
     "                .watches\n"
     "                .iter()\n"
     "                .any(|(w, _)| server.sync.studio_has_inbound(&w.inner))\n"
     "            || self.catchup.result_parked()\n"
     "            || self.catchup.service_owed(server)\n",
     "        false\n"
     "            && (server.sync.has_epoch_service_interest()\n"
     "                || self\n"
     "                    .watches\n"
     "                    .iter()\n"
     "                    .any(|(w, _)| server.sync.studio_has_inbound(&w.inner))\n"
     "                || self.catchup.result_parked()\n"
     "                || self.catchup.service_owed(server))\n",
     RECEIVER + "a_signing_visit_yields_to_a_members_checkpoint_request_until_it_is_served",
     "a signing slice ran while a member's checkpoint request waited"),
    # Each term the request exercises, removed alone: first while it is queued as service
    # interest, then once it is reserved and its source installed but it is still unanswered.
    ("handoff-priority-service", "crates/catcoms-app/src/studio/receiver.rs",
     "        server.sync.has_epoch_service_interest()\n            || self\n",
     "        (false && server.sync.has_epoch_service_interest())\n            || self\n",
     RECEIVER + "a_signing_visit_yields_to_a_members_checkpoint_request_until_it_is_served",
     "a signing slice ran while a member's checkpoint request waited"),
    ("handoff-priority-owed", "crates/catcoms-app/src/studio/receiver.rs",
     "            || self.catchup.service_owed(server)\n",
     "            || (false && self.catchup.service_owed(server))\n",
     RECEIVER + "a_signing_visit_yields_to_a_members_checkpoint_request_until_it_is_served",
     "a signing slice ran while a member's checkpoint request waited"),
    # The parked-result term. In the request's flow a parked preparation is always the reserved
    # request's own, so the owed term answers on the same turns and masks this one; a local owner
    # capture parks a result with no request behind it.
    ("handoff-priority-parked", "crates/catcoms-app/src/studio/receiver.rs",
     "            || self.catchup.result_parked()\n",
     "            || (false && self.catchup.result_parked())\n",
     RECEIVER + "a_parked_catch_up_result_alone_makes_a_signing_slice_yield",
     "a parked result did not make a signing slice yield"),
]


def qualified(test):
    """A name relative to PREFIX, or a crate-absolute one beginning at `RECEIVER`."""
    return test if test.startswith(RECEIVER) else PREFIX + test


def run(test):
    env = os.environ.copy()
    env["CARGO_INCREMENTAL"] = "0"
    if os.name == "nt":
        env["_LINK_"] = "/DEBUG:NONE"
    return subprocess.run(COMMAND + [qualified(test), "--", "--exact", "--nocapture"],
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
