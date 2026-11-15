"""Serialized source-integrity, patched protocol and actual old-peer gates.

This is not a replacement for full tests, strict Clippy, dependency audits or
physical-network acceptance. The predecessor's only addition is the exact same
public-API example; its shipping sources/manifests/lock are never changed.
"""
from __future__ import annotations

import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import queue
import re
import runpy
import shutil
import subprocess
import sys
import tarfile
import tempfile
import threading
import time
import tomllib
import urllib.request

REPO = Path(__file__).resolve().parents[2]
BASE = runpy.run_path(str(REPO / ".github/scripts/check-chat-v2-member-save.py"))
OLD = "944115c2bce5e028f55f8a2c2685b4c23396ea51"
EXAMPLE = "crates/catcoms-net/examples/network_compat_peer.rs"
CRATE_SHA = "6c28d7256540bd1a5df45cf6d7a7d8c58e913ad63ccfaa2f97983ec077b2794a"
CHANGED = {"Cargo.toml", "Cargo.lock", "src/lib.rs", "src/behaviour.rs", "src/protocol.rs"}
ADDED = {"MEWTUAL-UPSTREAM.json", "MEWTUAL-PATCH.md", "LICENSE.upstream",
         "src/subscription_chunks.rs", "src/protocol/mewtual_tests.rs"}


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def git(*args: str, cwd: Path = REPO) -> str:
    result = subprocess.run(["git", "-C", str(cwd), "-c", f"safe.directory={cwd}",
                             "-c", "core.longpaths=true", *args],
                            capture_output=True, text=True, check=True)
    return result.stdout.strip()


def package_version(lock: bytes, name: str) -> list[str]:
    return [p["version"] for p in tomllib.loads(lock.decode("utf-8"))["package"]
            if p["name"] == name]


def integrity() -> None:
    """Compare all packaged files against the checksum-pinned published archive."""
    url = "https://static.crates.io/crates/libp2p-gossipsub/libp2p-gossipsub-0.50.1.crate"
    with urllib.request.urlopen(url, timeout=60) as response:
        data = response.read(2 * 1024 * 1024)
    if sha(data) != CRATE_SHA:
        raise RuntimeError("published gossipsub archive checksum mismatch")
    originals: dict[str, str] = {}
    with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as archive:
        for entry in archive:
            if not entry.isfile():
                raise RuntimeError("unexpected non-file archive entry")
            path = Path(entry.name)
            if path.parts[0] != "libp2p-gossipsub-0.50.1" or ".." in path.parts:
                raise RuntimeError("unexpected archive path")
            relative = Path(*path.parts[1:]).as_posix()
            stream = archive.extractfile(entry)
            assert stream is not None
            if relative in originals:
                raise RuntimeError("duplicate archive path")
            originals[relative] = sha(stream.read())
    vendor = REPO / "vendor/libp2p-gossipsub"
    recorded = json.loads((vendor / "MEWTUAL-UPSTREAM.json").read_text(encoding="utf-8"))
    if len(originals) != 46 or recorded["files"] != originals or recorded["archive_sha256"] != CRATE_SHA:
        raise RuntimeError("vendor provenance inventory differs from the published archive")
    present = {p.relative_to(vendor).as_posix() for p in vendor.rglob("*")
               if p.is_file() and not p.relative_to(vendor).parts[0] == "target"}
    if present != originals.keys() | ADDED:
        raise RuntimeError(f"unexpected vendor inventory: {present ^ (originals.keys() | ADDED)}")
    changed = {name for name, digest in originals.items()
               if sha((vendor / name).read_bytes()) != digest}
    if changed != CHANGED:
        raise RuntimeError(f"unexpected patched upstream files: {changed}")
    for workspace in (REPO, REPO / "apps/desktop/src-tauri"):
        manifest = tomllib.loads((workspace / "Cargo.toml").read_text(encoding="utf-8"))
        patch = manifest["patch"]["crates-io"]["libp2p-gossipsub"]["path"]
        if (workspace / patch).resolve() != vendor.resolve():
            raise RuntimeError("workspaces do not patch the same actual runtime")
        lock = (workspace / "Cargo.lock").read_bytes()
        for name, expected in (("libp2p", "0.57.1"), ("libp2p-gossipsub", "0.50.1"),
                               ("hickory-proto", "0.26.3"), ("hickory-net", "0.26.3"),
                               ("hickory-resolver", "0.26.3")):
            if package_version(lock, name) != [expected]:
                raise RuntimeError(f"wrong actual {workspace.name} resolution for {name}")
        gossip = [p for p in tomllib.loads(lock.decode("utf-8"))["package"]
                  if p["name"] == "libp2p-gossipsub"]
        if "source" in gossip[0]:
            raise RuntimeError("shipping lock bypasses the local correction")
    print("Verified complete 46-file archive provenance, five changed upstream files and both shipping patches.")


