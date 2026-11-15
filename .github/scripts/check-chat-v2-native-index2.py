"""Actual native1-aware predecessor compatibility, never a simulated support flag.

Runs after the serialized head cohorts. Strict lint/advisory gates are unchanged.
Every marker/count must occur exactly once; --self-test invokes no Cargo.
"""
from pathlib import Path
import json
import os
import re
import runpy
import shutil
import sys
import tempfile

REPO = Path(__file__).resolve().parents[2]
BASE = runpy.run_path(str(REPO / ".github/scripts/check-chat-v2-member-save.py"))
GIT = runpy.run_path(str(REPO / ".github/scripts/check-chat-v2-native-registry.py"))["git"]
PREDECESSOR = "382a30a047426228be82f4bb35fb7a87a5ea2e24"
TARGET = "crates/catcoms-app/tests/native_index2_gate.rs"
PREFIX = "store::critical_namespaces::native2_predecessor_tests::"
WRITTEN = re.compile(r"native2 predecessor fixtures: (\d+) written")
CHECKED = re.compile(r"native2 predecessor gate: (\d+) fixtures x (\d+) openers checked")
VERIFIED = re.compile(r"native2 predecessor round trip: (\d+) verified")


def self_test() -> None:
    for pattern, good, expected in (
        (WRITTEN, "native2 predecessor fixtures: 18 written", ("18",)),
        (CHECKED, "native2 predecessor gate: 18 fixtures x 4 openers checked", ("18", "4")),
        (VERIFIED, "native2 predecessor round trip: 18 verified", ("18",)),
    ):
        groups = lambda text: [match.groups() for match in pattern.finditer(text)]
        BASE["require_marker"](groups(good), expected)
        for bad in ([], groups(good + "\n" + good), groups(good.replace("18", "17"))):
            try:
                BASE["require_marker"](bad, expected)
            except RuntimeError:
                continue
            raise AssertionError("missing, duplicate or wrong native2 marker accepted")
    print("Native2 exact-marker self-checks passed; Cargo was not invoked.")


def run() -> None:
    directory = REPO / "logs/chat-v2-native-index2"
    directory.mkdir(parents=True, exist_ok=True)
    results = []
    with tempfile.TemporaryDirectory(prefix="n2-", dir=os.environ.get("RUNNER_TEMP")) as temporary:
        owned = Path(temporary).resolve()
        env = dict(os.environ, MEWTUAL_NATIVE2_FIXTURES=str(owned / "fixtures"),
                   CARGO_TARGET_DIR=os.environ.get("CARGO_TARGET_DIR", str(REPO / "target")))
        try:
            BASE["ignored_head_case"]("native2-fixtures", PREFIX + "generate_native2_fixtures",
                                      directory, env, WRITTEN, ("18",), results)
            worktree = owned / "p"
            if not worktree.resolve().is_relative_to(owned):
                raise RuntimeError("predecessor escaped the owned temporary directory")
            # Fetch the full pinned object closure even when its commit object
            # exists. A local full repository must not acquire a new shallow
            # boundary as a side effect of this test; hosted checkout is shallow.
            fetch = ["fetch", "--no-tags"]
            if GIT("rev-parse", "--is-shallow-repository") == "true":
                fetch.append("--depth=1")
            GIT(*fetch, "origin", PREDECESSOR)
            GIT("worktree", "add", "--detach", str(worktree), PREDECESSOR)
            try:
                if GIT("rev-parse", "HEAD", cwd=worktree) != PREDECESSOR:
                    raise RuntimeError("native2 predecessor is not the exact pinned commit")
                target = worktree / TARGET
                if target.exists():
                    raise RuntimeError("predecessor already has the harness")
                shutil.copyfile(REPO / ".github/predecessor/native_index2_gate.rs", target)
                if GIT("status", "--porcelain", "--untracked-files=all", cwd=worktree) != f"?? {TARGET}":
                    raise RuntimeError("predecessor changed beyond its single integration test")
                code, summaries, timed_out, markers = BASE["run_logged"](
                    "native2-predecessor", ["cargo", "test", "--locked", "-p", "catcoms-app", "--all-features",
                     "--test", "native_index2_gate", "--", "--nocapture", "--test-threads=1"],
                    directory, cwd=worktree, env=env, limit=BASE["PREDECESSOR_TIMEOUT_SECONDS"], markers=(CHECKED,))
                results.append({"name": "native2-predecessor", "commit": PREDECESSOR, "exit": code, "timed_out": timed_out})
                if timed_out:
                    raise RuntimeError("native2 predecessor exceeded its owned-process deadline")
                BASE["validate"]("".join(summaries), code, 1)
                BASE["require_marker"](markers[0], ("18", "4"))
                if GIT("status", "--porcelain", "--untracked-files=all", cwd=worktree) != f"?? {TARGET}":
                    raise RuntimeError("predecessor source changed during its test")
            finally:
                GIT("worktree", "remove", "--force", str(worktree))
            BASE["ignored_head_case"]("native2-round-trip", PREFIX + "verify_native2_predecessor_fixtures",
                                      directory, env, VERIFIED, ("18",), results)
        finally:
            (directory / "results.json").write_text(json.dumps(results, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    if sys.argv[1:] == ["--self-test"]:
        self_test()
    elif sys.argv[1:]:
        raise SystemExit("only --self-test is supported")
    else:
        run()
