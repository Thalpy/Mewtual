# Pending repository-wide actions

Actions that are deliberately waiting for a trigger. When the trigger happens, do the action, then
delete its entry here (and any pointer to it).

## Upgrade Rust from 1.89.0 to 1.91.0

Flagged 24 October 2026.

**Trigger:** both of these have been merged into `main`:

- PR #54, "Chat method redesign" (branch `Chat-method-redesign`)
- PR #26, "Creative studio funtimes!" (branch `Create-suite-2`)

**Why it waits:**

- The toolchain bump causes a full rebuild of every target and CI cache.
- It is likely to trigger new Clippy findings. CI runs Clippy with `-D warnings`.
- Landing it under two large in-flight branches would push that churn and its merge conflicts
  into them.

**What 1.91 unlocks:**

- **openmls 0.9.x.** It fixes openmls security advisories that the 0.8 line never received,
  including two pre-authentication denial-of-service issues fixed only in 0.9.0. It also lets three
  libcrux advisory ignores in `deny.toml` go. Stored state written by 0.8.1 has loaded under 0.9.2
  in probes; the remaining compatibility fixtures are listed in
  `docs/design-crypto-bigint-upgrade.md` on the `crypto-bigint-upgrade` branch.
- **`portmapper` 0.19.** `crates/catcoms-net/Cargo.toml` pins `=0.18.0` only because 0.19 requires
  Rust 1.91.

**What 1.91 does NOT do:** clear RUSTSEC-2026-0387 (constant-time failures in `crypto-bigint`
before 0.7.6).

- `openmls_rust_crypto` 0.6.0 and `openmls_basic_credential` 0.6.0 still depend on `p256 ^0.13`, so
  `crypto-bigint` 0.5.5 stays. A build of openmls 0.9.2 still reports the advisory.
- The only openmls backend on `p256` 0.14 is on upstream openmls `main`. It is unreleased and
  declares Rust 1.94. If that backend is what the bump should enable, target 1.94 instead. That is
  an open owner decision.
- How the advisory is handled until then is decided separately, in
  `docs/design-crypto-bigint-upgrade.md`.

**What to change, in one commit, before any dependency upgrade:**

1. `rust-toolchain.toml`: set `channel = "1.91.0"`, and remove the pointer comment there.
2. `rust-version = "1.91"` in the workspace `Cargo.toml` and in `apps/desktop/src-tauri/Cargo.toml`.
   The latter is a separate Cargo workspace with its own field.
3. Every `dtolnay/rust-toolchain@1.89.0` in `.github/workflows/`, including `release.yml`.
4. `docker/linux-test.Dockerfile`: change the base image from `rust:1.89-bookworm` to the 1.91
   image.
5. Docs that state the toolchain version.
6. **Re-audit `apps/desktop/src-tauri/src/voice_handoff/context.rs`.** Its `ArcAllocation` models
   the pinned Rust 1.89 `ArcInner` layout (two atomic counters, then the value) for a memory bound.
   Check it against the new toolchain's `library/alloc/src/sync.rs` and update the source link.

**Then:**

- Fix new Clippy findings for the root workspace and for `apps/desktop/src-tauri`, which is a
  separate Cargo workspace.
- Run `cargo fmt` for both.
- Re-check the anchors of the studio `check-*.py` mutation harnesses, because code moved by lint
  fixes can silently break them.
- Expect one slow, cold CI round.
- As separate follow-up PRs: openmls 0.9.x with its compatibility fixtures, and `portmapper` 0.19.

**Watch for:**

- Since Rust 1.90, `x86_64-unknown-linux-gnu` links with LLD by default. If the Linux release build
  objects, opt out with `-C linker-features=-lld`.
- `x86_64-apple-darwin` has been tier 2 since 1.90.
