"""Require NATIVE-TEST-001 regressions to detect two isolated native mutations."""
from pathlib import Path
import subprocess


ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / "apps/desktop/src-tauri/src/studio.rs"
COMMAND = [
    "cargo", "test", "--locked", "--manifest-path",
    "apps/desktop/src-tauri/Cargo.toml", "--lib",
]


def run(test):
    return subprocess.run(
        COMMAND + [test, "--", "--exact", "--nocapture"],
        cwd=ROOT, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
        timeout=600, check=False,
    )


def main():
    original = SOURCE.read_bytes()
    mutations = [
        (
            '"awaitingTenureReceipt": true', '"awaitingTenureReceipt": false',
            "studio::tests::preview::native_studio_preview_serializes_real_actor_content",
            "preview trust-state flag",
        ),
        (
            "if preview_delivery\n        .as_ref()\n        .is_some_and(|delivery| !delivery.is_current())",
            "if false",
            "studio::tests::preview::native_studio_preview_expiring_after_conversion_is_rejected",
            "expired preview escaped final native delivery fence",
        ),
    ]
    for before, after, test, assertion in mutations:
        before, after = before.encode(), after.encode()
        if original.count(before) != 1:
            raise AssertionError(f"mutation anchor is not unique: {test}")
        changed = original.replace(before, after, 1)
        try:
            SOURCE.write_bytes(changed)
            result = run(test)
            print(result.stdout, flush=True)
            if not (result.returncode != 0 and assertion in result.stdout
                    and "test result: FAILED. 0 passed; 1 failed;" in result.stdout):
                raise AssertionError(f"mutation did not fail at its intended assertion: {test}")
            print(f"DETECTED: {assertion}", flush=True)
        finally:
            # Refuse to overwrite an unrelated concurrent edit, even on a failed Cargo run.
            if SOURCE.read_bytes() != changed:
                raise AssertionError("native source changed during mutation")
            SOURCE.write_bytes(original)
            assert SOURCE.read_bytes() == original
            print("RESTORED: native source byte-for-byte", flush=True)
    for _, _, test, _ in mutations:
        result = run(test)
        print(result.stdout, flush=True)
        if result.returncode != 0 or "test result: ok. 1 passed; 0 failed;" not in result.stdout:
            raise AssertionError(f"restored native regression failed: {test}")


if __name__ == "__main__":
    main()
