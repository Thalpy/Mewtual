"""Require the overlay-lifecycle regressions to catch isolated guard removals.

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
    # --- the branch-generation namespace ---
    (
        "admission-not-trusted", "catcoms-replication", REPL_TESTS,
        f"{REPL}/overlay/handoff.rs",
        "if generation != self.next_generation()? {\n            return Err(ReplError::IntentConflict);\n        }",
        "let _ = generation;",
        "lifecycle::a_fabricated_admission_cannot_mint_a_branch_at_a_chosen_generation",
        "would give a new branch its identity",
    ),
    # The expected assertion here is NOT the test's generation assertion, and the difference is
    # worth reading rather than fixing away.
    #
    # When this mutation was written by hand it failed at "must take the next generation". It no
    # longer reaches that line: the disposal work later added a structural rule to `validate` - a
    # live branch beside a retained disposal must be a strictly later generation - and `append`
    # now refuses outright, so the test dies at its `expect` several lines earlier.
    #
    # The guard is therefore anchored twice and the stronger one fires first, which is the right
    # outcome and not a reason to weaken either. What it does mean is that under THIS mutation the
    # test's own generation assertions are unreachable and so have no mutant of their own; they are
    # anchored by `a_fabricated_admission_cannot_mint_a_branch_at_a_chosen_generation` above, which
    # reaches the same namespace through the admission path where no disposal exists to catch it.
    (
        "append-mints-next-generation", "catcoms-replication", REPL_TESTS,
        f"{REPL}/overlay/handoff.rs",
        "if minting {\n            next.branch_generation = self.next_generation()?;\n        }",
        "let _ = minting;",
        "lifecycle::appending_where_no_branch_exists_takes_the_next_generation",
        "the first Save after a disposal is ordinary and must work",
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
            # The mutation must fail THIS test, at THIS assertion, and alone. "0 passed; 1 failed"
            # is what makes it alone: a mutation that also breaks a sibling is not isolated, and an
            # isolated guard is the whole claim.
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
