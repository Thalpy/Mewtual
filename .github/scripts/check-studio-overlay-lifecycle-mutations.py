"""Require the overlay-lifecycle regressions to catch guard removals at their named assertions.

**Scope, stated precisely because it used to be overstated:** each entry runs ONE test with
`--exact`, so this script establishes "detected at the named assertion, and the restored source
passes". It does not establish isolation - no sibling test is selected, so a mutant that also breaks
one would go unnoticed here. Isolation rests on the hand-runs behind each entry.

Covers Agent 2's scope: the draft archive and its release, the disposal transaction, the
branch-generation namespace, and the owner-tenure observation rule.

Every mutation here was first proved by hand: each fails its own named test, at its own intended
assertion, and the restored source passes. The script exists so that stays true - a guard that stops
being anchored shows up as a mutation that no longer fails.

Run from a Rust test environment; no native/UI build is needed. Logs stay local under logs/.
Restoration is byte-exact and refuses to overwrite an unrelated concurrent source change.

`RUST_MIN_STACK` is set because these suites currently need more thread stack than the default. That
is a workaround for a stack regression elsewhere in the tree, not a property of this scope, and it is
set here so a CI run and a local run agree rather than one of them mysteriously aborting.
"""
from pathlib import Path
import os
import subprocess


ROOT = Path(__file__).resolve().parents[2]

APP = "crates/catcoms-app/src/store"
REPL = "crates/catcoms-replication/src/studio"

APP_TESTS = "store::epoch_studio::tests::rotation::overlay::"
REPL_TESTS = "studio::epoch::owner::tests::"

