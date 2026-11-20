"""Run exact, nonignored selected-history cohorts; --self-test invokes no Cargo.

This lane is Core proof evidence only, not a durable archive/Chat V2 acceptance
claim. Full root, native, frontend, workspace lint and audit gates remain required.
"""
from __future__ import annotations

import argparse
from pathlib import Path
import runpy

REPO = Path(__file__).resolve().parents[2]
COHORTS = (
    ("selected-replay", "v2_general::tests::selected_history", 12),
    ("selected-arithmetic", "v2_general::selected_history::tests", 2),
    ("existing-read-accessors", "v2_general::tests::read_accessors", 3),
)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    guard = runpy.run_path(str(REPO / ".github/scripts/check-chat-v2-member-save.py"))
    guard["self_test"]()
    assert len({name for name, *_ in COHORTS}) == len(COHORTS)
    assert all(expected > 0 for *_, expected in COHORTS)
    if args.self_test:
        return
    directory = REPO / "logs/chat-v2-selected-history"
    directory.mkdir(parents=True, exist_ok=True)
    for name, selector, expected in COHORTS:
        code, summaries, timed_out, _ = guard["run_logged"](
            name,
            ["cargo", "test", "--locked", "-p", "catcoms-replication", "--all-features", "--lib", selector],
            directory, limit=30 * 60,
        )
        if timed_out:
            raise RuntimeError(f"{name}: owned cohort exceeded its deadline")
        guard["validate"]("".join(summaries), code, expected)


if __name__ == "__main__":
    main()
