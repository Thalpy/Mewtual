# Chrono maintenance dependency repair — 20 November 2026

Fresh hosted audits fail on RUSTSEC-2026-0543, issued 17 November after the
previous integration evidence. RustSec reports all Chrono versions unmaintained
and no patched version. This repair introduces no advisory exception.

`catcoms-log` replaces its three Chrono calls with Jiff 1.0.1. Log filenames
keep the same local-time format; session IDs still hash epoch nanoseconds and
the process ID, and sink health still stores epoch milliseconds. No protocol
clock, admission deadline, logging filter, file limit or public DTO changes.

The exact existing Netwatch 0.19.3 package is vendored with its locked crate
checksum verified. Both workspaces select that source. Its sole source change
is in the normalized/original manifests: disable the Windows WMI dependency's
default features. The real route query reads a single `Name` string and uses no
optional WMI date representation. All Rust sources remain byte-identical to
the published crate; upstream license texts come from its recorded commit.
See `vendor/netwatch/MEWTUAL-PATCH.md`.

The root lock no longer contains Chrono. The native lock retains Serde With's
optional Chrono package rows; Tauri requests Serde With's default features,
which do not enable its optional Chrono conversions. A shared cargo-deny ban
rejects Chrono in either active dependency graph, including future feature
reintroductions. Hosted audits must confirm the resolved feature graphs; the
presence of inactive lockfile rows is not itself a passing audit result.

Both lockfiles preserve unrelated package versions and dependency selections.
Jiff's packages and portable-atomic-util are the new dependency family. Existing
strict lint, Root/native complete suites, logging tests, Windows WMI compilation
and both dependency audits remain required on the changed source. Local work
used lockfile resolution, source comparison, formatting and whitespace checks
only; no build or test ran on this machine. This is a candidate repair, not
accepted Chat V2 or native/platform validation.

Primary references:

- https://rustsec.org/advisories/RUSTSEC-2026-0543.html
- https://docs.rs/jiff/latest/jiff/_documentation/changelog/index.html
