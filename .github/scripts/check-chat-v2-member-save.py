"""Run exact-count member-storage cohorts without substituting for strict/full CI.

The save host is Windows-only today. Do not run its filter on Linux and credit a
zero-test success. Storage/catalog cohorts execute on both operating systems.

After the cohorts, the namespace5 predecessor gate runs on both operating systems:
1. the head writes fixture vaults (an ignored in-crate generator);
2. the UNMODIFIED predecessor commit is checked out in a git worktree, the head's
   harness file is copied in as its only addition (proven by `git status`), and that
   harness drives the predecessor's public openers against the fixtures, keeping the
   trees its full runs produce from the may-open fixtures;
3. the head reopens those trees (an ignored in-crate verifier) and must recover.
Fixture, opener and round-trip counts are pinned in the head tests, the harness and here.
"""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import tomllib


REPO = Path(__file__).resolve().parents[2]
ANSI = re.compile(r"\x1b\[[0-9;]*m")
SUMMARY = re.compile(
    r"test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;"
)
COHORT_TIMEOUT_SECONDS = 15 * 60
# The predecessor build compiles every workspace crate again from its own sources; only
# registry crates can be shared through the target directory (the lockfile is unchanged).
PREDECESSOR_TIMEOUT_SECONDS = 40 * 60

# Keep expected counts explicit: renamed/empty filters and unexpected ignored cases
# fail instead of quietly shrinking the evidence. Changes require reviewing the cases.
COHORTS = (
    ("sorted-declarations", "critical_namespaces::catalog_declaration", 4),
    ("namespace-codec", "critical_namespaces::member_outbox_namespace_codec", 6),
    ("join-producer", "critical_namespaces::pending_join::producer::member_outbox_tests", 4),
    # This module also owns the genuine index2 JOIN and both lifecycle3 JOIN
    # regressions. Keep all22 in this broad cohort; do not trim it to the old19.
    ("current-mount", "critical_namespaces::current_layout::tests::member_outbox", 22),
    ("governance-mount", "critical_namespaces::governance_layout::tests::member_outbox", 8),
    ("outbox-engine", "v2_member_outbox", 38),
)

# The last commit before namespace5. Its source is never modified by this gate. The annotated
# remote tag compat/chat-v2-namespace5-predecessor-ce402364 retains this exact commit independently
# of PR history. The gate still pins/asserts the full SHA rather than trusting a movable tag.
# An unreachable commit fails the fetch loudly; it never skips the gate.
PREDECESSOR = "ce4023649b1a2f37f7d8032c950fbefff03daef3"
PREDECESSOR_FIXTURES = 23
PREDECESSOR_OPENERS = 10
# May-open fixtures x {Current3, governance} full predecessor runs, reopened by the head.
ROUND_TRIP = 4
FIXTURE_TESTS = "store::critical_namespaces::current_layout::tests::predecessor_fixtures::"
GENERATOR = FIXTURE_TESTS + "generate_namespace5_predecessor_fixtures"
VERIFIER = FIXTURE_TESTS + "verify_namespace5_predecessor_round_trip"
HARNESS = REPO / ".github" / "predecessor" / "namespace5_gate.rs"
HARNESS_TARGET = "crates/catcoms-app/tests/namespace5_gate.rs"
WRITTEN = re.compile(r"namespace5 predecessor fixtures: (\d+) written")
CHECKED = re.compile(r"namespace5 predecessor gate: (\d+) fixtures x (\d+) openers checked")
KEPT = re.compile(r"namespace5 predecessor round trip: (\d+) written")
VERIFIED = re.compile(r"namespace5 predecessor round trip: (\d+) verified")


def validate(output: str, exit_code: int, expected: int) -> None:
    summaries = SUMMARY.findall(ANSI.sub("", output))
    if exit_code != 0 or summaries != [(str(expected), "0", "0")] or expected <= 0:
        raise RuntimeError(
            f"expected exactly {expected} passing, nonignored tests; "
            f"exit={exit_code}, summaries={summaries!r}"
        )


def require_marker(markers: list[tuple[str, ...]], expected: tuple[str, ...]) -> None:
    """Exactly one count line with exactly the pinned values."""
    if markers != [expected]:
        raise RuntimeError(f"expected one count line {expected!r}; got {markers!r}")


