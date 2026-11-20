"""Hosted invitation cohorts with exact nonignored counts and retained logs."""
from pathlib import Path
import runpy
import sys

COHORTS = {
    "bindings": ("catcoms-replication", "chat::join_bindings::tests::", 15),
    "journal": ("catcoms-app", "unsigned_invite_journal_", 4),
    "native": ("catcoms-app", "native_runtime_invite_", 15),
    "packet": ("catcoms-app", "native_invite_packet_", 3),
    "join-request": ("catcoms-sync", "prepared_direct_join::v2::tests::", 3),
    "join-welcome": ("catcoms-sync", "v2_join_request_and_welcome_complete_with_actual_add", 1),
    "predecessor": (
        "catcoms-app",
        "invite_journal_survives_predecessor_save_and_physical_retirement",
        1,
    ),
}


def main() -> None:
    if len(sys.argv) != 2 or sys.argv[1] not in COHORTS:
        raise SystemExit("choose " + ", ".join(COHORTS))
    name = sys.argv[1]
    package, selector, expected = COHORTS[name]
    guard = runpy.run_path(str(Path(__file__).with_name("check-chat-v2-member-save.py")))
    directory = Path("logs/chat-v2-invite-intent")
    directory.mkdir(parents=True, exist_ok=True)
    code, summaries, timed_out, _ = guard["run_logged"](
        name,
        ["cargo", "test", "--locked", "-p", package, "--lib", selector, "--", "--test-threads=1"],
        directory,
        limit=60 * 60,
    )
    if timed_out:
        raise RuntimeError(f"{name} exceeded its hosted compile/execution bound")
    guard["validate"]("".join(summaries), code, expected)


if __name__ == "__main__":
    main()
