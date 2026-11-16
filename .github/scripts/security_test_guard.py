"""Bounded security-test execution, independent of the unfinished Chat V2 gates.

Extracted from the existing reviewed exact-count runner. Keep subprocess ownership,
strict nonempty result checks and bounded in-memory diagnostics together: a renamed
filter, duplicate summary or cancelled child must never become passing evidence.
"""
from __future__ import annotations

import os
from pathlib import Path
import re
import signal
import subprocess
import sys
import threading

REPO = Path(__file__).resolve().parents[2]
ANSI = re.compile(r"\x1b\[[0-9;]*m")
SUMMARY = re.compile(r"test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;")


def validate(output: str, exit_code: int, expected: int) -> None:
    """Require one complete, nonignored cohort of precisely the reviewed size."""
    summaries = SUMMARY.findall(ANSI.sub("", output))
    if exit_code != 0 or expected <= 0 or summaries != [(str(expected), "0", "0")]:
        raise RuntimeError(f"expected exactly {expected} passing tests; exit={exit_code}, summaries={summaries!r}")


def self_test() -> None:
    """Exercise fail-closed parsing without running Cargo or network programs."""
    good = "test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured;"
    validate(good, 0, 2)
    validate(good.replace("ok", "\x1b[32mok\x1b[0m"), 0, 2)
    for output, code, expected in (
        ("", 0, 2), (good, 101, 2), (good, 0, 1), (good, 0, 0),
        (good.replace("2 passed", "0 passed"), 0, 2),
        (good.replace("0 failed", "1 failed"), 0, 2),
        (good.replace("0 ignored", "1 ignored"), 0, 2),
        (good + "\n" + good, 0, 2),
    ):
        try:
            validate(output, code, expected)
        except RuntimeError:
            continue
        raise AssertionError("an empty, failed, ignored, duplicate or wrong-size cohort was accepted")


def stop_owned_cohort(process: subprocess.Popen[str], timed_out: threading.Event, limit: int) -> None:
    """Stop only this still-owned child tree; never find processes by name."""
    if process.poll() is not None:
        return
    timed_out.set()
    print(f"Stopping owned child after {limit}s.", flush=True)
    if sys.platform == "win32":
        subprocess.run(["taskkill.exe", "/PID", str(process.pid), "/T", "/F"],
                       check=False, timeout=30)
    else:
        # Every child starts a new session, including its Cargo/test descendants.
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass


def run_logged(name: str, command: list[str], directory: Path, *, cwd: Path = REPO,
               env: dict[str, str] | None = None, limit: int = 15 * 60,
               markers: tuple[re.Pattern[str], ...] = (),
               ) -> tuple[int, list[str], bool, list[list[tuple[str, ...]]]]:
    """Stream a child to disk, retaining at most two summaries/marker matches.

    Two summaries already fail the exact-one check. Always retire the child on
    exceptional output/log handling too, before cancelling its deadline thread.
    """
    if limit <= 0:
        raise ValueError("execution limit must be positive")
    directory.mkdir(parents=True, exist_ok=True)
    print(f"{name}: {' '.join(command)}", flush=True)
    summaries: list[str] = []
    found: list[list[tuple[str, ...]]] = [[] for _ in markers]
    with (directory / f"{name}.log").open("w", encoding="utf-8") as log:
        with subprocess.Popen(command, cwd=cwd, env=env, stdout=subprocess.PIPE,
                              stderr=subprocess.STDOUT, text=True, encoding="utf-8",
                              errors="replace", start_new_session=sys.platform != "win32") as process:
            timed_out = threading.Event()
            deadline = threading.Timer(limit, stop_owned_cohort, args=(process, timed_out, limit))
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
            except BaseException:
                stop_owned_cohort(process, threading.Event(), limit)
                process.wait(timeout=30)
                raise
            finally:
                deadline.cancel()
                deadline.join()
    return code, summaries, timed_out.is_set(), found
