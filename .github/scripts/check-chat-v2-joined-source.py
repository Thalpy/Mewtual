"""Execute nonempty, exact-count joined-source cohorts independently of strict CI.

This is focused evidence only. It neither enables Chat V2 nor substitutes for full
root/native/frontend, strict-lint or dependency checks. --self-test runs no Cargo.
"""
from __future__ import annotations

import argparse
from pathlib import Path
import runpy
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
    ("native-key-lifecycle", "catcoms-rt", "native_chat_key::", 25),
    ("native-warm-sdk-tail", "catcoms-sync", "native_warm_sdk_tail_", 1),
    ("native-founder-sdk", "catcoms-app", "native_sdk_", 5),
    ("native-network-identity", "catcoms-net", "seeded_transport_identity_data", 3),
    ("bounded-local-recovery", "catcoms-replication", "doc::saved_local_recovery::tests", 6),
    ("existing-local-recovery", "catcoms-replication", "doc::saved_local_chat_tests", 5),
    ("selected-history-ordering", "catcoms-replication", "v2_general::tests::selected_history", 12),
    ("fresh-founder-mount", "catcoms-app", "current_layout::tests::fresh_founder", 1),
    ("fresh-founder-driver", "catcoms-app", "founder::plan::source::warm::tests", 11 if sys.platform in ("win32", "linux") else 4),
    # Includes private lifecycle3 DATA cases; no physical3 opener is implied.
    ("native-registry-physical", "catcoms-app", "critical_namespaces::native_registry::", 133),
    ("native-registry-directory", "catcoms-app", "native_registry_directory_capture", 1),
    ("native-registry-join-refusal", "catcoms-app", "native_registry_preparation_refuses_real", 1),
    ("native-registry-join-lifecycle", "catcoms-app", "native_registry_actual_join_lifecycle", 1),
)


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
        code, summaries, timed_out, _ = guard["run_logged"](
            name, command, directory, limit=30 * 60,
        )
        if timed_out:
            raise RuntimeError(f"{name}: owned cohort exceeded its deadline")
        guard["validate"]("".join(summaries), code, expected)


if __name__ == "__main__":
    main()