def self_test() -> None:
    good = "test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured;"
    validate(good, 0, 2)
    validate(good.replace("ok", "\x1b[32mok\x1b[0m"), 0, 2)
    rejected = (
        ("", 0, 2),
        (good.replace("2 passed", "0 passed"), 0, 2),
        (good, 101, 2),
        (good.replace("0 ignored", "1 ignored"), 0, 2),
        (good + "\n" + good, 0, 2),
        (good, 0, 1),
        (good.replace("2 passed", "0 passed"), 0, 0),
    )
    for output, code, count in rejected:
        try:
            validate(output, code, count)
        except RuntimeError:
            continue
        raise AssertionError("an empty, failed, ignored or mismatched cohort was accepted")
    lines = (
        (WRITTEN, f"namespace5 predecessor fixtures: {PREDECESSOR_FIXTURES} written",
         (str(PREDECESSOR_FIXTURES),)),
        (CHECKED, f"namespace5 predecessor gate: {PREDECESSOR_FIXTURES} fixtures x "
         f"{PREDECESSOR_OPENERS} openers checked: {{}}",
         (str(PREDECESSOR_FIXTURES), str(PREDECESSOR_OPENERS))),
        (KEPT, f"namespace5 predecessor round trip: {ROUND_TRIP} written", (str(ROUND_TRIP),)),
        (VERIFIED, f"namespace5 predecessor round trip: {ROUND_TRIP} verified",
         (str(ROUND_TRIP),)),
    )
    for pattern, line, expected in lines:
        found = pattern.search(line)
        assert found is not None, line
        require_marker([found.groups()], expected)
    # The "written" and "verified" round-trip lines must never satisfy each other.
    assert KEPT.search(f"namespace5 predecessor round trip: {ROUND_TRIP} verified") is None
    assert VERIFIED.search(f"namespace5 predecessor round trip: {ROUND_TRIP} written") is None
    for markers in ([], [("20",)], [("21",), ("21",)]):
        try:
            require_marker(markers, ("21",))
        except RuntimeError:
            continue
        raise AssertionError("a missing, changed or repeated count was accepted")
    source_retirement_self_test()
    print("Exact-count/source-retirement guard self-checks passed; Cargo was not invoked.")


def stop_owned_cohort(
    process: subprocess.Popen[str], timed_out: threading.Event, limit: int,
) -> None:
    """Bound deadlock regressions by stopping only this still-owned child tree."""
    if process.poll() is not None:
        return
    timed_out.set()
    print(f"Step exceeded {limit}s; stopping its child tree.", flush=True)
    if sys.platform == "win32":
        subprocess.run(
            ["taskkill.exe", "/PID", str(process.pid), "/T", "/F"], check=False, timeout=30,
        )
    else:
        # Each step starts a fresh session, so this group contains only its Cargo/test
        # children. Killing the group also releases an inherited stdout pipe on deadlock.
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass


def run_logged(
    name: str,
    command: list[str],
    directory: Path,
    *,
    cwd: Path = REPO,
    env: dict[str, str] | None = None,
    limit: int = COHORT_TIMEOUT_SECONDS,
    markers: tuple[re.Pattern[str], ...] = (),
    marker_limits: tuple[int, ...] | None = None,
) -> tuple[int, list[str], bool, list[list[tuple[str, ...]]]]:
    """Stream one owned subprocess to its artifact log under a deadline.

    Returns the exit code, at most two test summaries, whether the deadline fired,
    and bounded marker matches. Ordinary markers retain at most two: a duplicate
    already fails the exact-one guards. Explicit test-name inventories may request
    a larger fixed cap (including one overflow sentinel), with per-pattern and
    aggregate limits checked before spawning. Build logs are never buffered.
    """
    limits = marker_limits if marker_limits is not None else (2,) * len(markers)
    if (len(limits) != len(markers) or any(type(n) is not int or not 1 <= n <= 400 for n in limits)
            or sum(limits) > 1024):
        raise RuntimeError("invalid bounded marker inventory")
    print(f"\n{name}: {' '.join(command)}", flush=True)
    summaries: list[str] = []
    found: list[list[tuple[str, ...]]] = [[] for _ in markers]
    with (directory / f"{name}.log").open("w", encoding="utf-8") as log:
        with subprocess.Popen(
            command, cwd=cwd, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
            text=True, encoding="utf-8", errors="replace",
            start_new_session=sys.platform != "win32",
        ) as process:
            timed_out = threading.Event()
            deadline = threading.Timer(
                limit, stop_owned_cohort, args=(process, timed_out, limit),
            )
            deadline.start()
            try:
                assert process.stdout is not None
                for line in process.stdout:
                    log.write(line)
                    log.flush()
                    print(line, end="", flush=True)
                    plain = ANSI.sub("", line)
                    if len(summaries) < 2 and SUMMARY.search(plain):
                        summaries.append(line)
                    for index, pattern in enumerate(markers):
                        match = pattern.search(plain)
                        if match is not None and len(found[index]) < limits[index]:
                            found[index].append(match.groups())
                code = process.wait()
            finally:
                deadline.cancel()
                deadline.join()
    return code, summaries, timed_out.is_set(), found


