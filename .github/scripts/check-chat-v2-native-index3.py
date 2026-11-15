"""Actual pre-refresh index3 reader; no modified production or support flag.

The exact 4f45d586 predecessor already understands selected3 and Rename recovery, but refuses a
Refresh Stage. Append a test-only
module to its existing cfg(test) file and prove that is the only changed file.
All original bytes stay intact. Both positive controls and expected refusals
are decided by the harness, not a head-provided acceptance manifest.
"""
from pathlib import Path
import json
import os
import re
import runpy
import sys
import tempfile

REPO = Path(__file__).resolve().parents[2]
BASE = runpy.run_path(str(REPO / ".github/scripts/check-chat-v2-member-save.py"))
GIT = runpy.run_path(str(REPO / ".github/scripts/check-chat-v2-native-registry.py"))["git"]
PREDECESSOR = "4f45d586f6e23f6acc0914fa83d93792544c5bba"
TARGET = "crates/catcoms-app/src/store/critical_namespaces/native_registry/operational/cell/tests.rs"
PREFIX = "store::critical_namespaces::native_registry::lifecycle::cell::tests::compatibility::"
OLD_CASE = "store::critical_namespaces::native_registry::operational::cell::tests::normal3_predecessor::index3_predecessor_gate"
WRITTEN = re.compile(r"native3 predecessor fixtures: (\d+) written")
CHECKED = re.compile(r"native3 predecessor gate: (\d+) fixtures x (\d+) openers checked")
VERIFIED = re.compile(r"native3 predecessor round trip: (\d+) verified")


def self_test() -> None:
    for pattern, good, expected in (
        (WRITTEN, "native3 predecessor fixtures: 11 written", ("11",)),
        (CHECKED, "native3 predecessor gate: 11 fixtures x 3 openers checked", ("11", "3")),
        (VERIFIED, "native3 predecessor round trip: 11 verified", ("11",)),
    ):
        groups = lambda text: [match.groups() for match in pattern.finditer(text)]
        BASE["require_marker"](groups(good), expected)
        for bad in ([], groups(good + "\n" + good), groups(good.replace("11", "8"))):
            try:
                BASE["require_marker"](bad, expected)
            except RuntimeError:
                continue
            raise AssertionError("missing/duplicate/wrong native3 marker accepted")
    print("Native3 exact-marker self-checks passed; Cargo was not invoked.")
    # Cleanup must attempt head restoration even after worktree removal fails,
    # and must never replace the actual predecessor validation exception.
    for remove_fails, restore_fails in ((False, False), (True, False), (False, True), (True, True)):
        for has_primary in (False, True):
            calls, results = [], []
            primary = ValueError("original predecessor failure") if has_primary else None
            def action(name: str, fails: bool) -> None:
                calls.append(name)
                if fails: raise OSError(name)
            caught = None
            try:
                try:
                    if primary is not None: raise primary
                finally:
                    retire_old_source(lambda: action("remove", remove_fails),
                        lambda: action("restore", restore_fails), results, sys.exception())
            except Exception as error:
                caught = error
            assert calls == ["remove", "restore"]
            if primary is not None: assert caught is primary
            else: assert (caught is not None) == (remove_fails or restore_fails)
            assert len(results) == int(remove_fails) + int(restore_fails)
    print("Native3 failure-path artifact cleanup self-checks passed; Cargo was not invoked.")


def clean_app(label: str, worktree: Path, env: dict, directory: Path, results: list) -> None:
    # One serialized queue, one package only. Never remove the target tree,
    # which may contain worktrees. Prevent wrong-source shared-target reuse.
    code, _, timed_out, _ = BASE["run_logged"](
        label, ["cargo", "clean", "-p", "catcoms-app"], directory,
        cwd=worktree, env=env)
    results.append({"name": label, "exit": code, "timed_out": timed_out})
    if code or timed_out:
        raise RuntimeError("could not retire the previous source's App artifacts")


