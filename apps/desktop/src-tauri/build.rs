//! Build-time wiring for the Tauri application manifest.
//!
//! SEC-IPC-001. Registering a command with `tauri::generate_handler!` does not, on its own, put it
//! behind the access-control list. Tauri only checks custom application commands when the crate
//! declares an application manifest here; without one, `has_app_acl_manifest` is false and every
//! registered command is callable by every local-origin webview in the process, with no capability
//! entry required. That was this app's state until this file grew past `tauri_build::build()`,
//! which is why a second, deliberately restricted window could not be added safely.
//!
//! Declaring the manifest autogenerates an `allow-<command>` and `deny-<command>` permission for
//! each name below. From that point a command is reachable only from a window whose capability
//! grants it, so the capability files under `capabilities/` become the enforced surface map rather
//! than documentation.
//!
//! The names come from `security/command-policy.json` rather than a list kept here, so the file a
//! human reviews and the list the runtime enforces cannot drift apart. Adding a command to
//! `generate_handler!` without adding it to the policy leaves it denied everywhere, which is the
//! direction a mistake should fail in; the companion tests catch it before anyone meets that at
//! runtime.

use std::{collections::BTreeSet, fs};

use tauri_build::{AppManifest, Attributes};

/// The single reviewed source of truth for what the bridge exposes.
const POLICY_PATH: &str = "security/command-policy.json";

/// Risk classes a policy entry may declare. Adding one is a deliberate edit here, because the
/// vocabulary is what makes the policy file reviewable rather than a list of names.
const RISKS: &[&str] = &[
    "local-session",
    "diagnostics",
    "account-read",
    "account-write",
    "membership-admin",
    "sensitive-authority",
    "os-facing",
    "media-local",
];

/// Window labels a command may be exposed to. A label here is a security identity: it must match
/// a window label the app actually creates, and a capability file that grants it.
const SURFACES: &[&str] = &["main", "security-approval"];

fn main() {
    println!("cargo:rerun-if-changed={POLICY_PATH}");
    tauri_build::try_build(Attributes::new().app_manifest(AppManifest::new().commands(commands())))
        .expect("failed to build the Tauri application manifest");
    link_native_test_resources();
}

/// Tauri links its generated Windows resources to application binaries only.
/// The native libtest also imports rfd's Common Controls v6 entry points, so it
/// must load the SAME manifest before Windows resolves those imports. Cargo's
/// `rustc-link-arg-tests` covers integration targets but not `--test src/lib.rs`;
/// the cfg(test) native_test_manifest module supplies that missing library link.
/// Keep production Tauri resource generation/linkage unchanged: creating another
/// RT_MANIFEST or globally enabling /MANIFEST:EMBED could replace its app resource.
fn link_native_test_resources() {
    // Build-script cfg! describes the HOST; these inputs describe the TARGET.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows")
        || std::env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc")
    {
        return;
    }
    let output = std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("Cargo OUT_DIR"));
    // Pinned tauri-winres/embed-resource names its MSVC resource output
    // resource.lib. It is a linkable .res, not an additional runtime DLL.
    let resource = output.join("resource.lib");
    assert!(
        resource.is_file(),
        "Tauri did not generate {}",
        resource.display()
    );
    // An absolute +verbatim link prevents another dependency's resource.lib or
    // native search path from satisfying this test-only link. link's name must
    // be a literal (env! is not accepted), so emit a tiny private declaration.
    // Rust's own string Debug formatter escapes quotes/backslashes in OUT_DIR.
    let literal = resource.to_str().expect("Cargo OUT_DIR is valid Unicode");
    fs::write(
        output.join("native_test_manifest_link.rs"),
        format!(
            "#[link(name = {literal:?}, kind = \"dylib\", modifiers = \"+verbatim\")]\nunsafe extern \"C\" {{}}\n"
        ),
    )
    .expect("failed to write the native test resource link");
    println!("cargo:rustc-link-arg-tests={}", resource.display());
}

/// Parse and validate the command policy, returning the command names for the ACL manifest.
///
/// Every failure here panics rather than degrading. A policy this build cannot read is not a
/// reason to fall back to the permissive behaviour: it is a reason to stop, because the fallback
/// is precisely the state this file exists to leave.
fn commands() -> &'static [&'static str] {
    let raw = fs::read_to_string(POLICY_PATH)
        .unwrap_or_else(|e| panic!("cannot read the command policy at {POLICY_PATH}: {e}"));
    let parsed: serde_json::Value = serde_json::from_str(&raw)
        .unwrap_or_else(|e| panic!("{POLICY_PATH} is not valid JSON: {e}"));
    let entries = parsed
        .as_array()
        .unwrap_or_else(|| panic!("{POLICY_PATH} must be a JSON array of policy entries"));
    assert!(
        !entries.is_empty(),
        "{POLICY_PATH} is empty, which would silently deny every command",
    );

    let mut seen = BTreeSet::new();
    let mut names: Vec<&'static str> = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        let at = format!("{POLICY_PATH} entry {index}");
        let command = entry
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| panic!("{at} has no string `command`"));
        // The permission identifier is derived by replacing underscores with dashes, so a name
        // outside this shape would generate an identifier that no capability could name back.
        assert!(
            !command.is_empty()
                && command
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'),
            "{at}: command `{command}` must be lower snake_case",
        );
        assert!(
            seen.insert(command.to_string()),
            "{at}: command `{command}` is listed more than once, so one classification is unread",
        );

        let risk = entry
            .get("risk")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| panic!("{at}: command `{command}` has no string `risk`"));
        assert!(
            RISKS.contains(&risk),
            "{at}: command `{command}` has unknown risk `{risk}`; known: {RISKS:?}",
        );

        let surfaces = entry
            .get("surfaces")
            .and_then(|v| v.as_array())
            .unwrap_or_else(|| panic!("{at}: command `{command}` has no `surfaces` array"));
        for surface in surfaces {
            let label = surface
                .as_str()
                .unwrap_or_else(|| panic!("{at}: command `{command}` has a non-string surface"));
            assert!(
                SURFACES.contains(&label),
                "{at}: command `{command}` names unknown surface `{label}`; known: {SURFACES:?}",
            );
        }

        // `commands` needs a 'static slice and the policy is only known at build time. Leaking is
        // correct rather than lazy here: these live for the whole of a short-lived build process.
        names.push(Box::leak(command.to_string().into_boxed_str()));
    }
    Box::leak(names.into_boxed_slice())
}
