//! The second layer of the construction gates (design 8.1, review (ii)).
//!
//! `clippy.toml`'s `disallowed-methods` is the primary gate: it resolves names, so it sees
//! aliases, re-exports and method syntax. This test is the belt to its braces. It runs under plain
//! `cargo test`, so a build that skips Clippy still trips it, and it names the files that may
//! mention the two reserved constructors at all, comments included. A new caller has to be added
//! here deliberately, in review, not merely silenced with an `allow`.
use std::path::{Path, PathBuf};

/// The two hidden constructors. Whoever can call either from bytes of their choosing could turn
/// an archive, an installed checkpoint or copied callback bytes into an Unconfirmed draft base.
const RESERVED: [&str; 2] = ["parse_live_transfer", "mint_from_live_preview"];

/// Where they may appear: their definitions, their one sanctioned production caller, this
/// crate's own tests of them, and this file.
fn is_sanctioned(relative: &str) -> bool {
    matches!(
        relative,
        "crates/catcoms-replication/src/studio/provisional.rs"
            | "crates/catcoms-replication/src/studio/overlay.rs"
            | "crates/catcoms-sync/src/registry_seed/provisional/seed.rs"
            | "crates/catcoms-replication/tests/construction_gates.rs"
    ) || relative.starts_with("crates/catcoms-replication/src/studio/provisional/tests")
        || relative == "crates/catcoms-replication/src/studio/epoch/owner/tests/unconfirmed.rs"
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // Build output and dependencies are not source, and src-tauri keeps its own `target`.
            // A build directory is recognised by the CACHEDIR.TAG Cargo writes into it, not by its
            // name alone, so a source module that happens to be called `target` is still scanned.
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let build_output = name == "target" && path.join("CACHEDIR.TAG").exists();
            if build_output || name == "node_modules" || name.starts_with('.') {
                continue;
            }
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn construction_gates_are_called_only_from_their_sanctioned_sites() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    // The whole src-tauri workspace, not only `src`: its integration tests and `build.rs` depend
    // on catcoms-replication and catcoms-sync directly, and CI runs no Clippy over that workspace.
    for dir in ["crates", "bins", "apps/desktop/src-tauri"] {
        rust_files(&root.join(dir), &mut files);
    }
    assert!(
        files.len() > 100,
        "the scan must actually cover the repository, found only {} files",
        files.len()
    );

    let mut offenders = Vec::new();
    let mut sanctioned_caller_seen = [false; 2];
    for file in &files {
        let relative = file
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let Ok(text) = std::fs::read_to_string(file) else {
            continue;
        };
        for (n, name) in RESERVED.iter().enumerate() {
            if !text.contains(name) {
                continue;
            }
            // A call, not a mention: a surviving comment must not satisfy the non-vacuity check
            // after the call itself has gone.
            if relative == "crates/catcoms-sync/src/registry_seed/provisional/seed.rs"
                && text.contains(&format!("{name}("))
            {
                sanctioned_caller_seen[n] = true;
            }
            if !is_sanctioned(&relative) {
                offenders.push(format!("{relative}: {name}"));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "a reserved constructor appears outside its sanctioned sites (design 8.1 (ii)): {offenders:#?}"
    );
    // Non-vacuity: if the sanctioned caller moved, this scan would pass while checking nothing.
    assert_eq!(
        sanctioned_caller_seen,
        [true, true],
        "the sanctioned caller in catcoms-sync must still use both constructors"
    );
}