def retire_old_source(remove, restore_head, results: list, primary: BaseException | None) -> None:
    """Attempt BOTH cleanup steps, preserving a primary validation failure.

    Called only after the old owned child returned. Failure to remove a worktree
    must not leave its App artifacts available to subsequent head commands.
    """
    failures = []
    for name, action in (("remove-predecessor", remove), ("restore-head-artifacts", restore_head)):
        try:
            action()
        except Exception as error:
            failures.append(error)
            results.append({"name": name, "cleanup_error": repr(error)})
    if failures:
        if primary is not None:
            for error in failures: primary.add_note(f"Additional predecessor cleanup failure: {error!r}")
        else:
            raise RuntimeError("predecessor cleanup failed; see results.json") from failures[0]


def run() -> None:
    directory = REPO / "logs/chat-v2-native-index3"
    directory.mkdir(parents=True, exist_ok=True)
    results = []
    with tempfile.TemporaryDirectory(prefix="n3-", dir=os.environ.get("RUNNER_TEMP")) as temporary:
        owned = Path(temporary).resolve()
        env = dict(os.environ, MEWTUAL_NATIVE3_FIXTURES=str(owned / "fixtures"),
                   CARGO_TARGET_DIR=os.environ.get("CARGO_TARGET_DIR", str(REPO / "target")))
        try:
            clean_app("native3-clean-head-before", REPO, env, directory, results)
            BASE["ignored_head_case"]("native3-fixtures", PREFIX + "generate_native3_fixtures",
                directory, env, WRITTEN, ("11",), results)
            worktree = owned / "p"
            if not worktree.resolve().is_relative_to(owned):
                raise RuntimeError("predecessor escaped the owned temporary directory")
            fetch = ["fetch", "--no-tags"]
            if GIT("rev-parse", "--is-shallow-repository") == "true": fetch.append("--depth=1")
            GIT(*fetch, "origin", PREDECESSOR)
            GIT("worktree", "add", "--detach", str(worktree), PREDECESSOR)
            try:
                if GIT("rev-parse", "HEAD", cwd=worktree) != PREDECESSOR:
                    raise RuntimeError("wrong predecessor")
                if GIT("status", "--porcelain", "--untracked-files=all", cwd=worktree):
                    raise RuntimeError("predecessor must start pristine")
                target = worktree / TARGET
                original = target.read_bytes()
                harness = (REPO / ".github/predecessor/native_index3_gate.rs").read_bytes()
                target.write_bytes(original + b"\n" + harness)
                def require_test_only_change() -> None:
                    if target.read_bytes() != original + b"\n" + harness or GIT(
                        "status", "--porcelain", "--untracked-files=all", cwd=worktree) != f"M {TARGET}":
                        raise RuntimeError("predecessor changed beyond the appended test module")
                require_test_only_change()
                clean_app("native3-clean-old", worktree, env, directory, results)
                code, summaries, timed_out, markers = BASE["run_logged"](
                    "native3-predecessor", ["cargo", "test", "--locked", "-p", "catcoms-app",
                    "--all-features", "--lib", OLD_CASE, "--", "--exact", "--nocapture", "--test-threads=1"],
                    directory, cwd=worktree, env=env, limit=BASE["PREDECESSOR_TIMEOUT_SECONDS"], markers=(CHECKED,))
                results.append({"name": "native3-predecessor", "commit": PREDECESSOR,
                    "exit": code, "timed_out": timed_out})
                if timed_out: raise RuntimeError("native3 predecessor exceeded its owned-process deadline")
                BASE["validate"]("".join(summaries), code, 1)
                BASE["require_marker"](markers[0], ("11", "3"))
                require_test_only_change()
            finally:
                # Exact checked path in this fresh owned directory, after all
                # child processes returned; never touch another checkout.
                retire_old_source(
                    lambda: GIT("worktree", "remove", "--force", str(worktree)),
                    lambda: clean_app("native3-clean-head-after", REPO, env, directory, results),
                    results, sys.exception())
            BASE["ignored_head_case"]("native3-round-trip", PREFIX + "verify_native3_predecessor_fixtures",
                directory, env, VERIFIED, ("11",), results)
        finally:
            (directory / "results.json").write_text(json.dumps(results, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    if sys.argv[1:] == ["--self-test"]: self_test()
    elif sys.argv[1:]: raise SystemExit("only --self-test is supported")
    else: run()