class Peer:
    """One owned child; drain bounded lines continuously, even while awaiting a marker."""
    def __init__(self, command: list[str], label: str, directory: Path):
        self.events: queue.Queue[tuple[str, str]] = queue.Queue(maxsize=64)
        self.label = label
        self.problem: list[str] = []
        self.lines: list[str] = []
        self.process = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                        stderr=subprocess.PIPE, text=True, encoding="utf-8",
                                        start_new_session=sys.platform != "win32")
        self.threads = []
        for kind, pipe in (("out", self.process.stdout), ("err", self.process.stderr)):
            thread = threading.Thread(target=self.drain,
                                      args=(kind, pipe, directory / f"{label}-{kind}.log"), daemon=True)
            thread.start()
            self.threads.append(thread)

    def drain(self, kind, pipe, path: Path) -> None:
        with path.open("w", encoding="utf-8") as log:
            for _ in range(128):
                line = pipe.readline(4097)
                if not line:
                    return
                log.write(line)
                log.flush()
                if len(line) > 4096 or not line.endswith("\n"):
                    self.problem.append("oversized/incomplete peer output line")
                    return
                if kind == "out":
                    try:
                        self.events.put_nowait((kind, line.rstrip("\r\n")))
                    except queue.Full:
                        self.problem.append("peer output queue exceeded bound")
                        return
            self.problem.append("peer output exceeded line bound")

    def line(self, deadline: float) -> str:
        while time.monotonic() < deadline:
            if self.problem:
                raise RuntimeError(f"{self.label}: {self.problem}")
            try:
                _, line = self.events.get(timeout=min(0.1, max(0.001, deadline-time.monotonic())))
                self.lines.append(line)
                return line
            except queue.Empty:
                if self.process.poll() is not None:
                    raise RuntimeError(f"{self.label} exited before expected marker: {self.lines}")
        raise RuntimeError(f"{self.label} marker deadline: {self.lines}")

    def send(self, value: str) -> None:
        self.process.stdin.write(value + "\n")
        self.process.stdin.flush()

    def stop(self) -> None:
        if self.process.poll() is None:
            BASE["stop_owned_cohort"](self.process, threading.Event(), 0)
        self.process.wait(timeout=30)
        for thread in self.threads:
            thread.join(timeout=5)
        if any(thread.is_alive() for thread in self.threads):
            raise RuntimeError("owned peer pipe failed to retire")


