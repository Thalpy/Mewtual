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
    ("founder-member", "catcoms-sync", "owner_v2::member::tests", 6),
    ("bounded-local-recovery", "catcoms-replication", "doc::saved_local_recovery::tests", 6),
    ("existing-local-recovery", "catcoms-replication", "doc::saved_local_chat_tests", 5),
    ("selected-history-ordering", "catcoms-replication", "v2_general::tests::selected_history", 12),
)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    # Reuse the existing bounded process owner and rejection tests. run_path does not
    # execute that script's main or its member/predecessor Cargo commands.
    guard = runpy.run_path(str(REPO / ".github/scripts/check-chat-v2-member-save.py"))
    guard["self_test"]()
    assert len({name for name, *_ in COHORTS}) == len(COHORTS)
    assert all(expected > 0 for *_, expected in COHORTS)
    if args.self_test:
        return
    directory = REPO / "logs/chat-v2-joined-source"
    directory.mkdir(parents=True, exist_ok=True)
    cohorts = list(COHORTS)
    if sys.platform == "win32":
        cohorts.append(("negative-retirement-bound", "catcoms-app", "negative_retirement_reserves_the_last_slot", 1))
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
