"""Execute nonempty, exact-count joined-source cohorts independently of strict CI.

This is focused evidence only. It neither enables Chat V2 nor substitutes for full
root/native/frontend, strict-lint or dependency checks. --self-test runs no Cargo.
"""
from __future__ import annotations

import argparse
from pathlib import Path
import runpy
import re
import sys

REPO = Path(__file__).resolve().parents[2]
COHORTS = (
    ("core-read-accessors", "catcoms-replication", "v2_general::tests::read_accessors", 3),
    ("joined-facts", "catcoms-sync", "owner_v2::joined_source::tests", 4),
    ("joined-fence", "catcoms-sync", "owner_v2::joined_fence::tests", 18),
    # The eleven operation cases include both new V1 exclusion tests and the genuine
    # preexisting success controls, so they are not counted/run again as a separate cohort.
    ("owner-operations", "catcoms-sync", "owner_evidence::initial_tail::operation::tests", 11),
    ("owner-tenure", "catcoms-sync", "owner_tenure::tests", 17),
    ("founder-member", "catcoms-sync", "owner_v2::member::tests", 9),
    # Exercise the real retained producer and its Root owner separately. Counts
    # must not vanish behind a selector typo or a platform-only fixture.
    ("retained-bootstrap-signing", "catcoms-sync", "retained_bootstrap", 10),
    ("retained-bootstrap-owner", "catcoms-app", "retained_bootstrap", 3),
    # Includes the genuine whole-Host observation/ordinary-UI-lock distinction.
    ("native-key-lifecycle", "catcoms-rt", "native_chat_key::", 27),
    ("native-authentication-life", "catcoms-app", "chat_admission::revocation::tests", 5),
    ("native-warm-sdk-tail", "catcoms-sync", "native_warm_sdk_tail_", 1),
    ("native-founder-sdk", "catcoms-app", "native_sdk_", 5),
    ("native-network-identity", "catcoms-net", "seeded_transport_identity_data", 3),
    ("bounded-local-recovery", "catcoms-replication", "doc::saved_local_recovery::tests", 6),
    ("existing-local-recovery", "catcoms-replication", "doc::saved_local_chat_tests", 5),
    ("selected-history-ordering", "catcoms-replication", "v2_general::tests::selected_history", 12),
    ("fresh-founder-mount", "catcoms-app", "current_layout::tests::fresh_founder", 1),
    ("fresh-founder-driver", "catcoms-app", "founder::plan::source::warm::tests", 11 if sys.platform in ("win32", "linux") else 4),
    # Includes original physical custody and actual private native founder/SDK
    # production. Synthetic UI captures still prove no checked Tauri caller,
    # installed backend, joined source or operational cold SDK restore.
    ("native-registry-physical", "catcoms-app", "critical_namespaces::native_registry::", 283),
    ("current-cached-mount", "catcoms-app", "current_cached_mount_check_never_constructs_a_revocation_observer", 1),
    ("native-registry-directory", "catcoms-app", "native_registry_directory_capture", 1),
    ("native-registry-join-refusal", "catcoms-app", "native_registry_preparation_refuses_real", 1),
    ("native-registry-join-lifecycle", "catcoms-app", "native_registry_actual_join_lifecycle", 1),
    ("native-index2-join-lifecycle", "catcoms-app", "native_index2_actual_join_lifecycle", 1),
    ("native-namespace-pair-preservation", "catcoms-app", "native_namespace_pair_", 2),
    ("native-index3-interrupted-join", "catcoms-app", "lifecycle3_seed_claim_preserves_a_real_interrupted_join_stage", 1),
    ("native-index3-cold-join-role", "catcoms-app", "lifecycle3_actual_join_cold_open_retains_the_exact_selected_catalog_role", 1),
)


# These are artifact producers, not skipped acceptance regressions. Their real
# --ignored --exact executions remain mandatory in the index3/Native4 gates.
# The broad normal cohort must stay zero-ignored.
FIXTURE_PREFIX = "store::critical_namespaces::native_registry::lifecycle::cell::tests::compatibility::"
FIXTURE_TESTS = tuple(FIXTURE_PREFIX + suffix for suffix in (
    "generate_native3_fixtures", "verify_native3_predecessor_fixtures",
)) + tuple("store::critical_namespaces::native_registry::operational::sources::network::tests::compatibility::" + suffix
           for suffix in ("generate_native4_fixtures", "verify_native4_predecessor_fixtures"))
# The shared runner retains at most two matches PER pattern. Use one pattern
# per skip string so an extra matching name always becomes a rejected second
# match, even when a different ordinary test vanished and total count is equal.
INVENTORY_MATCHES = tuple(re.compile(r"^(.*" + re.escape(name) + r".*): test$")
                          for name in FIXTURE_TESTS)
INVENTORY_COUNT = re.compile(r"^(\d+) tests, (\d+) benchmarks$")


def validate_inventory(matches: list, counts: list, expected: int) -> None:
    if (matches != [[(name,)] for name in FIXTURE_TESTS]
            or counts != [(str(expected), "0")]):
        raise RuntimeError(f"native fixture inventory changed: {matches!r}, {counts!r}")