def git(*arguments: str, cwd: Path = REPO) -> str:
    return subprocess.run(
        ["git", *arguments], cwd=cwd, check=True, capture_output=True, text=True,
    ).stdout.strip()


def local_source_packages(worktree: Path) -> tuple[str, ...]:
    """Exact bounded path-package IDs, including local patches, for this source.

    Cargo can retain a newer-mtime artifact from another checkout with the same
    name/version. Cleaning only App leaves RT/SDK metadata from the other source
    and can even turn a predecessor test into a mixed-source binary. Registry
    artifacts remain reusable; no target tree is recursively removed.
    """
    root = worktree.resolve()
    manifest = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))
    members = manifest["workspace"]["members"]
    patches = [entry["path"] for table in manifest.get("patch", {}).values()
               for entry in table.values() if "path" in entry]
    paths = [*members, *patches]
    if not 1 <= len(members) <= 32 or len(patches) > 8 or len(set(paths)) != len(paths):
        raise RuntimeError("unexpected local source package inventory")
    ids = set()
    for relative in paths:
        if not isinstance(relative, str) or any(c in relative for c in "*?["):
            raise RuntimeError("local package path is not exact")
        source = (root / relative).resolve()
        if not source.is_relative_to(root):
            raise RuntimeError("local package escaped its source worktree")
        package = tomllib.loads((source / "Cargo.toml").read_text(encoding="utf-8"))["package"]
        name, version = package["name"], package["version"]
        if (not isinstance(name, str) or not isinstance(version, str)
                or not re.fullmatch(r"[A-Za-z0-9_-]{1,128}", name)
                or not re.fullmatch(r"[0-9][A-Za-z0-9.+-]{0,127}", version)
                or (name, version) in ids):
            raise RuntimeError("invalid or duplicate local package identity")
        ids.add((name, version))
    lock = tomllib.loads((root / "Cargo.lock").read_text(encoding="utf-8"))
    locked = {(p["name"], p["version"]) for p in lock["package"] if "source" not in p}
    if locked != ids:
        raise RuntimeError("source-less lock packages differ from workspace/local patch manifests")
    return tuple(f"{name}@{version}" for name, version in sorted(ids))


def source_clean_command(worktree: Path, env: dict) -> list[str]:
    """Build a package-only command; never permit Cargo's unscoped target clean."""
    target = Path(env.get("CARGO_TARGET_DIR", str(REPO / "target")))
    if not target.is_absolute():
        raise RuntimeError("source transitions require one absolute Cargo target")
    packages = local_source_packages(worktree)
    return ["cargo", "clean", "--target-dir", str(target),
            *(arg for package in packages for arg in ("-p", package))]


def clean_source(label: str, worktree: Path, env: dict, directory: Path, results: list) -> None:
    """Retire this source's path artifacts in the ONE serialized absolute target."""
    command = source_clean_command(worktree, env)
    code, _, timed_out, _ = run_logged(label, command, directory, cwd=worktree, env=env)
    results.append({"name": label, "packages": command[5::2], "exit": code, "timed_out": timed_out})
    if code or timed_out:
        raise RuntimeError("could not retire the previous source's local artifacts")


def retire_old_source(remove, restore_head, results: list, primary: BaseException | None) -> None:
    """Always attempt both cleanup steps without replacing a validation failure."""
    failures = []
    for name, action in (("remove-predecessor", remove), ("restore-head-artifacts", restore_head)):
        try:
            action()
        except Exception as error:
            failures.append(error)
            results.append({"name": name, "cleanup_error": repr(error)})
    if failures:
        if primary is not None:
            for error in failures:
                primary.add_note(f"Additional predecessor cleanup failure: {error!r}")
        else:
            raise RuntimeError("predecessor cleanup failed; see results.json") from failures[0]


