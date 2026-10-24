# Design: RUSTSEC-2026-0387 (crypto-bigint timing variability)

Status: design for owner decision (Phase 1). Nothing here is implemented. Base commit `2f08bde0`
(`origin/chat-update-implementation`, merge of PR #50). Investigated 2026-10-24 on Rust 1.89.0.
Owner constraint received during the investigation: the Rust 1.91 bump waits until PR #54 and
PR #26 are merged (`docs/PENDING-ACTIONS.md` on `main`, commit `e6f1d13e`).

## 0. Headline

1. **No released upgrade removes `crypto-bigint` 0.5.5 on Rust 1.89, and none does on 1.91
   either.** Every released `openmls_rust_crypto` (0.5.0, 0.5.1, 0.6.0) and
   `openmls_basic_credential` (0.5.0, 0.6.0) depends directly on `p256 ^0.13`. `p256 0.13` means
   `elliptic-curve 0.13`, which means `crypto-bigint ^0.5`. A probe built openmls 0.9.2 +
   `openmls_rust_crypto` 0.6.0 and `cargo deny` still reports RUSTSEC-2026-0387 on it.
2. **`docs/PENDING-ACTIONS.md` on `main` rests on a wrong premise.** It says the fixed chain
   "(`openmls` 0.9, `openmls_rust_crypto` 0.6, `p256` 0.14) needs Rust 1.91". `openmls_rust_crypto`
   0.6.0 is on `p256 0.13`, not 0.14. The first openmls backend on `p256 0.14` exists only on
   upstream `main`, unreleased, with workspace MSRV **1.94**. A 1.91 bump therefore unlocks no
   fix for this advisory. It may still be worth doing for other reasons.
3. **A clearing path on 1.89 exists only by owning code:** a Mewtual single-suite OpenMLS crypto
   provider and signer that drop `openmls_rust_crypto`/`openmls_basic_credential` (option B), or a
   vendored fork of upstream (option F).
4. **The one place that puts secrets through crypto-bigint 0.5.5 is the native voice bootstrap**
   (`p256 = "=0.13.2"` ECDH). It is reachable today through the main-window IPC command
   `setup_voice_channel`, although no UI flow calls it yet. Moving it to `p256 0.14.0` works on
   1.89 and reproduces the committed Rust/WebCrypto vectors exactly (section 4).
5. **The defects fixed in 0.7.6 do not appear to reach those native secrets** (section 3): the
   fixed code paths are absent from 0.5.5 or not called by p256 0.13 ECDH. An x86-64 assembly
   spot check (Windows and Linux targets, Rust 1.89 release builds) of the 0.5.5 functions that
   do see the secret found no secret-dependent branch.
   That is evidence, not a constant-time proof.
6. **State created under openmls 0.8.1 loads and keeps working under 0.9.2, and back**, for our
   configuration. A two-version probe measured this, including a PendingCommit, the stored
   signer, wire traffic in both directions, and downgrading that state to 0.8.1 (section 5.3).
   State *created* under 0.9.2 and read by 0.8.1, and the production staged-commit shapes, are
   still unmeasured; they become fixture requirements for the upgrade PR. The eventual upgrade
   still needs a backend release on `p256 0.14`.

Recommended sequence (section 7):

* **Now, on 1.89 (this PR):** port the native bootstrap to `p256 0.14`. Add a reasoned
  `deny.toml` ignore for the MLS backend's leftover 0.5.5, fenced by `[bans]` so that the ignore
  cannot hide a new *dependency* path to the affected crates. New calls inside `catcoms-mls` are
  invisible to bans; review triggers and MLS refusal tests cover those (section 8). Gate the
  native workspace's bans in CI.
* **After the toolchain bump:** openmls 0.9.x is an optional, storage-compatible step. It
  retires the three libcrux ignores but **not** 0387.
* **Exit:** remove the ignore when an openmls backend release on `p256 >= 0.14` is adoptable
  (expect Rust >= 1.94). Alternatively, adopt option B at any time to clear 0387 with no
  exception on 1.89. That is a crypto-backend decision for the owner.

## 1. Looking for a Rust 1.89 path

### 1.1 Where 0.5.5 comes from

The same inclusion graph appears in both workspaces (`cargo deny`, `cargo tree --target all -i`):

```
crypto-bigint 0.5.5 <- elliptic-curve 0.13.8 <- { ecdsa 0.16.9, p256 0.13.2, p384 0.13.1,
                                                  k256 0.13.4, primeorder 0.13.6 }
p256 0.13.2   <- openmls_rust_crypto 0.5.1, openmls_basic_credential 0.5.0,
                 hpke-rs-rust-crypto 0.6.1, mewtual-desktop (native only, direct)
p384, k256    <- hpke-rs-rust-crypto 0.6.1
openmls_rust_crypto / openmls_basic_credential <- catcoms-mls <- catcoms-{app,replication,sync}
```

No other crate uses `elliptic-curve`. The fix `crypto-bigint` 0.7.6 declares rust-version 1.87.
0.7.7, published 2026-10-24 with the same MSRV, is what a fresh resolve now selects.

### 1.2 Every candidate, checked against the crates.io index (2026-10-24)

| Question | Answer | Evidence |
|---|---|---|
| A patch release on the 0.8 line? | No | `openmls` 0.8.2 and 0.8.3 exist, but they only depend on `openmls_traits 0.5`; the matching backends are `openmls_rust_crypto` 0.5.0/0.5.1 and `openmls_basic_credential` 0.5.0, all on `p256 ^0.13`. (`openmls`'s own dependency on the backend is optional, `test-utils`.) |
| A newer `openmls_rust_crypto` 0.5.x? | None exists | Only 0.5.0 and 0.5.1. |
| An `hpke-rs-rust-crypto` usable with `openmls_rust_crypto` 0.5? | No | 0.5.1 requires `hpke-rs-rust-crypto ^0.6.0` directly, which allows only 0.6.0 and 0.6.1, both on `p256`/`p384`/`k256 ^0.13` (non-optional). It also depends on `p256 ^0.13` itself, so even a different HPKE provider would leave it. |
| Turn the NIST curves off by feature? | No | Non-optional in `openmls_rust_crypto` 0.5/0.6, `openmls_basic_credential` 0.5/0.6 and `hpke-rs-rust-crypto` 0.6/0.7. |
| The 0.9 line (Rust 1.91 declared)? | No | `openmls_rust_crypto` 0.6.0 and `openmls_basic_credential` 0.6.0 depend on `p256 ^0.13` (and `p384 ^0.13`); `hpke-rs` 0.7 brings `hpke-rs-rust-crypto` 0.7 on `p256 ^0.13`. Probe `probe-compat` (openmls 0.9.2 + rust_crypto 0.6.0): `cargo deny` reports `crypto-bigint 0.5.5 RUSTSEC-2026-0387`. |
| `hpke-rs-rust-crypto` 0.8.0 (on `p256 0.14`)? | Not usable by any released backend | Implements `hpke-rs-crypto` 0.8; `openmls_rust_crypto` needs 0.6 (for 0.5.x) or 0.7 (for 0.6.0). |
| `openmls_libcrux_crypto` 0.3.1 (openmls 0.8)? | Removes 0.5.5, adds worse findings | Probe: `cargo deny` reports 11 libcrux advisories, 8 of them new and reachable because libcrux would run every MLS operation. They include RUSTSEC-2026-0124 (ChaCha20-Poly1305 panic; our AEAD), 2026-0073 (Poly1305 panic), 2026-0075 (Ed25519 all-zero key on RNG failure; our signature scheme), 2026-0209/0210/0211 (AES-GCM), 2026-0330/0331 (hybrid KEM). It also still needs a replacement for `openmls_basic_credential`. |
| Upstream `openmls` `main`? | Fixed, unreleased | `openmls_rust_crypto` and `basic_credential` on `p256 0.14`/`p384 0.14`/`hpke-rs 0.8`; workspace `rust-version = "1.94.0"` (CHANGELOG #2234). |

### 1.3 The two ways to drop the NIST-curve code on 1.89

* **Option B, a Mewtual-owned provider.** Implement `openmls_traits::OpenMlsCrypto` and
  `OpenMlsRand` for exactly `MLS_128_DHKEMX25519_CHACHA20POLY1305_SHA256_Ed25519` on RustCrypto
  primitives already in our graph: `x25519-dalek` 2, `chacha20poly1305` 0.10, `hkdf` 0.12,
  `hmac` 0.12, `sha2` 0.10 and `ed25519-dalek` 2 (`verify_strict`, as `openmls_rust_crypto`
  does). Keep `openmls_memory_storage` 0.5 as the storage provider. Replace
  `openmls_basic_credential::SignatureKeyPair` with an Ed25519-only signer whose stored form is
  byte-identical (section 5.2).
  * HPKE runs through the `hpke-rs` 0.6 core with our own X25519-only `hpke_rs_crypto::HpkeCrypto`
    impl (14 trait methods, about 120 lines).
  * Total size: about 200 lines of provider, 80 of signer, plus KATs and fixtures. Compare
    `openmls_rust_crypto` 0.5.1 `provider.rs`, which is 459 lines for five suites.
  * Blast radius: `catcoms-mls` only. Only `device.rs` and `persist.rs` name the provider type.
  * It is new code in the trusted base.
  * B8, the same on `hpke-rs` 0.8, fails on 1.89. It needs `libcrux-sha3 =0.0.11`, which needs
    `libcrux-platform =0.0.4`, which does not compile on Rust 1.89: `E0133: call to unsafe function
    __cpuid`. It assumes rust-lang/stdarch#1935 (merged 2025-11-30, so stable 1.93 or 1.94) even
    though libcrux declares 1.89.
* **Option F, a vendored fork.** Port `openmls_rust_crypto` 0.5.1, `openmls_basic_credential`
  0.5.0 and `hpke-rs-rust-crypto` 0.6.1 to `p256 0.14`/`ecdsa 0.17`/`rand_core 0.10` under
  `[patch.crates-io]`. Rejected: it means keeping a private fork of upstream crypto code alive
  just to delete code we never execute.

## 2. Reachability

### 2.1 Native voice bootstrap: secret, reachable from the main window

`apps/desktop/src-tauri/src/voice_handoff/channel/bootstrap.rs`:

* `Prepared::compute` does four things in order:
  * validates the selected browser point: `point[0] == 4`, then `PublicKey::from_sec1_bytes`;
  * draws `EphemeralSecret::random(&mut BorrowedRng(rng))` from the injected `catcoms-rt` RNG;
  * publishes the native point;
  * calls `agree`.
* `agree` runs `diffie_hellman`, then `Hkdf::<Sha256>::new(Some(hash), shared.raw_secret_bytes())`,
  then expands `"Mewtual native voice bootstrap seed v1"` into the seed for `Keys::new`.
* Test users: `bootstrap/tests.rs` (vectors, layout guard) and `service/tests.rs` (simulated
  browser using `SecretKey::from_slice` and `p256::ecdh::diffie_hellman`).

**Secret values:** the native ephemeral scalar, every intermediate point of the two scalar
multiplications (`[k]G` for the public key, `[k]B` for ECDH), the shared x-coordinate, the HKDF seed
and the framing keys. **Public values:** both points, the transcript and its hash.

**Reachability (corrected after review):** `setup_voice_channel` is registered
(`src-tauri/src/lib.rs:18051`) and allowed for the main window
(`security/command-policy.json:147`). `commands.rs` reaches `bootstrap::ensure` and then
`Prepared::compute`. `Host::prepare` binds a loopback TCP listener (`service.rs:110`). INTERFACES
("App-owned public bootstrap and status channel") and THREAT-MODEL ("The main-window ACL alone
exposes these fixed public bootstrap commands") already describe this, superseding the older
"disabled candidate" text in THREAT-MODEL "Internal ECDH bootstrap boundary" and HANDOVER. No
production frontend module imports `voice-ipc-native.ts` yet, so no UI flow triggers it. Any
main-window script can.

The two ends are one app instance's native process and its own WebView, talking over loopback.
They always ship together, so no cross-version peer exists on this channel. The only
compatibility property is Rust/WebCrypto agreement, which `voice_handoff/channel/vectors.json`
pins for both the native suite and the TypeScript suites.

The `catcoms-sync` comments that the WebView keeps its P-256 private key are true for the remote
V2 agreement:

* `catcoms-sync` has no `p256` dependency;
* it checks only the 65-byte `0x04` structure and the key reference (`voice_v2.rs:376`);
* WebCrypto validates the curve point (`voice-crypto.ts:1555`, `:1849`), with non-extractable keys
  (`:605`, `:1715`).

As a statement about native code as a whole, though, it is incomplete: the local bootstrap holds a
native P-256 secret.

### 2.2 MLS backend: compiled, but no secret reaches the NIST-curve code

`openmls_rust_crypto` advertises a P-256 suite: `supports()` lists
`MLS_128_DHKEMP256_AES128GCM_SHA256_P256`. `openmls_basic_credential` can make ECDSA keys. A
secret P-256/P-384/K-256 operation would need one of three things:

* a P-256 signer. `device.rs` only calls `SignatureKeyPair::new(CIPHERSUITE.signature_algorithm())`,
  which is Ed25519.
* a P-256 HPKE private key. KeyPackages and groups use only `CIPHERSUITE` (`config.rs`).
* an HPKE seal to a foreign-suite key. That requires adding such a KeyPackage to one of our groups.

The enforcement points, read in openmls 0.8.1:

* `config.rs`: one pinned suite, and capabilities advertise only that suite. This is the documented
  downgrade floor.
* `group/public_group/validation.rs:386` (ValSem105) rejects an Add whose KeyPackage suite differs
  from the group's.
  * Send side: `commit_builder.rs:523` runs it before `compute_path` (`:606`) and
    `encrypt_group_secrets` (`:803`).
  * Receive side: `staged_commit.rs:176` runs it, and `messages/proposals_in.rs:179` repeats it
    for inbound proposals.
* `group/mls_group/creation.rs:165` returns `CiphersuiteMismatch` when the Welcome's suite differs
  from our KeyPackage's. That check runs before the `GroupSecrets` HPKE open with our (X25519)
  init key.
  * The GroupInfo check at `:225` runs after that open. It is harmless, because the open already
    used the suite matched to the KeyPackage.
* There are no external commits, GroupInfo joins, direct `hpke_*` calls or second
  `SignatureScheme` anywhere in `crates/` (the review checked this).

What a remote peer can drive is public-key work: `MlsDevice::parse_key_package` verifies a
foreign P-256 KeyPackage's ECDSA signature against the peer's own key. Its timing reveals nothing
secret. `hpke-rs-rust-crypto` never selects `k256` or `p384` for any suite we accept.

This argument holds **only while** `config.rs` pins one X25519/Ed25519 suite and the openmls
version keeps those checks. Section 8 turns that into review triggers and bans.

### 2.3 What `cargo deny` checks, and what other scanners see

`cargo deny` evaluates the feature-resolved build graph, not raw `Cargo.lock` rows. Cargo also
writes into the lock any optional dependency that is named through a weak feature (`dep?/feat`)
(rust-lang/cargo#10801). For example, `hpke-rs` 0.6.1's `std` feature names
`hpke-rs-rust-crypto?/std`, which is why today's root lock already lists uncompiled libcrux
crates. The libcrux probe's lock contained `crypto-bigint 0.5.5`, and `cargo deny` did not report
it.

Consequences:

* Under option B, `cargo deny` passes, but 0.5.5 stays in both locks as an uncompiled row.
* Under option A, `cargo deny` passes only through the ignore.
* Lock-file scanners such as `cargo audit` and GitHub dependency alerts flag 0.5.5 under both A
  and B.

So when we say "cleared", we mean cleared for `cargo deny`.

## 3. Do the defects fixed in 0.7.6 reach the native secrets?

The advisory gives `patched = [">= 0.7.6"]` and no `unaffected` range, so the database treats every
0.5.x release as affected. Its security fixes (crypto-bigint PR #1365, the v0.7.6 release) are:

| Fix | What it changed | In 0.5.5? |
|---|---|---|
| #1358 `borrowing_sub` | Removes a branch introduced by the #1029 AArch64 workaround (an `overflowing_sub` form) | No. 0.5.5 `Limb::sbb` is wide-word arithmetic (`(a as u128).wrapping_sub(b + borrow)`) and predates #1029. See the caveat below the table |
| #1360 CTZ, `div_rem`, `nlimbs`, Montgomery mul | Zero counting, bit-length rounding, division, `MontyForm` multiplication | Not called by p256 0.13 ECDH. p256 0.13 field multiplication is its own `fe_mul` (`adc/mac/sbb` in `arithmetic/util.rs`) |
| #1311 `UintRef` eq | `UintRef` | Type does not exist in 0.5 |
| #1355 `BoxedMontyForm::invert` | Boxed Montgomery inversion | Type does not exist in 0.5 |
| #1350/#1359 `ctutils` 0.4.4 | The advisory's "updates `ctutils` to include its constant-time fixes" | Not applicable: 0.5.5 has no `ctutils`. Its `CtChoice` to `subtle::Choice` conversion goes through `subtle::black_box`, and the assembly check below covers it |

Caveat on the #1358 row. The patch replaces wide-word casts because "rustc 1.87 through 1.91
incorrectly optimize some WideWord bit arithmetic" (rust-lang/rust#149522: an AArch64 Linux
miscompile of `ct_lt` loops, a correctness bug fixed in LLVM). Our 1.89 is in that range, and both
0.5.5 `sbb` and p256 0.13's own `util.rs` `adc/sbb/mac` use the same `u128` pattern. The x86-64
vectors and the assembly check pass. Record this with the existing aarch64 revisit for
RUSTSEC-2026-0212: before any ARM build, re-check it or make sure the native path is already on
0.7.x (which this PR does).

The 0.5.5 functions that p256 0.13 ECDH does feed with secret-dependent data are:

* `Uint::conditional_select`: every secret-indexed window lookup selects a `ProjectivePoint`
  through three `FieldElement` selects (`primeorder-0.13.6/src/projective.rs:132-137`; the field
  type wraps `U256`).
* `Uint::ct_eq` / `ct_is_nonzero` on the secret-dependent Z coordinate in `invert`/`to_affine`
  (`primeorder field.rs:192`, `:327`).
* `Uint::ct_lt` via `sbb` and `from_be_byte_array`: the range check in `Scalar::from_repr` on each
  candidate scalar.
* `is_zero`: the `NonZeroScalar` check.

`add_mod`/`sub_mod` are used only by ECDSA. Note that the advisory's "zero counting" means
leading/trailing-zero counts, not a zero test.

**Spot check** (`scratchpad/probe-ct`): Rust 1.89, x86-64 baseline, `--release`, assembly emitted
for every crate.

* These contain no conditional jumps: the non-inlined wrappers for `U256::conditional_select`,
  `ct_eq`, `ct_lt`, `Scalar::from_repr`, `ProjectivePoint::conditional_select`, the full
  `diffie_hellman` and `PublicKey::from_secret_scalar`.
* `FieldElement::invert` likewise; its zero test compiles to `cmp eax, 1; sbb rcx, rcx`.
* The instantiated `ProjectivePoint * Scalar` has four conditional jumps. All test public loop
  counters: table index parity (even entries double, odd entries add) and the window position.
  The secret digit becomes a mask (`dec ecx; shr ecx, 8; and cl, 1`) passed to the
  conditional-select calls.
* `invert_unchecked` loops are `dec r15; jne`, a fixed exponent.
* Rejection sampling branches by design: `Scalar::random` loops until `from_repr` accepts a
  candidate, and `NonZeroScalar::random` until it is non-zero. Those branches reveal only that a
  discarded candidate was out of range; the accepted scalar does not influence them.

The same probe built for `x86_64-unknown-linux-gnu` (WSL Ubuntu, Rust 1.89, `--release`) gives
the same result. The wrappers and `FieldElement::invert` again have no conditional jumps; the
multiplication's four jumps again test only `r13` parity, the table counter and the window
position; `invert_unchecked` again loops on `dec r12; jne`.

What this does not cover: BINSEC/`cargo checkct`, the real binary's inlining and codegen units,
aarch64 and other CPUs, and microarchitectural channels other than branches.

**Conclusion for the interim ignore:** the named defects do not reach the native secret path, and
the spot check found no compiler-introduced branch there. The ignore's comment records this
analysis. The plan does not rely on it to land the ignore ahead of the port, though: section 8
lands the ignore, the fence and the port in one commit, so the only secret path a local script
can drive is on 0.7.x from the first revision that carries the ignore.

## 4. Porting the native bootstrap to p256 0.14

Probe `scratchpad/probe-native` (`cargo +1.89.0 run` on Windows, and again on Linux under WSL;
`p256 = "0.14.0"` with `default-features = false, features = ["ecdh"]`, and the native crate's
`hkdf =0.12.4` and `sha2 =0.10.9`):

* Both `vectors.json` bootstrap cases (`ordinary`, `leading-zero-shared`) match at every step:
  native scalar to native point, browser point to shared secret (with the leading zero kept),
  shared secret to HKDF seed, and the static-key `diffie_hellman` that `service/tests.rs` uses.
* The scalar draw is still one 32-byte fill, rejected and redrawn when it is `>= n`
  (`p256 0.14 Scalar::try_random`, the same contract as 0.13). A draw of `n` followed by `1` gives
  two draws and the generator point.
* An all-zero `04||0||0` point is refused.

API changes:

* `EphemeralSecret::random` is deprecated. Use `elliptic_curve::Generate::generate_from_rng`.
  * The RNG bound is now `rand_core` **0.10** `CryptoRng + ?Sized`, and `catcoms-rt` re-exports
    `rand_core` 0.6.
  * So `BorrowedRng` becomes a 0.6 to 0.10 adapter, implementing `TryRng<Error = Infallible>` and
    `TryCryptoRng` by delegating to the injected RNG's `fill_bytes`. It adds a direct
    `rand_core = "=0.10.1"` dependency, which is already in the native lock.
  * Today's semantics are kept: a failing OS RNG panics, Pending precedes the draw, and the
    existing unwinding-RNG test (`voice_handoff/bootstrap/tests.rs:418`) shows that the panic closes
    the ticket.
  * The adapter must never seed a second RNG, and must never call `Generate::generate()`, which
    uses `SysRng` when `crypto-common`'s `getrandom` feature is on.
* `elliptic_curve::sec1::ToEncodedPoint` is deprecated; use `ToSec1Point::to_sec1_point(false)`.
* These keep their shape: `PublicKey::from_sec1_bytes`, `SecretKey::from_slice`,
  `to_nonzero_scalar`, `as_affine`, `ecdh::diffie_hellman` and `SharedSecret::raw_secret_bytes`.
  `FieldBytes` is now a `hybrid_array::Array` and still `Deref<[u8]>`.
* `precomputed-tables` stays off: our edge disables default features, and nothing else in the
  native graph depends on `p256 0.14`. Both multiplications therefore use `primeorder 0.14
  ProjectivePoint::mul`: an 8-point `LookupTable [P..8P]`, a `Radix16Decomposition` and `lincomb`.
  The layout guard (`measured_native_setup_..._fit_ordinary_work_charge`) cites
  "primeorder0.13.6 ... 16-entry table". Its comment and point budget must be re-derived. If
  anything later unifies `precomputed-tables`, the algorithm changes silently, so the guard
  comment should name that feature.
* `EphemeralSecret` and `SharedSecret` still zeroize on drop. The p256 0.14 README repeats the
  "never independently audited" warning, so the THREAT-MODEL limits are unchanged.

Native lock delta (exploratory resolve, reverted):

* Added: `p256 0.14.0`, `elliptic-curve 0.14.1`, `crypto-bigint 0.7.7`, `primeorder 0.14.0`,
  `primefield 0.14.0`, `wnaf 0.14.1`, `ff 0.14.0`, `group 0.14.0`, `sec1 0.8.1`, `der 0.8.2`,
  `base16ct 1.0.0`, `ctutils 0.4.5`, `cmov 0.5.4`, `cpubits 0.1.1`, `hkdf 0.13.0`, `hmac 0.13.0`.
* Changed: `hybrid-array` moves from 0.4.12 to 0.4.15.
* All are MIT/Apache-2.0, with MSRVs 1.85 to 1.87 and no build scripts.
* `cmov` uses Rust inline `asm!` (x86 `cmov`, aarch64 `csel`) for constant-time selection. That is
  not a `cc`-compiled blob; it is the hardening the fix relies on.
* `p256 0.13.2` stays in the native graph for openmls.

## 5. Compatibility

### 5.1 Option A

* Wire and derivation do not change: transcript version/suite bytes stay `[1, 1]`, the point
  encoding, the fixed-width x-coordinate and the HKDF are the same, and section 4 proves it.
* Nothing is persisted.
* Root sources and the root lock do not change; only `deny.toml` and CI do.
* A tester sees nothing.

### 5.2 Option B

MLS is RFC 9420. HKDF, `DeriveKeyPair` (RFC 9180) and Ed25519 (RFC 8032) are bit-exact, so an
RFC-correct provider derives the same keys and the same signatures as `openmls_rust_crypto`.
Mixed-version groups must interoperate.

The sealed MLS snapshot (`persist.rs`) wraps the `openmls_memory_storage` 0.5 key/value map
(`serde_json` values) in our own codec. B keeps that map, so every group, epoch, tree, KeyPackage
and HPKE-key entry keeps its bytes. Two kinds of entry depend on the provider:

* **The signer.**
  * Key: the `SignatureKeyPair` label, then JSON of `StorageId { value: public ||
    b"RustCryptoSignatureKey" || 0x0807 }`, then version `1`.
  * Value: `{"private": [32-byte seed], "public": [...], "signature_scheme": "ED25519"}`.
  * A mismatched key makes `restore_server` fail with "signer missing". A mismatched value shape
    **panics** inside `openmls_memory_storage` (`lib.rs:501`, `serde_json::from_slice(..).unwrap()`).
  * So a byte-equality fixture must gate B.
* **HPKE private keys**, which are raw RFC 9180 X25519 bytes, clamped at use.

If either kind is wrong, the tester sees a server that cannot be restored, or one that fails on the
next Welcome or commit, until they rejoin it.

### 5.3 openmls 0.8.1 to 0.9.2 (the eventual upgrade): measured

Two scratch crates, needed because the two lines cannot share one lock (`hax-lib =0.3.6` against
`^0.3.7`):

* `probe-compat-writer`: openmls 0.8.1 + `openmls_rust_crypto` 0.5.1 + `basic_credential` 0.5.0,
  the exact alpha stack.
* `probe-compat`: openmls 0.9.2 + 0.6.0 + 0.6.0. It builds on Rust 1.89 with
  `--ignore-rust-version`. That is not usable in CI, but enough to measure.

Both use our `config.rs` settings: suite 3, pure-ciphertext wire format, ratchet-tree extension,
locked capabilities, Basic credentials. Data passes as MemoryStorage maps, the same maps
`persist.rs` serializes.

All of these passed:

* **0.8.1 storage, read by 0.9.2:**
  * Alice's snapshot holds a **PendingCommit** (a staged self-update). 0.9.2 `MlsGroup::load`
    finds the group; `SignatureKeyPair::read` (0.6.0) reads the 0.8.1 signer; the epoch and the
    exporter match; `pending_commit()` is present; merging it gives the exporter 0.8.1 computed.
  * Bob's plain snapshot loads, decrypts a 0.8.1 application message, merges a 0.8.1 commit and
    matches the exporter.
* **0.9.2 wire output, consumed by 0.8.1:** 0.8.1 Bob decrypts a 0.9.2 application message, merges
  a 0.9.2 Add commit and agrees on the exporter.
* **A 0.9.2 Welcome, joined by 0.8.1:** 0.8.1 Carol, holding KeyPackage material stored by 0.8.1
  that the migrated Alice added, joins and decrypts.
* **Downgrade:** 0.8.1 loads the storage 0.9.2 wrote after two commits, derives the same exporter,
  and sends a message that 0.8.1 Bob and Carol decrypt.
* `CURRENT_VERSION` is 1 in both `openmls_traits` lines. `basic_credential` 0.6.0 deliberately
  keeps the `Vec<u8>` serde form for `private` (`secret_bytes_as_vec`).
* Break-it check: renaming the stored signer's `private` field in the 0.8.1 map makes the 0.9.2
  restore panic (`openmls_memory_storage-0.6.0/src/lib.rs:513`, `missing field private`). So the
  probe really exercises the signer entry, and a shape mismatch is a panic, not an error.

The stored entry kinds exercised were `ConfirmationTag`, `EpochKeyPairs`, `EpochSecrets`,
`GroupContext`, `GroupState`, `InterimTranscriptHash`, `MessageSecrets`, `MlsGroupJoinConfig`,
`OwnLeafNodeIndex`, `ResumptionPsk`, `SignatureKeyPair` and `Tree`, plus KeyPackage and
encryption-key entries.

Not covered. The first four are fixture requirements for the upgrade PR:

* (i) a 0.8.1 inviter's Welcome joined by a 0.9.2 joiner;
* (ii) a KeyPackage generated by 0.9.2 (with 0.9 default capabilities) parsed and added by 0.8.1;
* (iii) state *created* under 0.9.2 and read by 0.8.1: a `basic_credential` 0.6.0 signer entry,
  a group and GroupContext created by 0.9.2, a 0.9.2 KeyPackage bundle, and a PendingCommit that
  0.9.2 wrote. The downgrade above only re-reads state that was born under 0.8.1;
* (iv) the production staged-commit shapes: `stage_add`/`stage_remove` (`group.rs:413-450`) and
  `abort_staged`, which uses `clear_pending_commit` (`group.rs:472-477`). The probe's PendingCommit
  is a self-update;
* states with many past epochs (`MessageSecretsStore` reordering on read, #2197 on main);
* our sealed codec around the map (unchanged by any of this);
* openmls releases after 0.9.2 (`Capabilities` defaults change in #2205 on main, and so do
  `ExportedSecret<T>` return types);
* the 0.9 API changes `catcoms-mls` would absorb.

The 0.9.0 storage-tag change (#2034) targets non-self-describing codecs. `MemoryStorage` uses
`serde_json`, which explains why the measurement passes without the `0-8-1-storage-format`
feature. Promote this probe to a committed fixture test in the upgrade PR.

Side benefit: the 0.9.2 graph reports no `libcrux-sha3`/`libcrux-secrets` advisories
(`hpke-rs 0.7` uses `libcrux-sha3 0.0.10`). The 0.9 step would retire RUSTSEC-2026-0207, 0208 and
0212, but not 0387.

## 6. Toolchain

Options A and B need no change. The released-fix route (E) needs >= 1.94. B8 needs 1.93 or
1.94. The 1.91 bump in `docs/PENDING-ACTIONS.md` is short of every one of them.

Corrections to its checklist:

* Item 2 names only the root `Cargo.toml`. `apps/desktop/src-tauri/Cargo.toml` also pins
  `rust-version = "1.89"`.
* `voice_handoff/context.rs:13` relies on "Pinned Rust1.89 ArcInner is repr(C)" for memory
  accounting. Re-audit it against the new `library/alloc/src/sync.rs`.
* `crates/catcoms-net/Cargo.toml:20` holds a dependency below 0.19 only because 0.19 needs 1.91.
  The bump frees it.
* "The fix needs Rust 1.91" should be removed or corrected (section 0, item 2).

The full set of pins, for whichever bump is chosen:

* `rust-toolchain.toml`;
* the root and native `rust-version`;
* 17 `dtolnay/rust-toolchain@1.89.0` lines: `ci.yml` (2), `release.yml` (2), `two-client.yml` (2),
  `studio-handoff.yml` (3), `studio-overlay.yml` (3), `studio-native.yml`,
  `studio-inspection.yml`, `agent3-repair-{core,runtime,store}.yml`;
* `docker/linux-test.Dockerfile`, the WSL toolchain;
* `README.md`, `docs/LINUX-TESTING.md`, `docs/TWO-CLIENT-TESTING.md`, `docs/DEVELOPMENT-SWEEP.md`.

## 7. Proposed sequence

**Now, Rust 1.89, this PR (option A):**

1. Native port to `p256 =0.14.0` (section 4), plus the regression tests in section 9.
2. A `deny.toml` ignore for RUSTSEC-2026-0387. Its comment records:
   * the native secret path, now on 0.7.x;
   * the leftover MLS 0.5.5 and why no secret reaches it (section 2.2);
   * the section 3 analysis;
   * the exit condition.
3. The fence in section 8, and a CI step that runs `cargo deny` **bans** on the native workspace.
4. Docs:
   * THREAT-MODEL: bootstrap boundary, MLS exception and fence, and corrected reachability;
   * HANDOVER;
   * the native `Cargo.toml` comment;
   * the `VOICE-NATIVE-RESULTS-NEXT.md` source-review pointers, which name 0.13.2 files.
5. Ask the coordinator to correct `docs/PENDING-ACTIONS.md` on `main`. This PR does not touch it.

**After the toolchain bump (separate PR on top of the bump commit):** openmls 0.9.x with
`openmls_rust_crypto` 0.6.0. Section 5.3 shows 0.8.1-born state is compatible; the PR must add
fixtures (i) to (iv) from section 5.3. It retires the libcrux ignores but keeps the 0387 ignore
and fence, and it changes the fence deliberately in two rings:

* ring 2: the `openmls`, `openmls_rust_crypto`, `basic_credential` and `hpke-rs-rust-crypto`
  version bans;
* ring 1: in the 0.9.2 graph, `p384 0.13.1`'s parents become `hpke-rs-rust-crypto` 0.7.0,
  `openmls_basic_credential` 0.6.0 and `openmls_rust_crypto` 0.6.0, and `ecdsa 0.16.9`'s parents
  become `p256` and `p384`. So the `p384` and `ecdsa` entries fire, and section 2.2 must be
  re-read for `basic_credential` 0.6's P-384 ECDSA.

The step is optional; the security case for it is the openmls GHSAs fixed only in 0.9
(section 12).

**Exit:** when an openmls backend release on `p256 >= 0.14` is adoptable, upgrade, then delete the
ignore and the fence; `cargo deny` warns `advisory-not-detected` once the ignore is stale. If the
owner wants 0387 cleared sooner without a toolchain jump, option B on 1.89 is the route, with
section 9's B tests as the gate.

## 8. The fence (deny.toml)

An id ignore is advisory-wide. Scoping it to one crate version does not work: in cargo-deny 0.20.2
`crate = "crypto-bigint@0.5.5"` under `[advisories] ignore` applies only to yanked crates, and the
review saw 0387 still reported. So the ignore must be fenced with bans. The proposed entries all
use the existing `name`/`version` form:

```toml
# Ring 0: crypto-bigint only inside curve crates, never an affected 0.6/0.7 build.
{ name = "crypto-bigint", wrappers = ["elliptic-curve", "primefield"] },   # version-less: covers prereleases
{ name = "crypto-bigint", version = ">=0.5.0-0, <0.5.0" },
{ name = "crypto-bigint", version = ">=0.6.0-0, <0.6.0" },
{ name = "crypto-bigint", version = ">=0.7.0-0, <0.7.0" },
{ name = "crypto-bigint", version = ">=0.6.0, <0.7.6" },
# Ring 1: the 0.13 curve family only under the reviewed wrappers.
{ name = "elliptic-curve", version = "<0.14", wrappers = ["p256", "p384", "k256", "ecdsa", "primeorder"] },
{ name = "p256", version = "<0.14", wrappers = ["openmls_rust_crypto", "openmls_basic_credential", "hpke-rs-rust-crypto"] },
{ name = "p384", version = "<0.14", wrappers = ["hpke-rs-rust-crypto"] },
{ name = "k256", version = "<0.14", wrappers = ["hpke-rs-rust-crypto"] },
{ name = "ecdsa", version = "<0.17", wrappers = ["p256"] },
{ name = "primeorder", version = "<0.14", wrappers = ["p256", "p384"] },
# Ring 2: those wrappers only under catcoms-mls, at the reviewed versions.
{ name = "openmls_rust_crypto", wrappers = ["catcoms-mls"] },
{ name = "openmls_basic_credential", wrappers = ["catcoms-mls"] },
{ name = "hpke-rs-rust-crypto", wrappers = ["openmls_rust_crypto"] },
{ name = "openmls_rust_crypto", version = "<0.5.1" }, { name = "openmls_rust_crypto", version = ">0.5.1" },
{ name = "openmls_basic_credential", version = "<0.5.0" }, { name = "openmls_basic_credential", version = ">0.5.0" },
{ name = "hpke-rs-rust-crypto", version = "<0.6.1" }, { name = "hpke-rs-rust-crypto", version = ">0.6.1" },
# openmls itself: section 2.2 reads its 0.8.1 enforcement code; any move must re-read it.
{ name = "openmls", version = "<0.8.1" }, { name = "openmls", version = ">0.8.1" },
```

Each entry needs a `reason`, left out above for width. The `openmls` pin means the separate 0.8.3 GHSA PR (section 12) edits the fence on purpose.

What was verified (cargo-deny 0.20.2, a scratch copy of `deny.toml` with exactly these 22
entries added):

* **Root:** bans pass. The only warning is `unused-wrapper` for `primefield`, which joins the graph
  only after the native port.
* **Today's native graph fails** with exactly one error, `crate 'p256 = 0.13.2' is explicitly
  banned`, because the direct parent `mewtual-desktop` is not a wrapper. That failure is the
  regression the fence exists for, and the port removes it.
* **Ring 2 and the prerelease forms:** the reviewer checked these separately (`scratchpad/review/`).
* **cargo-deny 0.20.2 traps found:**
  * ordinary ranges skip prereleases;
  * `crate = "p256@<0.14.0-0"` silently matched nothing, because `@` means an exact version
    (`crate = "p256:<0.14"` works);
  * `!=` is rejected.
* **Phase 2 must re-run** the complete fence against both graphs, including a deliberate break-it
  run for each ring.

What the fence cannot see: a new call **inside** `catcoms-mls` that makes an ECDSA signer or adds a
second suite. That stays a review obligation, backed by the MLS refusal tests in section 9 and by
review triggers:

* any change to `config.rs`'s suite or capabilities;
* any new `SignatureScheme`;
* any openmls version change, which requires re-reading section 2.2.

**CI gate.** `ci.yml`'s `deny` job checks only the root workspace, and native advisories are
already red for unrelated reasons. Add `cargo deny --manifest-path
apps/desktop/src-tauri/Cargo.toml --config deny.toml check bans`. Pass the config explicitly:
discovery walks up from the manifest directory, so a future `deny.toml` under `apps/desktop`
would otherwise silently replace the fence. The reviewer verified that native bans pass today
except for the intended p256 edge, which the port removes. Land the ignore, the fence, the CI step
and the native port in one commit, so no revision has the ignore without the port.

## 9. Tests

**Option A:**

* Native, existing tests, unchanged in content:
  * the shared Rust/WebCrypto vector test (both bootstrap vectors, framing open,
    transcript-hash substitution), which now draws through the production `BorrowedRng` adapter;
  * all `service/tests.rs` flows;
  * the unwinding-RNG ticket closure test.
* Native, new tests:
  * rejection sampling: a draw of `n` and then `1` gives exactly two draws and the generator point;
  * points refused before any RNG draw (compressed `02`/`03` prefix, off-curve, all-zero), using an
    RNG that panics if it is touched;
  * the re-derived layout guard.
* MLS, new tests that pin section 2.2 (review M4):
  * a P-256-suite KeyPackage, built with a test-only ECDSA signer, passes `parse_key_package` but
    `add_member`/`add_member_via_invite` refuse it with `InvalidAddProposalCiphersuiteOrVersion`;
    no commit or Welcome is produced and the epoch is unchanged;
  * `ServerGroup::join` refuses a Welcome whose suite does not match;
  * `MlsDevice`'s signer is Ed25519;
  * `capabilities()` lists only suite 3.
* Frontend: the TypeScript voice suites read the same `vectors.json`; re-run them.
* Ambient gate: add the patterns `SysRng`, `getrandom::`, `EphemeralSecret::generate()`,
  `NonZeroScalar::generate()` and `SecretKey::generate()`. A bare `::generate()` would collide with
  `MlsDevice::generate()`.
* Gates: Clippy `-D warnings` for root and native, fmt, the ambient and store raw-fs gates, root
  `cargo deny check`, native `check bans`, the break-it runs, and the complete suites
  (run by the coordinator).

**Option B, in addition:**

* RFC 9180 Appendix A.2 (X25519, HKDF-SHA256, ChaCha20-Poly1305, base mode): `DeriveKeyPair`,
  seal/open and export.
* RFC 8032 vectors.
* MLS `crypto-basics` interop vectors for suite 3.
* Golden fixtures generated at `2f08bde0` with the current backend, as test-only throwaway
  identities: a founder snapshot with a PendingCommit, a joiner's pre-Welcome map with its Welcome,
  and an application message. They must restore, join, decrypt, merge and agree on the exporter
  (the section 5.3 probe is the template).
* Byte equality of the signer's storage entry.
* `supports()` admits only suite 3.

**Upgrade PR (after the bump):** commit the section 5.3 probe as a fixture test (0.8.1-written maps
read by 0.9.x), and add the missing cases (i) to (iv) listed there.

## 10. Risks

* **A leaves an exception.** It depends on openmls enforcing suite equality before HPKE and on one
  pinned suite. A second suite in `config.rs` invalidates it; remove the exception first.
* **Native reachability is live.** A main-window script can drive the native ECDH today, so the
  port should not wait for the toolchain.
* **p256 0.14 is new and unaudited arithmetic** (`primefield`/`ctutils`/`cmov`). The vectors prove
  agreement, not constant time.
* **B moves crypto glue into our trust base.** We would own parity with `openmls_rust_crypto`:
  `verify_strict`, error mapping, AEAD tag handling.
* **The RNG seam does not reach MLS.** `openmls_rust_crypto` uses `ChaCha20Rng::from_entropy()`,
  `basic_credential` uses `OsRng`, and the ambient gate does not scan dependencies. A keeps that
  gap; B could route it through `catcoms_rt::OsCryptoRng`.
* **Duplicate crates** grow by 16 in the native graph (warn-level `multiple-versions`).

## 11. Design review record

The read-only adversarial review (2026-10-24) confirmed the three core claims: no released
backend clears it; MLS performs no secret NIST-curve operation; the RNG plan preserves the
ticket semantics. It found no blocker. Dispositions:

| Severity | Finding | Disposition in this note |
|---|---|---|
| HIGH H1 | The native workspace has no deny gate, so the fence's native failure only shows in manual runs | Section 8 CI step; one-commit landing |
| HIGH H2 | `wrappers` confine direct parents only; a new user of a wrapper crate (`hpke-rs-rust-crypto`, `basic_credential`) bypasses the fence | Ring 2 (section 8); review triggers; M4 tests |
| MEDIUM M1 | Ranges skip prereleases (`crypto-bigint 0.6/0.7` pre and rc builds) | Version-less ring-0 entry plus explicit prerelease ranges |
| MEDIUM M2 | Version drift inside the wrappers is unfenced | Exact-version bans in ring 2 |
| MEDIUM M3 | The bootstrap is reachable via `setup_voice_channel`, not "disabled", and is native-to-WebView over loopback, not intra-process | Section 2.1 corrected; urgency raised in sections 7 and 10 |
| MEDIUM M4 | A has no MLS-side tests for its own reasoning | Section 9 |
| MEDIUM M5 | The secret-path analysis was wrong in places (select, `ct_eq`, `add_mod`, "zero counting", two multiplications) | Section 3 rewritten, plus the assembly spot check |
| LOW L1-L7 | `creation.rs:225` order; openmls's backend dependency is optional; cargo-deny `@` syntax; full suites missing from the plan; `SysRng`/`Generate::generate()` seam gap; signer-mismatch panic; lock scanners still flag 0.5.5 | Sections 2.2, 1.2, 8, 9, 4/9, 5.2, 2.3 |

The short re-review of the revised note found no blocker and no high. It re-ran the asm scans and
the probes, and confirmed the fence does not fire wrongly after the port (`crypto-bigint 0.7.7`'s
only parents are `elliptic-curve 0.14.1` and `primefield 0.14.0`). Dispositions:

| Severity | Finding | Disposition |
|---|---|---|
| MEDIUM M-a | "Compatible in both directions" overstated: only 0.8.1-born state was downgraded; production staged-commit shapes untested; a test-only line was cited | Section 0 item 6 reworded; section 5.3 lists (i) to (iv) as upgrade-PR fixture requirements |
| MEDIUM M-b | The table omitted the `ctutils` part of the fix; the #1358 row missed the rustc 1.87 to 1.91 wide-word miscompile (AArch64) | Section 3 row and caveat; folded into the aarch64 revisit |
| LOW | `openmls` not version-fenced; native CI step should pass `--config`; the 0.9 step also changes ring 1; section 3 contradicted section 8 on landing order; rejection-sampling branches unmentioned; headline overclaimed the fence | Section 8 openmls bans (re-run: same result); section 8 CI text; section 7; section 3 conclusion; section 3 bullet; section 0 |

## 12. Separate findings (not in this PR)

* **RUSTSEC-2026-0386 (tokio `AsyncFd`, unsound)** also fails root `cargo deny` today. Root has
  tokio 1.52.3; patched versions are `>= 1.53.3` or `1.51.6` up to 1.52. CI deny stays red after
  this PR unless that is handled.
* **openmls 0.8.1 GitHub advisories that RustSec lacks:**
  * GHSA-gc79-23g3-8g52: high; unbounded recursion while decoding nested extensions aborts the
    process; fixed in 0.8.2.
  * GHSA-94v4-rg25-x625: medium; fixed in 0.8.3.
  * GHSA-p276-g3f3-wwgh: low; fixed in 0.8.3.
  * Two pre-authentication DoS issues fixed only in 0.9.0: GHSA-w62v-gv48-63rh and
    GHSA-rrmv-c79f-cf5r.

  openmls 0.8.3 with the 0.5.1 backend compiles on Rust 1.89 (probe). It deserves its own PR with a
  snapshot-reload check, and it must edit the fence's `openmls` version bans.
* **Other open native audit findings:** h2, quick-xml, zip, unic-*, proc-macro-error. HANDOVER
  already records that root deny does not clear them.
