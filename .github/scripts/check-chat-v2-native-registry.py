"""Real ID3/ID5-aware predecessor gate, after the workflow's serialized head cohorts.

No predecessor production source is modified. Existing exact-count and owned-process
guards are reused; this is not a second full suite or a lint/advisory waiver.
"""
from pathlib import Path
import json
import os
import re
import runpy
import shutil
import subprocess
import sys
import tempfile

REPO = Path(__file__).resolve().parents[2]
BASE = runpy.run_path(str(REPO / ".github/scripts/check-chat-v2-member-save.py"))
PREDECESSOR = "a607123afbd3ebff2ef8ba10257f274fedef7eb3"
TARGET = "crates/catcoms-app/tests/namespace7_gate.rs"
PREFIX = "store::critical_namespaces::native7_predecessor_tests::"
WRITTEN = re.compile(r"namespace7 predecessor fixtures: (\d+) written")
CHECKED = re.compile(r"namespace7 predecessor gate: (\d+) fixtures x (\d+) openers checked")
KEPT = re.compile(r"namespace7 predecessor round trip: (\d+) written")
VERIFIED = re.compile(r"namespace7 predecessor round trip: (\d+) verified")


def git(*arguments: str, cwd: Path = REPO) -> str:
    return subprocess.run(["git", "-C", str(cwd), "-c", f"safe.directory={cwd}", *arguments],
                          check=True, capture_output=True, text=True).stdout.strip()


def self_test() -> None:
    cases = ((WRITTEN, "namespace7 predecessor fixtures: 16 written", ("16",)),
             (CHECKED, "namespace7 predecessor gate: 16 fixtures x 4 openers checked", ("16", "4")),
             (KEPT, "namespace7 predecessor round trip: 1 written", ("1",)),
             (VERIFIED, "namespace7 predecessor round trip: 1 verified", ("1",)))
    for pattern, line, expected in cases:
        groups = lambda text: [match.groups() for match in pattern.finditer(text)]
        BASE["require_marker"](groups(line), expected)
        changed = line.replace("16", "15") if "16" in line else line.replace("1", "2")
        for bad in ([], groups(line + "\n" + line), groups(changed)):
            try:
                BASE["require_marker"](bad, expected)
            except RuntimeError:
                continue
            raise AssertionError("missing/duplicated marker accepted")
    print("Native7 exact-marker self-checks passed; Cargo was not invoked.")


def run() -> None:
    directory = REPO / "logs/chat-v2-native-registry"
    directory.mkdir(parents=True, exist_ok=True)
    results: list[dict] = []
    # Only a fresh owned temporary directory is ever cleaned. No preexisting path is deleted.
    with tempfile.TemporaryDirectory(prefix="mewtual-native7-", dir=os.environ.get("RUNNER_TEMP")) as temporary:
        owned = Path(temporary).resolve()
        env = dict(os.environ, MEWTUAL_NAMESPACE7_FIXTURES=str(owned / "fixtures"),
                   CARGO_TARGET_DIR=os.environ.get("CARGO_TARGET_DIR", str(REPO / "target")))
        try:
            BASE["ignored_head_case"]("native7-fixtures", PREFIX + "generate_native7_fixtures", directory,
                                      env, WRITTEN, ("16",), results)
            worktree = owned / "predecessor"
            if not worktree.resolve().is_relative_to(owned):
                raise RuntimeError("predecessor path escaped owned temporary directory")
            try:
                git("cat-file", "-e", f"{PREDECESSOR}^{{commit}}")
            except subprocess.CalledProcessError:
                git("fetch", "--no-tags", "--depth=1", "origin", PREDECESSOR)
            git("worktree", "add", "--detach", str(worktree), PREDECESSOR)
            try:
                if git("rev-parse", "HEAD", cwd=worktree) != PREDECESSOR:
                    raise RuntimeError("predecessor is not the exact pinned commit")
                target = worktree / TARGET
                if target.exists():
                    raise RuntimeError("predecessor already has the harness target")
                shutil.copyfile(REPO / ".github/predecessor/namespace7_gate.rs", target)
                if git("status", "--porcelain", "--untracked-files=all", cwd=worktree) != f"?? {TARGET}":
                    raise RuntimeError("predecessor differs beyond its one test harness")
                command = ["cargo", "test", "--locked", "-p", "catcoms-app", "--all-features",
                           "--test", "namespace7_gate", "--", "--nocapture", "--test-threads=1"]
                code, summaries, timed_out, markers = BASE["run_logged"](
                    "native7-predecessor", command, directory, cwd=worktree, env=env,
                    limit=BASE["PREDECESSOR_TIMEOUT_SECONDS"], markers=(CHECKED, KEPT))
                results.append({"name": "native7-predecessor", "commit": PREDECESSOR,
                                "exit": code, "timed_out": timed_out})
                if timed_out:
                    raise RuntimeError("native7 predecessor exceeded its owned-process deadline")
                BASE["validate"]("".join(summaries), code, 1)
                BASE["require_marker"](markers[0], ("16", "4"))
                BASE["require_marker"](markers[1], ("1",))
                if git("status", "--porcelain", "--untracked-files=all", cwd=worktree) != f"?? {TARGET}":
                    raise RuntimeError("predecessor source changed during its gate")
            finally:
                # Exact path within our live temporary directory, after the owned test
                # process has completed/stopped. Never remove another checkout.
                git("worktree", "remove", "--force", str(worktree))
            BASE["ignored_head_case"]("native7-round-trip", PREFIX + "verify_native7_predecessor_claim_recovery", directory,
                                      env, VERIFIED, ("1",), results)
        finally:
            (directory / "results.json").write_text(json.dumps(results, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    if sys.argv[1:] == ["--self-test"]:
        self_test()
    elif sys.argv[1:]:
        raise SystemExit("only --self-test is supported")
    else:
        run()