def peers(old_binary: Path, current_binary: Path, directory: Path, proof: dict, *, pressure: bool) -> None:
    directory.mkdir(parents=True, exist_ok=True)
    if sha(old_binary.read_bytes()) != proof["server_binary_sha256"]:
        raise RuntimeError("old executable changed before spawn")
    if sha(current_binary.read_bytes()) != proof["client_binary_sha256"]:
        raise RuntimeError("current executable changed before spawn")
    deadline = time.monotonic() + 45  # ONE deadline, never renewed by retries/output.
    scenario = "--pressure" if pressure else "--wait-target"
    proof["publication_phase"] = "held_128_then_2000_subscriptions_without_ready_wait" if pressure else "RR_then_actual_target_adoption"
    old = Peer([str(old_binary), "server", scenario], "old", directory)
    current = None
    try:
        ready = old.line(deadline)
        match = re.fullmatch(r"READY (12D3KooW[1-9A-HJ-NP-Za-km-z]{44}) (/ip4/127\.0\.0\.1/udp/([0-9]{1,5})/quic-v1)", ready)
        if match is None or not 0 < int(match[3]) <= 65535:
            raise RuntimeError(f"invalid old-peer ready marker: {ready!r}")
        current = Peer([str(current_binary), "client", f"{match[2]}/p2p/{match[1]}", scenario], "current", directory)
        if pressure:
            if current.line(deadline) != "HELD client 128":
                raise RuntimeError("pressure scenario did not hold 128 actual publications")
            if current.line(deadline) != "PRESSURE_RR client":
                raise RuntimeError("request/reply did not progress while 128 originals were held")
            proof["request_reply_before_subscription_release"] = True
            old.send("release-target")
        if old.line(deadline) != "EXCHANGED server 2000 61440":
            raise RuntimeError("old peer did not receive exact current traffic")
        if current.line(deadline) != "EXCHANGED client 2000 61440":
            raise RuntimeError("current peer did not receive exact old traffic")
        current.send("check-connected")
        if current.line(deadline) != "PRESENT client":
            raise RuntimeError("original path was lost before controlled old Mesh Drop")
        old.send("drop")
        if old.line(deadline) != "DROPPED server":
            raise RuntimeError("old Mesh owner did not drop")
        current.send("observe-withdrawal")
        if current.line(deadline) != "WITHDRAWN client" or old.process.poll() is not None:
            raise RuntimeError("withdrawal must precede old process exit")
        if current.line(deadline) != "COMPLETE client":
            raise RuntimeError("current peer did not finish")
        old.send("exit")
        if old.line(deadline) != "COMPLETE server":
            raise RuntimeError("old peer did not finish")
        for child in (old, current):
            if child.process.wait(timeout=max(0.01, deadline-time.monotonic())) != 0:
                raise RuntimeError(f"{child.label} exited unsuccessfully")
            for thread in child.threads:
                thread.join(timeout=5)
            if child.problem or not child.events.empty():
                raise RuntimeError(f"unexpected/repeated peer markers: {child.label}")
        proof["old_markers"] = old.lines
        proof["current_markers"] = current.lines
        if pressure:
            proof["post_original_receipt_acknowledged"] = True
            proof["originals_before_release"] = 128
            proof["active_signing_oracle_range"] = [256, 1280]
        print(f"{'Current pressure' if pressure else 'Phased old-peer'} gate passed: two actual processes, bidirectional RR/gossip, 2000 topics, live-process Mesh withdrawal.")
    finally:
        stop_peers(current, old)


def stop_peers(current: Peer | None, old: Peer) -> None:
    # The second owned child must still be retired if the first cleanup fails.
    try:
        if current is not None:
            current.stop()
    finally:
        old.stop()


def self_test() -> None:
    stopped = []
    class Fake:
        def __init__(self, name: str, fail: bool):
            self.name, self.fail = name, fail
        def stop(self):
            stopped.append(self.name)
            if self.fail:
                raise RuntimeError("injected first cleanup failure")
    try:
        stop_peers(Fake("current", True), Fake("old", False))
    except RuntimeError as error:
        assert str(error) == "injected first cleanup failure"
    else:
        raise AssertionError("injected cleanup failure disappeared")
    assert stopped == ["current", "old"]
    BASE["self_test"]()
    print("Both owned children are retired even when the first cleanup fails; no Cargo/network invoked.")


