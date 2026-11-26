"""Execute the pinned governed reader against actual head-created namespace6 files."""
from hashlib import sha256
import json
import os
from pathlib import Path
import runpy
import tempfile

REPO = Path(__file__).resolve().parents[2]
GUARD = runpy.run_path(str(Path(__file__).with_name("check-chat-v2-member-save.py")))
PREDECESSOR = "e0ff10336cf428dabfcefbf29652447788b516f9"
TARGET = (
    "crates/catcoms-app/src/store/critical_namespaces/native_registry/operational/"
    "facade/upgrade/governance_tests.rs"
)
HARNESS = REPO / ".github/predecessor/joined_pin_gate.rs"
CASES = {"governed", "staged", "selected"}


def inventory(root: Path) -> dict:
    if {entry.name for entry in root.iterdir()} != CASES:
        raise RuntimeError("expected the exact governed, staged and selected directories")
    for case in CASES:
        entry = root / case
        if entry.is_symlink() or not entry.is_dir():
            raise RuntimeError(f"invalid predecessor fixture {case}")
    result = {}
    for entry in root.rglob("*"):
        if entry.is_symlink() or not (entry.is_file() or entry.is_dir()):
            raise RuntimeError(f"unexpected fixture path {entry.relative_to(root)}")
        metadata = entry.stat()
        digest = sha256(entry.read_bytes()).hexdigest() if entry.is_file() else None
        result[entry.relative_to(root)] = (metadata.st_dev, metadata.st_ino, digest)
    return result


def main() -> None:
    logs = REPO / "logs/chat-v2-invite-intent"
    logs.mkdir(parents=True, exist_ok=True)
    results = []
    git = GUARD["git"]

    def execute(label: str, case: str, root: Path, env: dict, ignored: bool = False) -> None:
        identity = GUARD["locked_source_identity"](root)
        arguments = ["--include-ignored"] if ignored else []
        command = ["cargo", "test", "--locked", "-p", "catcoms-app", "--lib", case,
                   "--", "--test-threads=1", *arguments]
        code, summaries, timed_out, _ = GUARD["run_logged"](
            label, command, logs, cwd=root, env=env,
            limit=GUARD["PREDECESSOR_TIMEOUT_SECONDS"],
        )
        results.append(dict(name=label, **identity, exit=code, timed_out=timed_out))
        if GUARD["locked_source_identity"](root) != identity:
            raise RuntimeError(f"{label} changed source or dependency lock during execution")
        if timed_out:
            raise RuntimeError(f"{label} exceeded its hosted execution bound")
        GUARD["validate"]("".join(summaries), code, 1)

    try:
        with tempfile.TemporaryDirectory(prefix="join6-", dir=os.environ.get("RUNNER_TEMP")) as temp:
            owned = Path(temp).resolve()
            fixtures, old = owned / "fixtures", owned / "p"
            env = dict(os.environ, MEWTUAL_JOINED_PIN_FIXTURES=str(fixtures))
            head_target, old_target = GUARD["isolated_build_targets"](REPO, owned / "t")
            head_env = dict(env, **head_target)
            execute("joined-pin-fixtures", "joined_pin_write_predecessor_fixtures", REPO, head_env, True)
            before = inventory(fixtures)
            fetch = ["fetch", "--no-tags"]
            if git("rev-parse", "--is-shallow-repository") == "true":
                fetch.append("--depth=1")
            git(*fetch, "origin", PREDECESSOR)
            if old.resolve().parent != owned:
                raise RuntimeError("predecessor escaped the owned fixture directory")
            git("worktree", "add", "--detach", str(old), PREDECESSOR)

            def remove_old() -> None:
                if old.resolve().parent != owned:
                    raise RuntimeError("predecessor escaped the owned fixture directory")
                git("worktree", "remove", "--force", str(old))

            try:
                if (git("rev-parse", "HEAD", cwd=old) != PREDECESSOR
                        or git("status", "--porcelain", "--untracked-files=all", cwd=old)):
                    raise RuntimeError("expected pristine pinned governed reader")
                file = old / TARGET
                exact = file.read_bytes() + b"\n" + HARNESS.read_bytes()
                file.write_bytes(exact)

                def require_harness_only() -> None:
                    if (git("status", "--porcelain", "--untracked-files=all", cwd=old) != f"M {TARGET}"
                            or file.read_bytes() != exact):
                        raise RuntimeError("predecessor changed beyond its appended test")

                require_harness_only()
                execute("joined-pin-old", "joined_pin_predecessor_checks_control_and_refuses_both_new_states",
                        old, dict(env, **old_target))
                require_harness_only()
                if inventory(fixtures) != before:
                    raise RuntimeError("predecessor changed the head-created fixture tree")
            finally:
                remove_old()
            execute("joined-pin-round-trip", "joined_pin_verify_predecessor_fixtures", REPO, head_env, True)
    finally:
        (logs / "joined-pin-predecessor.json").write_text(json.dumps(results, indent=2) + "\n", encoding="utf8")


if __name__ == "__main__":
    main()
