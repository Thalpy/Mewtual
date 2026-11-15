"""Real Native4 refusal/preservation gate against the last pre-authoring reader.

Only a test module is appended to the exact predecessor. No production code or
head-supplied acceptance manifest changes its behavior. Positive DATA controls
must open; every refused tree must remain byte-identical.
"""
from pathlib import Path
import json
import os
import re
import runpy
import sys
import tempfile

REPO = Path(__file__).resolve().parents[2]
INDEX3 = runpy.run_path(str(REPO / ".github/scripts/check-chat-v2-native-index3.py"))
BASE, GIT = INDEX3["BASE"], INDEX3["GIT"]
PREDECESSOR = "9273dab48cdd973e360a82873c726e842b486b8b"
TARGET = "crates/catcoms-app/src/store/critical_namespaces/native_registry/lifecycle/cell/tests.rs"
PREFIX = "store::critical_namespaces::native_registry::operational::sources::network::tests::compatibility::"
OLD_CASE = "store::critical_namespaces::native_registry::lifecycle::cell::tests::native4_predecessor::native4_predecessor_gate"
WRITTEN = re.compile(r"native4 predecessor fixtures: (\d+) written")
CHECKED = re.compile(r"native4 predecessor gate: (\d+) fixtures x (\d+) openers checked")
VERIFIED = re.compile(r"native4 predecessor round trip: (\d+) verified")
CASES = frozenset(("selected3", "data-pending", "data-active", "native-pending", "native-stage", "native-active"))


def require_fixture_inventory(root: Path) -> None:
    entries = list(root.iterdir())
    if ({entry.name for entry in entries} != CASES
            or any(entry.is_symlink() or not entry.is_dir() for entry in entries)):
        raise RuntimeError("Native4 fixture root differs from the exact six real directories")


def self_test() -> None:
    with tempfile.TemporaryDirectory(prefix="native4-inventory-") as temporary:
        root = Path(temporary)
        for name in CASES: (root / name).mkdir()
        require_fixture_inventory(root)
        # A silently unprobed seventh fixture and a regular-file fixture must
        # both fail, even if the literal success/count markers remain correct.
        for extra_is_dir in (False, True):
            extra = root / "unprobed"
            if extra_is_dir: extra.mkdir()
            else: extra.write_bytes(b"")
            try:
                require_fixture_inventory(root)
            except RuntimeError:
                pass
            else:
                raise AssertionError("an unprobed extra fixture was accepted")
            if extra_is_dir: extra.rmdir()
            else: extra.unlink()
        (root / "native-active").rmdir()
        (root / "native-active").write_bytes(b"")
        try:
            require_fixture_inventory(root)
        except RuntimeError:
            pass
        else:
            raise AssertionError("regular file was accepted as a fixture directory")
    for pattern, good, expected in (
        (WRITTEN, "native4 predecessor fixtures: 6 written", ("6",)),
        (CHECKED, "native4 predecessor gate: 6 fixtures x 3 openers checked", ("6", "3")),
        (VERIFIED, "native4 predecessor round trip: 6 verified", ("6",)),
    ):
        groups = lambda text: [match.groups() for match in pattern.finditer(text)]
        BASE["require_marker"](groups(good), expected)
        for bad in ([], groups(good + "\n" + good), groups(good.replace("6", "5"))):
            try:
                BASE["require_marker"](bad, expected)
            except RuntimeError:
                continue
            raise AssertionError("missing/duplicate/wrong Native4 marker accepted")
    INDEX3["self_test"]()
    print("Native4 exact-marker self-checks passed; Cargo was not invoked.")


def run() -> None:
    directory = REPO / "logs/chat-v2-native-founder"
    directory.mkdir(parents=True, exist_ok=True)
    results = []
    with tempfile.TemporaryDirectory(prefix="n4-", dir=os.environ.get("RUNNER_TEMP")) as temporary:
        owned = Path(temporary).resolve()
        env = dict(os.environ, MEWTUAL_NATIVE4_FIXTURES=str(owned / "fixtures"),
                   CARGO_TARGET_DIR=os.environ.get("CARGO_TARGET_DIR", str(REPO / "target")))
        try:
            INDEX3["clean_app"]("native4-clean-head-before", REPO, env, directory, results)
            BASE["ignored_head_case"]("native4-fixtures", PREFIX + "generate_native4_fixtures",
                directory, env, WRITTEN, ("6",), results)
            require_fixture_inventory(owned / "fixtures")
            worktree = owned / "p"
            if not worktree.resolve().is_relative_to(owned):
                raise RuntimeError("predecessor escaped owned temporary directory")
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
                harness = (REPO / ".github/predecessor/native_founder_gate.rs").read_bytes()
                target.write_bytes(original + b"\n" + harness)
                def require_test_only_change() -> None:
                    if target.read_bytes() != original + b"\n" + harness or GIT(
                        "status", "--porcelain", "--untracked-files=all", cwd=worktree) != f"M {TARGET}":
                        raise RuntimeError("predecessor changed beyond appended test module")
                require_test_only_change()
                INDEX3["clean_app"]("native4-clean-old", worktree, env, directory, results)
                code, summaries, timed_out, markers = BASE["run_logged"](
                    "native4-predecessor", ["cargo", "test", "--locked", "-p", "catcoms-app",
                    "--all-features", "--lib", OLD_CASE, "--", "--exact", "--nocapture", "--test-threads=1"],
                    directory, cwd=worktree, env=env, limit=BASE["PREDECESSOR_TIMEOUT_SECONDS"], markers=(CHECKED,))
                results.append({"name": "native4-predecessor", "commit": PREDECESSOR,
                    "exit": code, "timed_out": timed_out})
                if timed_out: raise RuntimeError("Native4 predecessor exceeded owned-process deadline")
                BASE["validate"]("".join(summaries), code, 1)
                BASE["require_marker"](markers[0], ("6", "3"))
                require_fixture_inventory(owned / "fixtures")
                require_test_only_change()
            finally:
                INDEX3["retire_old_source"](
                    lambda: GIT("worktree", "remove", "--force", str(worktree)),
                    lambda: INDEX3["clean_app"]("native4-clean-head-after", REPO, env, directory, results),
                    results, sys.exception())
            BASE["ignored_head_case"]("native4-round-trip", PREFIX + "verify_native4_predecessor_fixtures",
                directory, env, VERIFIED, ("6",), results)
        finally:
            (directory / "results.json").write_text(json.dumps(results, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    if sys.argv[1:] == ["--self-test"]: self_test()
    elif sys.argv[1:]: raise SystemExit("only --self-test is supported")
    else: run()