def source_retirement_self_test() -> None:
    """Synthetic manifests/process output only: this must never invoke Cargo."""
    with tempfile.TemporaryDirectory(prefix="source-guard-") as temporary:
        root = Path(temporary).resolve()
        manifest = '[workspace]\nmembers = ["crates/a"]\n[patch.crates-io]\np = { path = "vendor/p" }\n'
        (root / "Cargo.toml").write_text(manifest, encoding="utf-8")
        for folder, name in (("crates/a", "a"), ("vendor/p", "p")):
            path = root / folder
            path.mkdir(parents=True)
            (path / "Cargo.toml").write_text(f'[package]\nname = "{name}"\nversion = "1.2.3"\n', encoding="utf-8")
        lock = '[[package]]\nname = "a"\nversion = "1.2.3"\n[[package]]\nname = "p"\nversion = "1.2.3"\n'
        (root / "Cargo.lock").write_text(lock, encoding="utf-8")
        target = str(root / "shared-target")
        assert source_clean_command(root, {"CARGO_TARGET_DIR": target}) == [
            "cargo", "clean", "--target-dir", target, "-p", "a@1.2.3", "-p", "p@1.2.3"]
        for changed in (lock.replace('name = "p"', 'name = "unexpected"'),
                        lock + '[[package]]\nname = "extra"\nversion = "1.0.0"\n'):
            (root / "Cargo.lock").write_text(changed, encoding="utf-8")
            try:
                local_source_packages(root)
            except RuntimeError:
                pass
            else:
                raise AssertionError("incomplete local-source retirement accepted")
        (root / "Cargo.lock").write_text(lock, encoding="utf-8")
        for changed in (manifest.replace('vendor/p', '../../outside'), manifest.replace('crates/a', 'crates/*')):
            (root / "Cargo.toml").write_text(changed, encoding="utf-8")
            try:
                local_source_packages(root)
            except RuntimeError:
                pass
            else:
                raise AssertionError("nonlocal or nonexact package path accepted")
        (root / "Cargo.toml").write_text(manifest, encoding="utf-8")
        try:
            source_clean_command(root, {"CARGO_TARGET_DIR": "relative-target"})
        except RuntimeError:
            pass
        else:
            raise AssertionError("relative source-transition target accepted")
        # A larger explicit inventory stays bounded; ordinary count markers keep
        # their original two-match cap. Extra names are retained as rejection evidence.
        command = [sys.executable, "-c", "for i in range(8): print('name' + str(i))"]
        pattern = re.compile(r"^(name\d+)$")
        for cap in (2, 5):
            code, _, timed_out, found = run_logged("bounded-names-" + str(cap), command, root,
                markers=(pattern,), marker_limits=(cap,))
            assert not code and not timed_out and len(found[0]) == cap
    # Restore is attempted even when removal fails, and a primary validation
    # exception remains the exception observed by CI.
    for remove_fails, restore_fails in ((False, False), (True, False), (False, True), (True, True)):
        for primary in (None, ValueError("original validation")):
            calls, results = [], []
            def action(name, fails):
                calls.append(name)
                if fails:
                    raise OSError(name)
            caught = None
            try:
                try:
                    if primary is not None:
                        raise primary
                finally:
                    retire_old_source(lambda: action("remove", remove_fails),
                        lambda: action("restore", restore_fails), results, sys.exception())
            except Exception as error:
                caught = error
            assert calls == ["remove", "restore"]
            assert (caught is primary) if primary is not None else ((caught is not None) == (remove_fails or restore_fails))
            assert len(results) == int(remove_fails) + int(restore_fails)


def ignored_head_case(
    name: str, case: str, directory: Path, env: dict[str, str], marker: re.Pattern[str],
    expected: tuple[str, ...], results: list[dict],
) -> None:
    """One exact ignored in-crate case at the head, with its pinned count line."""
    command = [
        "cargo", "test", "--locked", "-p", "catcoms-app", "--all-features", "--lib",
        case, "--", "--ignored", "--exact", "--nocapture",
    ]
    code, summaries, timed_out, found = run_logged(
        name, command, directory, env=env, markers=(marker,),
    )
    result = {"name": name, "case": case, "exit": code, "timed_out": timed_out}
    results.append(result)
    if timed_out:
        raise RuntimeError(f"{name}: deadline exceeded")
    validate("".join(summaries), code, 1)
    require_marker(found[0], expected)
    result["count"] = expected[0]