def run() -> None:
    directory = REPO / "logs/network-security"
    directory.mkdir(parents=True, exist_ok=True)
    target_directory = Path(os.environ.get("CARGO_TARGET_DIR", str(REPO / "target")))
    if not target_directory.is_absolute():
        target_directory = REPO / target_directory
    env = dict(os.environ, CARGO_TARGET_DIR=str(target_directory.resolve()))
    binary_name = "network_compat_peer" + (".exe" if sys.platform == "win32" else "")
    artifact = Path(env["CARGO_TARGET_DIR"]) / "debug/examples" / binary_name
    proof = {"old_head": OLD, "head": git("rev-parse", "HEAD"),
             "current_worktree_status": git("status", "--porcelain", "--untracked-files=all"),
             "current_lock_sha256": sha((REPO / "Cargo.lock").read_bytes()),
             "current_stack": {"libp2p": "0.57.1", "libp2p-gossipsub": "0.50.1"},
             "old_stack": {"libp2p": "0.56.0", "libp2p-gossipsub": "0.49.5"},
             "example_sha256": sha((REPO / EXAMPLE).read_bytes())}

    def cargo(name: str, args: list[str], cwd: Path = REPO) -> None:
        code, _, timed_out, _ = BASE["run_logged"](name, ["cargo", *args], directory,
                                                 cwd=cwd, env=env, limit=40*60)
        if code or timed_out:
            raise RuntimeError(f"{name} failed: {code}, timed_out={timed_out}")

    # Build/copy head before any package cleanup; the executable must never be
    # replaced by the opposite worktree's same-named artifact.
    cargo("head-build", ["build", "--locked", "-p", "catcoms-net", "--example", "network_compat_peer"])
    current_binary = directory / ("current-" + binary_name)
    old_binary = directory / ("old-" + binary_name)
    shutil.copy2(artifact, current_binary)
    proof["current_binary_sha256"] = sha(current_binary.read_bytes())
    with tempfile.TemporaryDirectory(prefix="net-old-", dir=os.environ.get("RUNNER_TEMP")) as temporary:
        owned = Path(temporary).resolve()
        worktree = owned / "p"
        if not worktree.resolve().is_relative_to(owned):
            raise RuntimeError("old worktree escaped owned directory")
        git("fetch", "--no-tags", "origin", OLD)
        git("worktree", "add", "--detach", str(worktree), OLD)
        try:
            target = worktree / EXAMPLE
            target.parent.mkdir(parents=True, exist_ok=True)
            if target.exists():
                raise RuntimeError("old checkout already contains the harness")
            shutil.copyfile(REPO / EXAMPLE, target)
            old_lock = (worktree / "Cargo.lock").read_bytes()
            if package_version(old_lock, "libp2p") != ["0.56.0"] or package_version(old_lock, "libp2p-gossipsub") != ["0.49.5"]:
                raise RuntimeError("predecessor is not the expected original stack")
            proof["old_lock_sha256"] = sha(old_lock)

            def original() -> None:
                if git("rev-parse", "HEAD", cwd=worktree) != OLD or git("status", "--porcelain", "--untracked-files=all", cwd=worktree) != f"?? {EXAMPLE}":
                    raise RuntimeError("old sources differ beyond their one added harness")
                if (worktree / "Cargo.lock").read_bytes() != old_lock or target.read_bytes() != (REPO / EXAMPLE).read_bytes():
                    raise RuntimeError("old lock/harness changed during gate")

            original()
            try:
                cargo("clear-head-net-rt", ["clean", "-p", "catcoms-net", "-p", "catcoms-rt"])
                cargo("old-build", ["build", "--locked", "-p", "catcoms-net", "--example", "network_compat_peer"], worktree)
                original()
                shutil.copy2(artifact, old_binary)
                proof["old_binary_sha256"] = sha(old_binary.read_bytes())
            finally:
                # Package-only cleanup: target contains other worktrees. Never
                # remove it wholesale, and never leave stale old Root artifacts.
                cargo("clear-old-net-rt", ["clean", "-p", "catcoms-net", "-p", "catcoms-rt"])
                cargo("restore-head", ["build", "--locked", "-p", "catcoms-net", "--example", "network_compat_peer"])
        finally:
            git("worktree", "remove", "--force", str(worktree))
    proof["server_binary_sha256"] = proof["old_binary_sha256"]
    proof["client_binary_sha256"] = proof["current_binary_sha256"]
    peers(old_binary, current_binary, directory, proof, pressure=False)
    (directory / "interop-proof.json").write_text(json.dumps(proof, indent=2) + "\n", encoding="utf-8")
    current_proof = {
        "head": proof["head"], "current_worktree_status": proof["current_worktree_status"],
        "example_sha256": proof["example_sha256"],
        "server_stack": proof["current_stack"], "client_stack": proof["current_stack"],
        "server_binary_sha256": proof["current_binary_sha256"],
        "client_binary_sha256": proof["current_binary_sha256"],
    }
    peers(current_binary, current_binary, directory / "current-pressure", current_proof, pressure=True)
    (directory / "pressure-proof.json").write_text(json.dumps(current_proof, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--integrity-only", action="store_true")
    parser.add_argument("--self-test", action="store_true")
    options = parser.parse_args()
    if options.self_test:
        self_test()
        sys.exit(0)
    integrity()
    if not options.integrity_only:
        run()
