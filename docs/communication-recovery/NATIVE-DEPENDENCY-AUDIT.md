# Native dependency audit follow-up — 27 October 2026

Runtime/configuration checkpoint: `c4aa9064c078440652e69ddb7aaac4346d2f041c`.
Both independent workspaces pass all advisory, ban, licence and source checks against the
current advisory database. Existing bounded exceptions remain; an audit pass is not a
cryptographic proof or a claim that every dependency is maintained.

## Changes and their enforcement

- Native `h2` is 0.4.16, fixing
  [RUSTSEC-2026-0258](https://rustsec.org/advisories/RUSTSEC-2026-0258.html).
  A downgrade ban rejects older stable versions.
- The updater retains `rustls-tls` with default features disabled. The resolved graph has
  no ZIP package. Updater 2.10.1 otherwise calls `ZipArchive::extract` on its legacy Windows
  ZIP path; signature verification alone does not establish extraction safety. Official
  configuration uses `createUpdaterArtifacts: true`, raw NSIS installers and AppImages,
  so removing legacy v1-compatible ZIP input preserves the current release contract.
  See [Tauri's artifact contract](https://v2.tauri.app/plugin/updater/#building).
  Published updater 2.14 still requests ZIP 4 and is not itself a remedy. The native
  graph has no waived ZIP advisory; the downgrade ban rejects ZIP below 9 if reintroduced.
- `tauri-utils` 2.10.2 resolves `urlpattern` 0.6 instead of 0.3, removing all five UNIC
  packages. This is a semver-compatible utility update, not a whole Tauri runtime upgrade.
  URL parsing is an authorization boundary: actual generated `RuntimeAuthority` tests now
  cover remote-labelled loopback, deceptive hosts and credentials, and the public parser
  has fixed scheme/host/port/path positive and rejection controls. Current capabilities
  still grant no remote scope. Those tests do not determine how a browser origin is
  classified at runtime or establish process isolation.
- The native arithmetic graph moves from newly yanked `crypto-bigint` 0.7.7 to compatible
  non-yanked 0.7.10, with `ctutils` 0.4.6. Native P-256 remains 0.14 and the injected RNG,
  transcript and encoding contracts remain required. Fresh vectors, layout guards and
  complete native checks are pending; previous-version results do not validate this patch.
- A Windows build exposed WMI 0.18.4 selecting `windows` 0.62.2 with an incompatible
  direct `windows-core` 0.61.2. Its published ranges permit both independently. Only
  WMI's core edge is restored to 0.62.2; other legitimate 0.61 users remain. Locked
  metadata accepts the graph, and the targeted Windows check passes in 14.47 seconds.
  CI now runs that native dependency check on Windows so a Linux-only native build
  cannot hide the same COM mismatch. Full native execution remains pending.

CI now performs all four cargo-deny categories on the separate desktop manifest using
the shared root policy. Its previous native step checked bans only, so a Root audit pass
could hide these independent native advisory failures.

## One additional maintenance decision

RUSTSEC-2024-0370 classifies `proc-macro-error` 1.0.4 as unmaintained, with no announced
vulnerability or safe upgrade. Linux GTK's `glib-macros` 0.18.5 and `gtk3-macros` 0.18.2
still use it. Both resolved packages are registry proc-macro build targets, processing
trusted build inputs; they ship no proc-macro code into the application. This records
maintenance risk rather than declaring the code generally secure.

The single advisory decision is paired with bans restricting ordinary resolution to
the reviewed version and those two parents. Stable wrapper-version changes and direct
application use fail. Current GTK constraints select stable 0.18 versions; admitting a
prerelease requires separate review because ordinary semver bans exclude prereleases.
Remove/review the exception on an upstream replacement/backport, a new advisory, or a
new use. There is no blanket allowance for unmaintained crates or ZIP/H2 vulnerabilities.

Metadata-only mutation probes exercise the actual root policy: the reviewed wrapper
graph passes; a direct application dependency fails with a banned-crate finding; an
unreviewed wrapper version also fails. The first positive fixture failed the independent
public-path wildcard policy, then was corrected to match this repository's private
packages before accepting either negative control. No production policy was weakened
for the probes.

Source design review and the before/after audit logs are retained under the integration
checkout's `target/backlog-*` files. Final native/Root/frontend/build/startup results are
recorded separately in [the integration handover](PR-BACKLOG-INTEGRATION.md).