def inventory_self_test() -> None:
    good = [[(name,)] for name in FIXTURE_TESTS]
    validate_inventory(good, [("223", "0")], 223)
    validate_inventory(good, [("4", "0")], 4)
    for names, counts in (
        (good[:1], [("223", "0")]),
        ([good[0] + good[0], *good[1:]], [("223", "0")]),
        ([good[0] + [(FIXTURE_TESTS[0] + "_extra",)], *good[1:]], [("223", "0")]),
        ([*good[:3], good[3] + [("prefix_" + FIXTURE_TESTS[3],)]], [("223", "0")]),
        (good, [("224", "0")]), (good, [("223", "0"), ("223", "0")]),
        ([], [("223", "0")]),
    ):
        try:
            validate_inventory(names, counts, 223)
        except RuntimeError:
            continue
        raise AssertionError("missing/duplicate/expanded native fixture selection accepted")
    # Pin actual regex/runner composition as well as the result validator.
    expanded = "\n".join(name + ": test" for name in (*FIXTURE_TESTS, FIXTURE_TESTS[3] + "_extra"))
    # Per-line runner applies patterns to a line stripped of ANSI/newline.
    matches = [[m.groups() for line in expanded.splitlines()
                for m in pattern.finditer(line)][:2] for pattern in INVENTORY_MATCHES]
    try:
        validate_inventory(matches, [("223", "0")], 223)
    except RuntimeError:
        pass
    else:
        raise AssertionError("actual skip-prefix twin was hidden by the bounded matcher")


def check_normal_inventory(guard: dict, directory: Path, command: list[str], expected: int) -> None:
    for label, extra, total in (("all", [], expected + len(FIXTURE_TESTS)),
                                ("ignored", ["--ignored"], len(FIXTURE_TESTS))):
        code, _, timed_out, found = guard["run_logged"](
            "native-registry-inventory-" + label, command + ["--", "--list", *extra],
            directory, limit=30 * 60, markers=(*INVENTORY_MATCHES, INVENTORY_COUNT),
        )
        if code or timed_out:
            raise RuntimeError("native registry inventory could not be established")
        validate_inventory(found[:-1], found[-1], total)


def fresh_founder_cohort(cohorts: tuple) -> list:
    selected = [cohort for cohort in cohorts if cohort[0].startswith("fresh-founder-")]
    if selected != [
        ("fresh-founder-mount", "catcoms-app", "current_layout::tests::fresh_founder", 1),
        ("fresh-founder-driver", "catcoms-app", "founder::plan::source::warm::tests", 11 if sys.platform in ("win32", "linux") else 4),
    ]:
        raise RuntimeError("fresh-founder mode requires both exact nonempty pinned cohorts")
    return selected


def selection_self_test() -> None:
    selected = fresh_founder_cohort(COHORTS)
    for invalid in ((), tuple(selected[:1]), tuple(selected * 2),
                    (selected[0], (selected[1][0], selected[1][1], "missing-selector", selected[1][3])),
                    (selected[0], (*selected[1][:3], 0))):
        try:
            fresh_founder_cohort(invalid)
        except RuntimeError:
            continue
        raise AssertionError("an empty, duplicated or changed fresh cohort was accepted")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--fresh-founder-only", action="store_true",
                        help="Run only fresh-founder library regressions; not native acceptance.")
    args = parser.parse_args()
    # Reuse the existing bounded process owner and rejection tests. run_path does not
    # execute that script's main or its member/predecessor Cargo commands.
    guard = runpy.run_path(str(REPO / ".github/scripts/check-chat-v2-member-save.py"))
    guard["self_test"]()
    selection_self_test()
    inventory_self_test()
    assert len({name for name, *_ in COHORTS}) == len(COHORTS)
    assert all(expected > 0 for *_, expected in COHORTS)
    if args.self_test:
        return
    directory = REPO / "logs/chat-v2-joined-source"
    directory.mkdir(parents=True, exist_ok=True)
    cohorts = fresh_founder_cohort(COHORTS) if args.fresh_founder_only else list(COHORTS)
    if sys.platform == "win32" and not args.fresh_founder_only:
        cohorts.append(("negative-retirement-bound", "catcoms-app", "negative_retirement_reserves_the_last_slot", 1))
    if sys.platform == "linux":
        # Actual Linux reader and role checks must run before accepting its now
        # enabled positive whole warm factory path. Never accept a zero selector.
        cohorts.extend([
            ("linux-fixed-inventory", "catcoms-app", "exact_inventory_linux::tests", 12),
            ("linux-genesis-inventory", "catcoms-app", "genesis_inventory::tests", 4),
            ("linux-initializing-inventory", "catcoms-app", "actual_initializing_inventory_requires_", 1),
            ("linux-pre-initializing-inventory", "catcoms-app", "actual_pre_initializing_inventory_never_", 1),
            ("linux-seed-settlement", "catcoms-app", "genesis_seed::current::tests::seed_settlement", 8),
            ("linux-covering-scope", "catcoms-app", "genesis_seed::current::tests::settlement_scope", 3),
            ("linux-promotion-recovery", "catcoms-app", "genesis_seed::current::tests::linux_inventory", 2),
        ])
    for name, crate, selector, expected in cohorts:
        command = [
            "cargo", "test", "--locked", "-p", crate, "--all-features", "--lib", selector,
        ]
        if name == "native-registry-physical":
            check_normal_inventory(guard, directory, command, expected)
            command += ["--", *(value for name in FIXTURE_TESTS for value in ("--skip", name))]
        code, summaries, timed_out, _ = guard["run_logged"](
            name, command, directory, limit=30 * 60,
        )
        if timed_out:
            raise RuntimeError(f"{name}: owned cohort exceeded its deadline")
        guard["validate"]("".join(summaries), code, expected)


if __name__ == "__main__":
    main()
