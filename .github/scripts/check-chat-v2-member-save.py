"""Run exact-count member-storage cohorts without substituting for strict/full CI.

The save host is Windows-only today. Do not run its filter on Linux and credit a
zero-test success. Storage/catalog cohorts execute on both operating systems.
"""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import sys
import threading


REPO = Path(__file__).resolve().parents[2]
ANSI = re.compile(r"\x1b\[[0-9;]*m")
SUMMARY = re.compile(
    r"test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;"
)
COHORT_TIMEOUT_SECONDS = 15 * 60

# Keep expected counts explicit: renamed/empty filters and unexpected ignored cases
# fail instead of quietly shrinking the evidence. Changes require reviewing the cases.
COHORTS = (
    ("sorted-declarations", "critical_namespaces::catalog_declaration", 4),
    ("namespace-codec", "critical_namespaces::member_outbox_namespace_codec", 6),
    ("join-producer", "critical_namespaces::pending_join::producer::member_outbox_tests", 2),
    ("current-mount", "critical_namespaces::current_layout::tests::member_outbox", 9),
    ("governance-mount", "critical_namespaces::governance_layout::tests::member_outbox", 5),
    ("outbox-engine", "v2_member_outbox", 31),
)


def validate(output: str, exit_code: int, expected: int) -> None:
    summaries = SUMMARY.findall(ANSI.sub("", output))
    if exit_code != 0 or summaries != [(str(expected), "0", "0")] or expected <= 0:
        raise RuntimeError(
            f"expected exactly {expected} passing, nonignored tests; "
            f"exit={exit_code}, summaries={summaries!r}"
        )


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
    print("Exact-count guard self-checks passed; Cargo was not invoked.")


def stop_owned_cohort(process: subprocess.Popen[str], timed_out: threading.Event) -> None:
    """Bound deadlock regressions by stopping only this still-owned child tree."""
    if process.poll() is not None:
        return
    timed_out.set()
    print(f"Cohort exceeded {COHORT_TIMEOUT_SECONDS}s; stopping its child tree.", flush=True)
    if sys.platform == "win32":
        subprocess.run(
            ["taskkill.exe", "/PID", str(process.pid), "/T", "/F"], check=False, timeout=30,
        )
    else:
        # Each cohort starts a fresh session, so this group contains only its Cargo/test
        # children. Killing the group also releases an inherited stdout pipe on deadlock.
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass


def run() -> None:
    directory = REPO / "logs" / "chat-v2-member-save"
    directory.mkdir(parents=True, exist_ok=True)
    cohorts = list(COHORTS)
    if sys.platform == "win32":
        cohorts.append(("member-save-host", "member_save", 18))
    else:
        print("Windows-only member-save host cases are not claimed on this platform.")
    results = []
    try:
        for name, selector, expected in cohorts:
            command = [
                "cargo", "test", "--locked", "-p", "catcoms-app", "--all-features",
                "--lib", selector, "--", "--test-threads=2",
            ]
            print(f"\n{name}: {' '.join(command)}", flush=True)
            # Stream full diagnostics to the artifact, retaining at most two summaries.
            # Two already makes the exact-one-summary guard fail; do not buffer build logs.
            summaries = []
            with (directory / f"{name}.log").open("w", encoding="utf-8") as log:
                with subprocess.Popen(
                    command, cwd=REPO, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                    text=True, encoding="utf-8", errors="replace",
                    start_new_session=sys.platform != "win32",
                ) as process:
                    timed_out = threading.Event()
                    deadline = threading.Timer(
                        COHORT_TIMEOUT_SECONDS, stop_owned_cohort, args=(process, timed_out),
                    )
                    deadline.start()
                    try:
                        assert process.stdout is not None
                        for line in process.stdout:
                            log.write(line)
                            log.flush()
                            print(line, end="", flush=True)
                            if len(summaries) < 2 and SUMMARY.search(ANSI.sub("", line)):
                                summaries.append(line)
                        code = process.wait()
                    finally:
                        deadline.cancel()
                        deadline.join()
            result = {
                "name": name, "selector": selector, "expected": expected,
                "exit": code, "timed_out": timed_out.is_set(),
            }
            results.append(result)
            if timed_out.is_set():
                raise RuntimeError(f"{name}: cohort deadline exceeded")
            validate("".join(summaries), code, expected)
            result["passed"] = expected
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
