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
    ("join-producer", "critical_namespaces::pending_join::producer::member_outbox_tests", 2),
    ("current-mount", "critical_namespaces::current_layout::tests::member_outbox", 18),
    ("governance-mount", "critical_namespaces::governance_layout::tests::member_outbox", 8),
    ("outbox-engine", "v2_member_outbox", 32),
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
    print("Exact-count guard self-checks passed; Cargo was not invoked.")


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
) -> tuple[int, list[str], bool, list[list[tuple[str, ...]]]]:
    """Stream one owned subprocess to its artifact log under a deadline.

    Returns the exit code, at most two test summaries, whether the deadline fired, and for
    each marker pattern at most two matches. Two of either already fails the exact-one
    guards, so build logs are never buffered.
    """
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
                        if match is not None and len(found[index]) < 2:
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
    env = dict(os.environ, MEWTUAL_NAMESPACE5_FIXTURES=str(fixtures))
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
        gate_env = dict(env, CARGO_TARGET_DIR=str(REPO / "target"))
        command = [
            "cargo", "test", "--locked", "-p", "catcoms-app", "--all-features",
            "--test", "namespace5_gate", "--", "--nocapture", "--test-threads=1",
        ]
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
        subprocess.run(
            ["git", "worktree", "remove", "--force", str(worktree)], cwd=REPO, check=False,
        )

    ignored_head_case(
        "predecessor-round-trip", VERIFIER, directory, env, VERIFIED,
        (str(ROUND_TRIP),), results,
    )


def run() -> None:
    directory = REPO / "logs" / "chat-v2-member-save"
    directory.mkdir(parents=True, exist_ok=True)
    cohorts = list(COHORTS)
    if sys.platform == "win32":
        cohorts.append(("member-save-host", "member_save", 19))
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
