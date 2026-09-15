"""Isolated inspection mutations: require one intended failure, exact restoration, then PASS."""
from pathlib import Path
import argparse
import subprocess

ROOT = Path(__file__).resolve().parents[2]
STORE = "crates/catcoms-app/src/store/epoch_intents/inspection.rs"
NATIVE = "apps/desktop/src-tauri/src/studio.rs"
PREFIX = "store::epoch_studio::tests::rotation::overlay::handoff::inspection::"
NPREFIX = "studio::inspection::tests::"
MUTATIONS = {
    "app": [
        (STORE, "Ok(version == stamp.version)", "Ok(true)",
         PREFIX + "studio_inspection_full_wrapper_change_with_unchanged_draft_is_stale",
         "changed complete wrapper escaped inspection currency fence"),
        (STORE, "metadata.check_target(self.stamp.target).map_err(invalid)?;", "",
         PREFIX + "studio_inspection_author_and_full_channel_are_validated_detached",
         "foreign channel escaped detached inspection"),
        (STORE, ".is_some_and(|o| o.author() != self.stamp.author)", ".is_some_and(|_| false)",
         PREFIX + "studio_inspection_author_and_full_channel_are_validated_detached",
         "foreign author escaped detached inspection"),
        (STORE, "Ok(version == stamp.version)",
         "Ok(version.as_ref().map(|(_, bytes)| bytes) == stamp.version.as_ref().map(|(_, bytes)| bytes))",
         PREFIX + "studio_inspection_same_size_authenticated_replacement_is_stale",
         "same-sized authenticated replacement escaped inspection digest fence"),
    ],
    "native": [
        (NATIVE, "|| inspection_delivery\n            .as_ref()\n            .is_some_and(|delivery| !delivery.is_current())", "",
         NPREFIX + "native_studio_inspection_expiring_after_conversion_is_rejected",
         "expired inspection escaped final native delivery fence"),
        ("apps/desktop/src-tauri/src/studio/inspection.rs", "after_rebuild(&context);",
         "after_rebuild(&context);\n    let context = InvokeContext::new(state, server, Some(target)).await?;",
         NPREFIX + "native_studio_inspection_original_session_request_and_instance_span_both_visits",
         "obsolete first inspection was legitimized by second custody visit"),
    ],
}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("kind", choices=MUTATIONS)
    kind = parser.parse_args().kind
    command = ["cargo", "test", "--locked", "-j", "4"]
    if kind == "app":
        command += ["--config", "profile.test.package.catcoms-app.debug=0", "-p", "catcoms-app"]
    else:
        command += ["--manifest-path", "apps/desktop/src-tauri/Cargo.toml"]
    command += ["--lib"]

    def run(test, label):
        result = subprocess.run(command + [test, "--", "--exact", "--nocapture"], cwd=ROOT,
                                text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                timeout=900, check=False)
        folder = ROOT / "logs/studio-inspection-mutations"
        folder.mkdir(parents=True, exist_ok=True)
        (folder / f"{kind}-{label}.log").write_text(result.stdout, encoding="utf-8")
        print(result.stdout, flush=True)
        return result

    for number, (file, before, after, test, assertion) in enumerate(MUTATIONS[kind]):
        source = ROOT / file
        original = source.read_bytes()
        newline = "\r\n" if b"\r\n" in original else "\n"
        before, after = (text.replace("\n", newline).encode() for text in (before, after))
        if original.count(before) != 1:
            raise AssertionError(f"mutation anchor is not unique: {test}")
        changed = original.replace(before, after, 1)
        try:
            source.write_bytes(changed)
            result = run(test, f"{number}-mutated")
            if not (result.returncode != 0 and assertion in result.stdout
                    and "test result: FAILED. 0 passed; 1 failed;" in result.stdout):
                raise AssertionError(f"mutation missed intended executed assertion: {test}")
            print(f"DETECTED: {assertion}", flush=True)
        finally:
            if source.read_bytes() != changed:
                raise AssertionError("source changed concurrently during mutation")
            source.write_bytes(original)
            assert source.read_bytes() == original
            print(f"RESTORED: {file} byte-for-byte", flush=True)
        result = run(test, f"{number}-restored")
        if result.returncode != 0 or "test result: ok. 1 passed; 0 failed;" not in result.stdout:
            raise AssertionError(f"restored regression failed: {test}")


if __name__ == "__main__":
    main()
