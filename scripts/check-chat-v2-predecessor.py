"""Serial two-revision public-open proof; no full suites or existing checkouts changed.

Run with --target-dir pointing at the primary's shared Cargo pipeline target.
The archived production sources are verified byte-for-byte before every build.
Only a new example caller is added to that separate, ignored source tree.
"""
import argparse
import hashlib
import os
from pathlib import Path, PurePosixPath
import subprocess
import tarfile
import tempfile

REVISION = "f5ef22a3688a1272edb418da8b02efe945c3008d"
TEST = ("store::critical_namespaces::governance_layout::tests::review_regressions::"
        "legacy_stage_fence_is_checked_by_actual_predecessor")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target-dir", required=True, type=Path)
    args = parser.parse_args()
    repo = Path(__file__).resolve().parent.parent
    target = args.target_dir.resolve()
    task = repo / "target" / "chat-v2-predecessor-proof"
    task.mkdir(parents=True, exist_ok=True)
    # A reused checkout could hide an extra auto-discovered build.rs/config file
    # outside the archive inventory. Each probe starts from a newly created empty
    # source directory; compilation artifacts go to the separate shared target.
    source = Path(tempfile.mkdtemp(prefix="source-f5-", dir=task))
    archive = task / "predecessor.tar"
    git = ["git", "-c", f"safe.directory={repo.as_posix()}",
           "-c", "core.longpaths=true"]
    subprocess.run(git + ["archive", "--format=tar", f"--output={archive}", REVISION],
                   cwd=repo, check=True)
    with tarfile.open(archive) as saved:
        for member in saved:
            name = PurePosixPath(member.name)
            if name.is_absolute() or ".." in name.parts:
                raise RuntimeError(f"unsafe archive path: {name}")
            destination = source.joinpath(*name.parts)
            destination.resolve().relative_to(source.resolve())
            if member.isdir():
                destination.mkdir(parents=True, exist_ok=True)
                continue
            if not member.isfile():
                raise RuntimeError(f"unsupported archive entry: {name}")
            expected = saved.extractfile(member).read()
            if destination.exists():
                if destination.read_bytes() != expected:
                    raise RuntimeError(f"predecessor production source changed: {name}")
            else:
                destination.parent.mkdir(parents=True, exist_ok=True)
                destination.write_bytes(expected)
                destination.chmod(member.mode & 0o777)
    example = source / "crates/catcoms-app/examples/chat_v2_predecessor_reader.rs"
    example.parent.mkdir(parents=True, exist_ok=True)
    probe = repo / "scripts/chat-v2-predecessor-reader.rs"
    example.write_bytes(probe.read_bytes())
    env = os.environ.copy()
    env.update(CARGO_TARGET_DIR=str(target), CARGO_INCREMENTAL="0", CARGO_PROFILE_DEV_DEBUG="0")
    # Same-name/version path crates from different source revisions can reuse
    # incompatible artifacts in a shared Cargo target. Keep the predecessor's
    # artifacts separate while still running the two builds strictly serially.
    predecessor_target = target / "predecessor-f5"
    predecessor_env = env.copy()
    predecessor_env["CARGO_TARGET_DIR"] = str(predecessor_target)
    subprocess.run(["cargo", "build", "--offline", "--locked", "--manifest-path",
                    str(source / "Cargo.toml"), "-p", "catcoms-app", "--example",
                    "chat_v2_predecessor_reader", "-j", "1"], cwd=source, env=predecessor_env, check=True)
    executable = predecessor_target / "debug/examples/chat_v2_predecessor_reader"
    if os.name == "nt":
        executable = executable.with_suffix(".exe")
    if not executable.is_file():
        raise RuntimeError("missing compiled predecessor probe")
    env["MEWTUAL_CHAT_PREDECESSOR_READER"] = str(executable)
    print(f"Actual predecessor: {REVISION}; caller SHA256: "
          f"{hashlib.sha256(probe.read_bytes()).hexdigest()}", flush=True)
    result = subprocess.run(["cargo", "test", "--offline", "--locked", "--manifest-path",
                    str(repo / "Cargo.toml"), "-p", "catcoms-app", "--all-features",
                    "--lib", "-j", "1", TEST, "--", "--exact", "--test-threads=1",
                    "--nocapture"], cwd=repo, env=env, stdout=subprocess.PIPE,
                    stderr=subprocess.STDOUT, text=True, encoding="utf-8", errors="replace")
    print(result.stdout, end="", flush=True)
    result.check_returncode()
    if "test result: ok. 1 passed; 0 failed" not in result.stdout:
        raise RuntimeError("predecessor gate did not execute exactly one successful test")
    for cut, outcome in [(0, "OPENED"), (229, "REFUSED"), (232, "REFUSED"), (234, "REFUSED")]:
        marker = f"Actual predecessor public open cut {cut}: CURRENT3_{outcome}"
        if marker not in result.stdout:
            raise RuntimeError(f"missing actual predecessor proof: {marker}")


if __name__ == "__main__":
    main()
