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

**Why it is needed:** RUSTSEC-2026-0387 (constant-time failures in `crypto-bigint` before 0.7.6)
now fails `cargo deny` on every branch.

- **Where it reaches us:** we get `crypto-bigint` 0.5.5 through `elliptic-curve` 0.13 and
  `p256`/`p384`/`k256`. Those come from `hpke-rs-rust-crypto` 0.6 and `openmls_rust_crypto` 0.5,
  plus a direct `p256 = "=0.13.2"` in `apps/desktop/src-tauri`.
- **The fix needs Rust 1.91:** the fixed chain (`openmls` 0.9, `openmls_rust_crypto` 0.6,
  `p256` 0.14) needs Rust 1.91, because `openmls_rust_crypto` 0.6.0 declares
  `rust-version = "1.91.0"`.
- **Investigation:** the branch `crypto-bigint-upgrade` is looking for the smallest path, and for
  the compatibility consequences for stored MLS state.

**What to change, in one commit, before any dependency upgrade:**

1. `rust-toolchain.toml`: set `channel = "1.91.0"`, and remove the pointer comment there.
2. The workspace `Cargo.toml`: set `rust-version = "1.91"`.
3. Every `dtolnay/rust-toolchain@1.89.0` in `.github/workflows/`, including `release.yml`.
4. `docker/linux-test.Dockerfile`: change the base image from `rust:1.89-bookworm` to the 1.91
   image.
5. Docs that state the toolchain version.

**Then:**

- Fix new Clippy findings for the root workspace and for `apps/desktop/src-tauri`, which is a
  separate Cargo workspace.
- Run `cargo fmt` for both.
- Re-check the anchors of the studio `check-*.py` mutation harnesses, because code moved by lint
  fixes can silently break them.
- Expect one slow, cold CI round.

**Watch for:**

- Since Rust 1.90, `x86_64-unknown-linux-gnu` links with LLD by default. If the Linux release build
  objects, opt out with `-C linker-features=-lld`.
- `x86_64-apple-darwin` has been tier 2 since 1.90.
