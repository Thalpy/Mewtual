# Mewtual OpenMLS 0.9.2 clock seam

This is the complete published `openmls` **0.9.2** package, not a new SDK version.
It is copied from the existing locked registry archive, not from an unpinned branch.

- Registry archive: `openmls-0.9.2.crate`.
- Registry/lockfile SHA-256: `ce4173f914cbc2dd28d9499f19e92f3e1bfebd9eea1607889924a77ca63d3ffc`.
- Packaged `.cargo_vcs_info.json`: commit `8a8f83b4b61fc45f28facec25e914992b3465de6`, `path_in_vcs = "openmls"`.
- Package manifests, version, dependencies and features are unchanged.
- `MEWTUAL-UPSTREAM-FILES.sha256` lists all 272 exact original package-file hashes before the one-file patch. The registry's `.cargo-ok` artifact is not included.
- The archive omits a LICENSE file. `LICENSE` reproduces the upstream MIT license at [that exact revision](https://raw.githubusercontent.com/openmls/openmls/8a8f83b4b61fc45f28facec25e914992b3465de6/LICENSE), independently checked against the existing OpenMLS license in this repository.

The original clock-seam code delta is `src/key_packages/key_package_in.rs`:

`KeyPackageIn::validate_with_time(self, crypto, protocol_version, now)` executes the
entire existing validation path. A private closed time mode chooses the existing
`Lifetime::validate()` or `Lifetime::validate_with_time(now)` only at the final
lifetime check. Legacy `validate()` therefore retains its original end-of-validation
clock sampling. The explicit API uses native `std::time::SystemTime` or wasm
`web_time::SystemTime`, matching the already-public Lifetime API exactly. No new
callback, unchecked mode, verified flag, message grammar or error type is added.

The caller must supply time derived from its trusted local `catcoms-rt::Clock`,
never a peer's timestamp. This hook is not a bounded parser, operation reservation,
Source/UI guard, membership grant, or permission to publish. A future bounded
KeyPackage consumer must separately enforce complete TLS consumption and nested
allocation limits, pinned suite/capabilities, full credential/group/nonce/KPref,
acceptable lifetime range and the admission remaining-lifetime floor.

Other OpenMLS paths still call legacy validation unless deliberately paired with
one of these hooks. The separate Add builder hook below supplies Add lifetime
validation, but neither hook makes merge, received commits, KeyPackage generation
or the rest of OpenMLS clock-free. Neither opens the closed SDK graph's mutation
fences or enables native dispatch.

Both root and desktop-native workspaces now activate this same path patch and
update their own lockfile entries together. Only the registry source/checksum was
removed from the existing 0.9.2 lock entries; package dependencies remain unchanged.
The root MLS tests and native check resolve offline with --locked. All six native
clock regressions pass, and subsequent MLS suite results are recorded with their
specific integration stage in docs/HANDOVER.md, with the existing fixture
generator ignored. Strict MLS lint also passes. Complete backend/native validation,
dependency auditing and wasm checks remain separate requirements; current results
are recorded in docs/HANDOVER.md at the repository root.

Six native-only focused tests live in `catcoms-mls/src/device/key_package_time.rs`, using genuine
MlsDevice keys, the production SDK builder/signatures/TLS parser/validator, and a
fixed Lifetime of 10,000..20,000 seconds. They cover exact inclusive/exclusive
boundaries, subsecond floor, pre-UNIX errors, outer/leaf signatures, protocol,
key separation, extension support/context and preserved legacy wall-clock expiry.
They do not enable upstream test-utils or unchecked-conversions.

The test module is explicitly native-only because Mls uses native SystemTime and
adds no wasm web-time development dependency. The SDK wasm API still matches
Lifetime, but its wasm compilation is a separate unrun primary verification gate.


## Separate numerical layout patch

The separately reviewed `group::text_add_layout` reports exact compiled native
size_of terms for 42 named wrappers. Three private numeric constants keep their
underlying private payload types inaccessible. The helper is absent on wasm and
the documented unsupported structural feature families. It reads no SDK object,
performs no operation and issues no admission, source or resource reservation.
Transitive allocations are excluded: consumers must audit actual simultaneous
copies, dynamic backing, crypto/codec scratch and phases under independent payment.
This patch is separate from the original single-file clock hook and is not a total
operation-budget claim.

## Separate Add builder clock patch

`CommitBuilder<LoadedPsks>::build_with_time` selects a trusted caller time only
for the existing Add KeyPackage lifetime checks. It shares the complete original
builder and validation algorithm, preserving legacy checks and legacy clock
sampling. Explicit checks cannot inherit the tree/KAT lifetime skip. See
`MEWTUAL-ADD-EXPLICIT-TIME.md` for the API, scope and six passing real SDK
build/stage/merge regressions. The separately reviewed serial route is described
in `MEWTUAL-PRIVATE-SERIAL-ADD.md`; the owning operation/invoice still remains open.

## Separate pending-merge timestamp patch

`MlsGroup::merge_pending_commit_with_time` shares the complete original merge
algorithm and supplies only the replacement message-secrets timestamp. Legacy
sampling stays at the original statement. The three genuine SDK regressions pass,
including stored timestamp, Welcome key agreement, cold restoration, no-op and
partial failure behavior. See `MEWTUAL-MERGE-EXPLICIT-TIME.md`; this does not repair
an uncertain merge or issue a private-Add operation permission.
