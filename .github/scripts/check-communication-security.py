"""Run real Linux syscall/xattr regressions with exact nonempty result checks.

This is focused compatibility evidence, not a clean audit or complete acceptance.
The shared process owner bounds execution and retains failing diagnostics.
"""
from __future__ import annotations

import argparse
from pathlib import Path
import runpy
import sys

REPO = Path(__file__).resolve().parents[2]
STORAGE = (
    "an_extended_acl_is_never_ruled_out_without_proof",
    "the_explicit_acl_query_observes_real_absence_and_an_extended_acl",
    "unix_removes_in_a_group_writable_folder_only_for_a_user_private_group",
)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("storage", "native", "gossip", "self-test"))
    args = parser.parse_args()
    guard = runpy.run_path(str(REPO / ".github/scripts/check-chat-v2-member-save.py"))
    guard["self_test"]()
    assert len(STORAGE) == len(set(STORAGE)) == 3
    if args.mode == "self-test":
        return
    if sys.platform != "linux" and args.mode != "gossip":
        raise RuntimeError("Linux-only regressions cannot be credited on another platform")
    directory = REPO / "logs/communication-security"
    directory.mkdir(parents=True, exist_ok=True)
    expected = 1
    if args.mode == "gossip":
        commands = [("gossip-resource", ["cargo", "test", "--locked", "-p", "catcoms-net",
                     "--all-features", "--lib", "gossip_resource_tests"])]
        expected = 4
    elif args.mode == "storage":
        commands = [(selector, ["cargo", "test", "--locked", "-p", "catcoms-storage",
                     "--all-features", "--lib", selector]) for selector in STORAGE]
    else:
        commands = [("native-xattr-compat", ["cargo", "test", "--locked", "--manifest-path",
                    "apps/desktop/src-tauri/Cargo.toml", "--test", "xattr_compat"])]
    for name, command in commands:
        code, summaries, timed_out, _ = guard["run_logged"](name, command, directory, limit=30 * 60)
        if timed_out:
            raise RuntimeError(f"{name}: owned regression exceeded its bound")
        guard["validate"]("".join(summaries), code, expected)


if __name__ == "__main__":
    main()