def predecessor_gate(directory: Path, results: list[dict]) -> None:
    """Head fixtures, the unmodified predecessor's public openers, then the head round trip."""
    fixtures = directory / "namespace5-fixtures"
    if fixtures.exists():
        shutil.rmtree(fixtures)
    env = dict(os.environ, MEWTUAL_NAMESPACE5_FIXTURES=str(fixtures),
               CARGO_TARGET_DIR=os.environ.get("CARGO_TARGET_DIR", str(REPO / "target")))
    clean_source("namespace5-clean-head-before", REPO, env, directory, results)
    ignored_head_case(
        "predecessor-fixtures", GENERATOR, directory, env, WRITTEN,
        (str(PREDECESSOR_FIXTURES),), results,
    )

    worktree = Path(os.environ.get("RUNNER_TEMP", tempfile.gettempdir())) / "n5-predecessor"
    if subprocess.run(
        ["git", "cat-file", "-e", f"{PREDECESSOR}^{{commit}}"], cwd=REPO, check=False,
    ).returncode != 0:
        git("fetch", "--no-tags", "--depth=1", "origin", PREDECESSOR)
    git("worktree", "add", "--detach", str(worktree), PREDECESSOR)
    try:
        if git("rev-parse", "HEAD", cwd=worktree) != PREDECESSOR:
            raise RuntimeError("predecessor worktree is not at the pinned commit")
        target = worktree / HARNESS_TARGET
        if target.exists():
            raise RuntimeError("the predecessor already has a file at the harness path")
        shutil.copyfile(HARNESS, target)
        # Prove the harness is the ONLY difference from the pinned predecessor tree.
        status = git("status", "--porcelain", "--untracked-files=all", cwd=worktree)
        if status != f"?? {HARNESS_TARGET}":
            raise RuntimeError(f"predecessor tree differs beyond the harness: {status!r}")
        gate_env = env
        command = [
            "cargo", "test", "--locked", "-p", "catcoms-app", "--all-features",
            "--test", "namespace5_gate", "--", "--nocapture", "--test-threads=1",
        ]
        clean_source("namespace5-clean-old", worktree, gate_env, directory, results)
        code, summaries, timed_out, found = run_logged(
            "predecessor-gate", command, directory, cwd=worktree, env=gate_env,
            limit=PREDECESSOR_TIMEOUT_SECONDS, markers=(CHECKED, KEPT),
        )
        result = {
            "name": "predecessor-gate", "commit": PREDECESSOR, "exit": code,
            "timed_out": timed_out,
        }
        results.append(result)
        if timed_out:
            raise RuntimeError("predecessor-gate: deadline exceeded")
        validate("".join(summaries), code, 1)
        require_marker(found[0], (str(PREDECESSOR_FIXTURES), str(PREDECESSOR_OPENERS)))
        require_marker(found[1], (str(ROUND_TRIP),))
        result["fixtures"] = PREDECESSOR_FIXTURES
        result["openers"] = PREDECESSOR_OPENERS
        result["round_trip"] = ROUND_TRIP
    finally:
        retire_old_source(
            lambda: git("worktree", "remove", "--force", str(worktree)),
            lambda: clean_source("namespace5-clean-head-after", REPO, env, directory, results),
            results, sys.exception())

    ignored_head_case(
        "predecessor-round-trip", VERIFIER, directory, env, VERIFIED,
        (str(ROUND_TRIP),), results,
    )


def run() -> None:
    directory = REPO / "logs" / "chat-v2-member-save"
    directory.mkdir(parents=True, exist_ok=True)
    cohorts = list(COHORTS)
    if sys.platform == "win32":
        # The original19 save-module cases plus the native whole-warm-Host
        # integration that now traverses the same real member-save boundary.
        cohorts.append(("member-save-host", "member_save", 20))
    else:
        print("Windows-only member-save host cases are not claimed on this platform.")
    results: list[dict] = []
    try:
        for name, selector, expected in cohorts:
            command = [
                "cargo", "test", "--locked", "-p", "catcoms-app", "--all-features",
                "--lib", selector, "--", "--test-threads=2",
            ]
            code, summaries, timed_out, _ = run_logged(name, command, directory)
            result = {
                "name": name, "selector": selector, "expected": expected,
                "exit": code, "timed_out": timed_out,
            }
            results.append(result)
            if timed_out:
                raise RuntimeError(f"{name}: cohort deadline exceeded")
            validate("".join(summaries), code, expected)
            result["passed"] = expected
        predecessor_gate(directory, results)
    finally:
        (directory / "results.json").write_text(
            json.dumps({"platform": sys.platform, "cohorts": results}, indent=2) + "\n",
            encoding="utf-8",
        )


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    arguments = parser.parse_args()
    if arguments.self_test:
        self_test()
    else:
        run()