# (name, package, test-path prefix, file, before, after, test, expected assertion text)
MUTATIONS = [
    # --- the archive release path: the only thing that destroys preserved evidence ---
    (
        "release-identity", "catcoms-app", APP_TESTS,
        f"{APP}/epoch_draft_archive.rs",
        "if archive.archive_id().map_err(invalid)? != expected_archive {",
        # Inverted rather than short-circuited. `if false {` drops the only use of both `archive`
        # and `expected_archive`, and under the `-D warnings` this script now sets, two unused
        # variables are compile ERRORS. A mutant that does not build proves nothing about the guard
        # it names, and the harness correctly refused to call that a detection. Inverting keeps both
        # bindings live and negates exactly the identity comparison. Found by Agent 3 in CI.
        "if archive.archive_id().map_err(invalid)? == expected_archive {",
        "archive::release_refuses_an_archive_other_than_the_one_it_names",
        "release must refuse a content it was not asked to destroy",
    ),
    (
        "release-budget-verify", "catcoms-app", APP_TESTS,
        f"{APP}/epoch_draft_archive.rs",
        "budget\n            .verify_record(&storage_scope, id, Some(observed))\n            .map_err(invalid)?;",
        "let _ = (&storage_scope, id, observed);",
        "archive::release_refuses_a_record_its_budget_does_not_know",
        "release must refuse when its budget never accounted the record it is destroying",
    ),
    (
        "release-closes-budgets", "catcoms-app", APP_TESTS,
        f"{APP}/epoch_draft_archive.rs",
        "budget.invalidate();\n        intents.begin_write();",
        "intents.begin_write();",
        "archive::a_release_that_fails_after_the_unlink_still_closes_both_budgets",
        "left a usable storage budget describing a record",
    ),
    (
        "release-scope-binding", "catcoms-app", APP_TESTS,
        f"{APP}/epoch_draft_archive.rs",
        "if d.get_bytes().map_err(invalid)? != scope {\n        return Err(invalid(\"wrong sealed scope\"));\n    }",
        "let _ = (d.get_bytes().map_err(invalid)?, scope);",
        "archive::a_record_sealed_for_another_slot_is_refused_by_the_addressed_readers",
        "must refuse a record sealed for another slot",
    ),
    # --- the disposal transaction: D1 and D4 ---
    (
        "dispose-authorship", "catcoms-app", APP_TESTS,
        f"{APP}/epoch_intents/disposal.rs",
        "if active.author() != device.device_id() {",
        "if false {",
        "disposal::a_group_member_who_did_not_author_the_branch_cannot_dispose_of_it",
        "a member who did not author the branch must not dispose of it",
    ),
    (
        "dispose-envelope-match", "catcoms-app", APP_TESTS,
        f"{APP}/epoch_intents/disposal.rs",
        ".matches_branch(active, &state.ledger)",
        ".matches_branch(active, &state.ledger)\n                    .map(|_| true)",
        "disposal::a_preserving_disposal_refuses_an_archive_whose_entries_are_not_the_branchs",
        "must not authorise destroying it",
    ),
    # The branch and generation compares are removed TOGETHER. They are deliberately redundant for
    # a correctly derived archive (`branch_id` is `H(basis, generation)`), so removing one alone
    # leaves the other refusing; the semantic defence is "an archive of another generation is not
    # this branch's", and that is what this mutant takes away. Without it, G3 is disposed of as
    # `Preserved` under G1's archive.
    (
        "dispose-generation-binding", "catcoms-app", APP_TESTS,
        f"{APP}/epoch_intents/disposal.rs",
        "                    || Some(archive.branch()) != metadata.branch_id()\n                    || archive.generation() != metadata.branch_generation()\n",
        "",
        "disposal::a_preserving_disposal_refuses_an_earlier_generations_archive_of_identical_work",
        "an archive of an earlier generation must not authorise disposing of a later one",
    ),
    # Membership on its own: the real author, removed. The stranger test cannot isolate this, since
    # a stranger fails authorship too, and it stays green under this mutant.
    (
        "dispose-membership", "catcoms-app", APP_TESTS,
        f"{APP}/epoch_intents/disposal.rs",
        "        if document.server_id != group.group_id()\n            || group.member_signature_key(&device.device_id()).as_deref()\n                != Some(device.public_key_bytes().as_slice())\n        {\n            return Err(invalid(\"disposal author is not a current local member\"));",
        "        if document.server_id != group.group_id() {\n            return Err(invalid(\"disposal author is not a current local member\"));",
        "disposal::the_branchs_own_author_cannot_dispose_of_it_once_removed_from_the_group",
        "a removed author must not dispose of the branch",
    ),
    # --- evidence before removal: a preserving disposal must establish its archive durably first ---
    #
    # Swallowing the barrier's error is caught, but read what catches it. The disposal is STILL
    # refused - by the budget, because the failed sync closed both budgets before its I/O and the
    # removal write then demands reconciliation. So "nothing is removed" is defended twice, and only
    # the test's assertion on the refusal MESSAGE distinguishes the barrier's own refusal from the
    # budget's. That is why the expected text below is the message assertion and not the expect_err.
    #
    # Skipping the barrier call entirely is the more alarming mutant - the disposal then SUCCEEDS and
    # records `Preserved` without the archive ever being made durable - but it cannot be expressed as
    # one string replacement, so it rests on the hand-run recorded in the status doc, where it failed
    # both ordering tests at their own assertions with the other ten disposal tests green.
    (
        "dispose-archive-durability-propagates", "catcoms-app", APP_TESTS,
        f"{APP}/epoch_intents/disposal.rs",
        "                })?;\n                StudioDisposalDecision::Preserve { archive: record.id }",
        "                }).ok();\n                StudioDisposalDecision::Preserve { archive: record.id }",
        "disposal::a_preserving_disposal_whose_archive_cannot_be_made_durable_removes_nothing",
        "the refusal must be the durability barrier's",
    ),
    # --- copy into current: the object probe at C3 and C4, each on its own ---
    #
    # The planner runs with no store, so it calls a dangling `PutObject` Ready; only the probe can
    # tell. The C3 mutant keeps the probe call (so every binding stays live under -D warnings) and
    # ignores its answer; the C4 mutant does the same at apply. Each fails its own assertion.
    (
        "copy-probe-preview", "catcoms-app", "studio::copy::tests::",
        "crates/catcoms-app/src/studio/copy.rs",
        "Self::probe_copy_object(store, server, group, device, &plan)\n        })? {",
        "Self::probe_copy_object(store, server, group, device, &plan).map(|_| true)\n        })? {",
        "the_copy_probe_refuses_an_object_that_is_missing_or_disappears_before_apply",
        "C3 must tell the user the object is missing",
    ),
    (
        "copy-probe-apply", "catcoms-app", "studio::copy::tests::",
        "crates/catcoms-app/src/studio/copy.rs",
        "if !Self::probe_copy_object(store, server, group, device, &plan)? {",
        "if !Self::probe_copy_object(store, server, group, device, &plan).map(|_| true)? {",
        "the_copy_probe_refuses_an_object_that_is_missing_or_disappears_before_apply",
        "C4 must refuse to publish an entry",
    ),
    # --- the branch-generation namespace ---
    (
        "admission-not-trusted", "catcoms-replication", REPL_TESTS,
        f"{REPL}/overlay/handoff.rs",
        "if generation != self.next_generation()? {\n            return Err(ReplError::IntentConflict);\n        }",
        "let _ = generation;",
        "lifecycle::a_fabricated_admission_cannot_mint_a_branch_at_a_chosen_generation",
        "would give a new branch its identity",
    ),
    # `append` no longer opens a branch: admission is the only way in. The mutant restores the old
    # third door, so a state with no live branch silently gets one. Under it `validate`'s "a live
    # branch beside a retained disposal must be strictly later" rule may also refuse, so the
    # expected message is the test's own refusal assertion, which runs before any of that.
    (
        "append-refuses-without-a-live-branch", "catcoms-replication", REPL_TESTS,
        f"{REPL}/overlay/handoff.rs",
        "let Some(mut active) = self.active.clone() else {\n            return Err(ReplError::IntentConflict);\n        };",
        "let mut active = self.active.clone().unwrap_or_else(|| StudioOverlay::new(basis));",
        "lifecycle::appending_where_no_branch_exists_is_refused_and_admission_opens_the_next_one",
        "append must not open a branch that admission never admitted",
    ),
    # --- M-1 on the receive path: the only defence against a one-commit remove-and-re-add ---
    #
    # Hand-run first: with this rule disabled the witness MERGES the hostile commit, so OpenMLS does
    # not make it redundant. `&& false` keeps `re_adds_committer` used under -D warnings.
    (
        "m1-receive-refusal", "catcoms-mls", "group::m1_tests::",
        "crates/catcoms-mls/src/group.rs",
        "if re_adds_committer {",
        "if re_adds_committer && false {",
        "a_witness_refuses_one_commit_that_removes_the_committer_and_re_adds_its_device_id",
        "a witness must refuse a commit that removes the committer",
    ),
    # --- the owner-tenure observation rule ---
    (
        "tenure-leaf-arm", "catcoms-sync", "owner_tenure::tests::",
        "crates/catcoms-sync/src/owner_tenure.rs",
        "} else if before.owner.is_some() && before.owner == after.owner && before.leaf != after.leaf\n        {",
        "} else if false {",
        "owner_tenure_same_owner_on_a_new_leaf_identity_starts_a_new_tenure",
        "is a new tenure, not preserved knowledge",
    ),
]


