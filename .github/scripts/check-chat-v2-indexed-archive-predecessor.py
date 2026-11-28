"""Run the indexed archive gate against the exact ID8-aware predecessor."""

from pathlib import Path
import os
import re
import runpy
import shutil
import subprocess
import sys
import tempfile

REPO = Path(__file__).resolve().parents[2]
BASE = runpy.run_path(str(REPO / ".github/scripts/check-chat-v2-member-save.py"))
PREDECESSOR = "7cfacdf62b2b52a0cacdd5bfd86c6240a91d4db1"
TEST = "crates/catcoms-app/tests/indexed_archive_gate.rs"
CASES = (
    "store::critical_namespaces::native_registry::operational::facade::upgrade::"
    "governance_tests::"
)
WRITTEN = re.compile(r"\bindexed history predecessor fixtures: (\d+) written$")
CHECKED = re.compile(r"\bindexed history predecessor gate: (\d+) fixtures checked$")
RECOVERED = re.compile(r"\bindexed history predecessor fixtures: (\d+) recovered$")


def git(*args, cwd=REPO):
    result = subprocess.run(
        ["git", "-c", f"safe.directory={cwd}", "-c", "core.longpaths=true", *args],
        cwd=cwd, capture_output=True, text=True, timeout=120,
    )
    if result.returncode:
        raise RuntimeError(f"git {args[0]} failed: {result.stderr.strip()}")
    return result.stdout.strip()


def self_test():
    for pattern, line, count in (
        (WRITTEN, "indexed history predecessor fixtures: 4 written", "4"),
        (CHECKED, "indexed history predecessor gate: 4 fixtures checked", "4"),
        (RECOVERED, "indexed history predecessor fixtures: 3 recovered", "3"),
    ):
        def find(output):
            return [match.groups() for row in output.splitlines() if (match := pattern.search(row))]

        BASE["require_marker"](find(f"test exact_case ... {line}\n"), (count,))
        for invalid in ("", line + "\n" + line, line.replace(count, "2")):
            try:
                BASE["require_marker"](find(invalid), (count,))
            except RuntimeError:
                continue
            raise AssertionError("an absent, duplicate or incorrect count was accepted")
    print("Indexed archive predecessor count checks passed.")


def run():
    logs = REPO / "logs/chat-v2-indexed-archive-predecessor"
    logs.mkdir(parents=True, exist_ok=True)
    outcomes = []
    with tempfile.TemporaryDirectory(prefix="idx13-", dir=os.environ.get("RUNNER_TEMP")) as tmp:
        owned = Path(tmp).resolve()
        env = dict(os.environ, MEWTUAL_INDEXED_HISTORY_FIXTURES=str(owned / "fixtures"))
        head_target = str(owned / "head-target")
        head_env = dict(env, CARGO_TARGET_DIR=head_target, CARGO_BUILD_BUILD_DIR=head_target)
        BASE["ignored_head_case"](
            "indexed-fixtures", CASES + "generate_indexed_history_predecessor_fixtures",
            logs, head_env, WRITTEN, ("4",), outcomes,
        )
        # A shallow Actions checkout need not contain the pinned predecessor.
        present = subprocess.run(
            ["git", "-c", f"safe.directory={REPO}", "cat-file", "-e", f"{PREDECESSOR}^{{commit}}"],
            cwd=REPO, capture_output=True, timeout=30,
        )
        if present.returncode:
            git("fetch", "--no-tags", "--depth=1", "origin", PREDECESSOR)
        predecessor = owned / "p"
        if predecessor.resolve().parent != owned or predecessor.exists():
            raise RuntimeError("predecessor checkout path is outside the owned temporary tree")
        git("worktree", "add", "--detach", str(predecessor), PREDECESSOR)
        try:
            if git("rev-parse", "HEAD", cwd=predecessor) != PREDECESSOR:
                raise RuntimeError("predecessor checkout is not at the pinned commit")
            target = predecessor / TEST
            if target.exists() or git("status", "--porcelain", cwd=predecessor):
                raise RuntimeError("predecessor tree was changed before copying the test")
            shutil.copyfile(REPO / ".github/predecessor/indexed_archive_gate.rs", target)
            expected_status = f"?? {TEST}"

            def require_exact_source():
                status = git("status", "--porcelain", "--untracked-files=all", cwd=predecessor)
                if status != expected_status:
                    raise RuntimeError(f"predecessor differs beyond the probe: {status!r}")

            require_exact_source()
            # Cargo artifacts from a different worktree cannot serve as the
            # predecessor executable or any of its linked dependencies.
            old_target = str(owned / "old-target")
            old_env = dict(env, CARGO_TARGET_DIR=old_target, CARGO_BUILD_BUILD_DIR=old_target)
            command = ["cargo", "test", "--locked", "-p", "catcoms-app", "--all-features",
                       "--test", "indexed_archive_gate", "--", "--nocapture", "--test-threads=1"]
            code, summaries, timed_out, markers = BASE["run_logged"](
                "indexed-predecessor", command, logs, cwd=predecessor, env=old_env,
                limit=BASE["PREDECESSOR_TIMEOUT_SECONDS"], markers=(CHECKED,),
            )
            if timed_out:
                raise RuntimeError("indexed archive predecessor exceeded its deadline")
            BASE["validate"]("".join(summaries), code, 1)
            BASE["require_marker"](markers[0], ("4",))
            require_exact_source()
        finally:
            BASE["retire_old_source"](
                lambda: git("worktree", "remove", "--force", str(predecessor)),
                lambda: None,
                outcomes, sys.exception(),
            )
        BASE["ignored_head_case"](
            "indexed-round-trip", CASES + "recover_indexed_history_predecessor_fixtures",
            logs, head_env, RECOVERED, ("3",), outcomes,
        )


if __name__ == "__main__":
    if sys.argv[1:] == ["--self-test"]:
        self_test()
    elif sys.argv[1:]:
        raise SystemExit("only --self-test is supported")
    else:
        run()
