//! What each window is actually allowed to invoke.
//!
//! SEC-IPC-001, SEC-IPC-002 and SEC-IPC-003. These tests call `RuntimeAuthority::resolve_access`,
//! which is the same function `Webview::on_message` calls before it dispatches an IPC message, on
//! the authority built by `generate_context!` from the real generated ACL. So a denial here means
//! the command body is never entered, rather than meaning a handler returned an error: the
//! dispatcher stops at the `None`.
//!
//! That is deliberately not a source scan and not a mock. A regex over `capabilities/main.json`
//! proves the file says something; this proves the runtime resolves it that way. What it still
//! does not prove is process isolation: a compromised renderer that runs native code inside the
//! `main` webview holds `main`'s label and therefore `main`'s permissions. Capability separation
//! is an IPC boundary, not a memory boundary, and nothing in this file should be read as claiming
//! otherwise.

use std::collections::BTreeSet;

use tauri::ipc::{Origin, RuntimeAuthority};

/// The reviewed policy that `build.rs` feeds into the ACL manifest.
const POLICY: &str = include_str!("../security/command-policy.json");

/// Window labels the app may ever create. A command must resolve for the surfaces its policy entry
/// names and for no others, so adding a label here without granting it anything is the safe state.
const KNOWN_SURFACES: &[&str] = &["main", "security-approval"];

struct Entry {
    command: String,
    surfaces: BTreeSet<String>,
}

fn policy() -> Vec<Entry> {
    let parsed: serde_json::Value = serde_json::from_str(POLICY).expect("policy must be valid JSON");
    let entries = parsed.as_array().expect("policy must be an array");
    assert!(
        // A policy that parsed to nothing would make every assertion below vacuously true, which
        // is the failure mode that lets an empty allow-set look like a passing security test.
        entries.len() > 150,
        "policy has only {} entries, far below the registered command count: the file is being \
         under-read and an under-read policy proves nothing",
        entries.len(),
    );
    entries
        .iter()
        .map(|entry| Entry {
            command: entry["command"].as_str().expect("command").to_string(),
            surfaces: entry["surfaces"]
                .as_array()
                .expect("surfaces")
                .iter()
                .map(|s| s.as_str().expect("surface").to_string())
                .collect(),
        })
        .collect()
}

/// The real context the app runs with, carrying the authority built from the generated ACL.
/// Held by value in each test because the authority borrows from it.
fn context() -> tauri::Context {
    tauri::generate_context!()
}

/// Whether the authority would dispatch `command` for a webview carrying `label`.
fn allowed(authority: &RuntimeAuthority, command: &str, label: &str, origin: &Origin) -> bool {
    authority
        .resolve_access(command, label, label, origin)
        .is_some_and(|resolved| !resolved.is_empty())
}

#[test]
fn every_policy_command_resolves_for_the_surfaces_it_names() {
    // The direction that catches a broken app rather than a leaky one. Declaring the ACL manifest
    // flips custom commands from "callable by every local webview" to "callable only where a
    // capability grants it", so a command missing from the capability file is not a weaker app,
    // it is a feature that fails at runtime with nothing at compile time to warn anyone.
    let mut context = context();
    let authority = context.runtime_authority_mut();
    let mut ungranted = Vec::new();
    for entry in policy() {
        for surface in &entry.surfaces {
            if !allowed(&authority, &entry.command, surface, &Origin::Local) {
                ungranted.push(format!("{} on {surface}", entry.command));
            }
        }
    }
    assert!(
        ungranted.is_empty(),
        "these commands are classified for a surface that cannot invoke them, so the feature is \
         broken at runtime; add the allow-<command> permission to that capability: {ungranted:#?}",
    );
}

#[test]
fn no_command_resolves_for_a_surface_its_policy_does_not_name() {
    // SEC-IPC-003. Capabilities merge when the same label appears in more than one of them, and a
    // wildcard window scope grants every future window at once. This is the assertion that makes
    // adding the restricted approval surface safe: it must start with nothing and receive exactly
    // what it is given, rather than inheriting main's set by virtue of existing.
    let mut context = context();
    let authority = context.runtime_authority_mut();
    let mut leaked = Vec::new();
    for entry in policy() {
        for surface in KNOWN_SURFACES {
            if entry.surfaces.contains(*surface) {
                continue;
            }
            if allowed(&authority, &entry.command, surface, &Origin::Local) {
                leaked.push(format!("{} leaks to {surface}", entry.command));
            }
        }
    }
    assert!(
        leaked.is_empty(),
        "these commands resolve for a surface their policy does not name: {leaked:#?}",
    );
}

#[test]
fn an_unlisted_window_label_gets_nothing_at_all() {
    // A label nobody granted anything is the state every new window starts in. If this ever
    // resolves, some capability is using a wildcard window scope.
    let mut context = context();
    let authority = context.runtime_authority_mut();
    let mut leaked = Vec::new();
    for entry in policy() {
        for label in ["media", "unregistered-window", ""] {
            if allowed(&authority, &entry.command, label, &Origin::Local) {
                leaked.push(format!("{} resolves for the unlisted label {label:?}", entry.command));
            }
        }
    }
    assert!(leaked.is_empty(), "{leaked:#?}");
}

#[test]
fn a_remote_origin_resolves_nothing_even_on_main() {
    // Defence in depth for the framed third-party embeds. Note what this does and does not settle:
    // it settles that the authority refuses a remote origin, and it does not settle how the origin
    // is determined at runtime, which is a separate question about header trust.
    let mut context = context();
    let authority = context.runtime_authority_mut();
    let remote = Origin::Remote {
        url: "https://open.spotify.com/embed/track/x".parse().expect("url"),
    };
    let mut leaked = Vec::new();
    for entry in policy() {
        if allowed(&authority, &entry.command, "main", &remote) {
            leaked.push(entry.command.clone());
        }
    }
    assert!(
        leaked.is_empty(),
        "these commands resolve for a remote origin: {leaked:#?}",
    );
}

#[test]
fn an_unregistered_command_name_is_denied_everywhere() {
    // Guards the guard. If `resolve_access` were somehow answering yes to everything, every
    // assertion above would pass while proving nothing at all.
    let mut context = context();
    let authority = context.runtime_authority_mut();
    for name in [
        "exfiltrate_vault_secret",
        "get_messages_but_evil",
        "",
        "allow-get-messages",
    ] {
        for label in KNOWN_SURFACES {
            assert!(
                !allowed(&authority, name, label, &Origin::Local),
                "the authority resolved an unregistered command {name:?} for {label}",
            );
        }
    }
}