def run(package, prefix, test):
    env = os.environ.copy()
    env["CARGO_INCREMENTAL"] = "0"
    env["RUST_MIN_STACK"] = "33554432"
    # Set here rather than inherited, because inheriting it is exactly what went wrong.
    #
    # The workflow sets `-D warnings` at job level, so CI built every mutant with it while a local
    # run built them without. One mutant then behaved differently in the two places: it left two
    # bindings unused, which is a warning locally and an error in CI, so it passed here and failed
    # there. A harness whose result depends on the caller's environment is not evidence, and the
    # divergence let this script report nine detections while the job was red.
    env["RUSTFLAGS"] = "-D warnings"
    if os.name == "nt":
        env["_LINK_"] = "/DEBUG:NONE"
    command = [
        "cargo", "test", "--locked", "-j", "1", "--config",
        f"profile.test.package.{package}.debug=0", "-p", package, "--lib",
    ]
    return subprocess.run(
        command + [prefix + test, "--", "--exact", "--nocapture"], cwd=ROOT,
        env=env, text=True, encoding="utf-8", errors="replace",
        stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=1800, check=False,
    )


def main():
    log_dir = ROOT / "logs"
    log_dir.mkdir(exist_ok=True)
    for name, package, prefix, path, before, after, test, assertion in MUTATIONS:
        source = ROOT / path
        original = source.read_bytes()
        before, after = before.encode(), after.encode()
        if original.count(before) != 1:
            raise AssertionError(f"mutation anchor is not unique: {name}")
        changed = original.replace(before, after, 1)
        try:
            source.write_bytes(changed)
            result = run(package, prefix, test)
            (log_dir / f"gate4-overlay-lifecycle-mutation-{name}.log").write_text(
                result.stdout, encoding="utf-8"
            )
            # The mutation must fail THIS test, at THIS assertion.
            #
            # **This script does NOT check isolation, and an earlier version of this comment claimed
            # it did.** The run below is `--exact` on one fully qualified test, so no sibling is ever
            # selected; "0 passed; 1 failed" therefore describes only that one test and says nothing
            # about whether the mutant also breaks others. A review pointed this out and was right.
            #
            # Isolation for these entries rests on the hand-runs recorded behind each of them, not on
            # anything this script observes. Do not report a passing run here as "the mutation is
            # isolated" - the supported claim is "the mutation was detected at its named assertion
            # and the restored source passes".
            if not (
                result.returncode != 0
                and assertion in result.stdout
                and "test result: FAILED. 0 passed; 1 failed;" in result.stdout
            ):
                print(result.stdout, flush=True)
                raise AssertionError(f"mutation did not fail at its intended assertion: {name}")
            print(f"DETECTED {name}: {assertion}", flush=True)
        finally:
            if source.read_bytes() != changed:
                raise AssertionError(f"source changed concurrently: {path}")
            source.write_bytes(original)
            assert source.read_bytes() == original
            print(f"RESTORED {path} byte-for-byte", flush=True)
    # Recompile the restored sources once, then require each exercised regression to pass. Green here
    # is not the evidence - the failing mutants above are - but a restored suite that does not pass
    # would mean the restoration itself was wrong.
    for name, package, prefix, _, _, _, test, _ in MUTATIONS:
        result = run(package, prefix, test)
        (log_dir / f"gate4-overlay-lifecycle-restored-{name}.log").write_text(
            result.stdout, encoding="utf-8"
        )
        if result.returncode != 0 or "test result: ok. 1 passed; 0 failed;" not in result.stdout:
            print(result.stdout, flush=True)
            raise AssertionError(f"restored regression failed: {name}")
        print(f"PASS restored {name}", flush=True)


if __name__ == "__main__":
    main()
