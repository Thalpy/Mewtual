# Communication dependency repairs (13 November)

This is a compatible security-repair subset discovered while validating the
native Chat V2 work. It does not activate Chat V2 or establish clean audit/full
acceptance. Native registry preparation remains in draft PR92.

| Dependency | Previous lock | Repaired lock |
|---|---:|---:|
| DOMPurify |3.4.14 |3.4.17 |
| Vite |6.4.3 |6.4.6 |
| devalue |5.8.1 |5.9.4 |
| nanoid |3.3.15 |3.3.20 |
| PostCSS |8.5.15 |8.5.32 |
| source-map-js |1.2.1 |1.2.2 |
| undici |7.29.0 |7.30.2 |
| rustix (both workspaces) |1.1.4 |1.1.6 |
| xattr (native tar dependency) |1.6.1 |1.7.0 |

Direct manifest floors are raised for DOMPurify, Vite and rustix. Named npm
transitives remain within their original parent ranges; no forced major,
override, Svelte/plugin migration or audit exemption is introduced. Native tar's
xattr1.x range accepts1.7.0; the Linux dev regression pins that compatible minimum.
Unrelated alternative syn/windows dependency resolutions were restored to keep
the lock diff scoped. Full native `cargo metadata --locked --offline` validates it.

The [DOMPurify release](https://github.com/cure53/DOMPurify/releases/tag/3.4.17)
repairs sanitizer configurations including in-place/template/shadow content.
Our renderer continues returning strings through its original strict allow-list;
it does not enable those options or accept new tags. No corresponding exploit
of those configurations in this app is claimed. The hostile corpus now also
checks SMIL, shadow templates, raw-text and namespace payloads across every real
renderer, parsing the actual returned HTML to distinguish inert text from markup.

## Explicit xattr queries and paired compatibility

[RUSTSEC-2026-0486](https://rustsec.org/advisories/RUSTSEC-2026-0486.html)
is fixed in rustix1.1.6. Empty-buffer reads no longer return an attribute's size;
the explicit `fgetxattr_query` API does. The app's ACL detector now uses that API,
retaining its exact conservative outcomes: only NODATA/OPNOTSUPP rule an ACL out;
success (including zero) or any ambiguous/access error still means it may exist.
The former empty-slice existence check is not claimed to reproduce the advisory's
spare-capacity Vec undefined-behavior example.

Updating rustix alone would break native tar's older xattr library: its large-
attribute/list fallback used the old empty-buffer size convention. The paired
[xattr1.7.0 release](https://github.com/Stebalien/xattr/releases/tag/v1.7.0) uses
the new query APIs. Both crates retain MSRV1.65, within this repo's pinned1.94.
No unsafe code, weaker filesystem policy or deletion permission is added.

Linux coverage adds actual ACL absence -> named extended ACL -> absence and
retains the existing conservative error/group-export cases. The native dependency
integration test exercises both a5109-byte value and a name list over4096 bytes
through the actual xattr FileExt API. It uses `/dev/shm` tmpfs because ordinary
ext4 inodes often cannot store the large value. This test requires that Linux
runner facility; it fails instead of silently skipping the compatibility path.
The named ACL uses an actually mapped euid so user namespaces need no fake UID.

## Verification and remaining gates

Executed locally on Windows:20 storage external cases,98 renderer/wiki/infobox
cases with the required DOM preload, all3438 frontend tests, Svelte0 errors/
warnings, production build and actual browser flow checks. Linux syscall and
native large-xattr tests have not run locally. The focused CI lane requires
exactly one nonignored pass for each of its three Linux storage selectors and
the native integration test, using the existing owned timeout/result guard.

Current audits: npm reports3 LOW findings in the existing plugin/inspector/
deepmerge chain, down from3LOW/6HIGH/1CRITICAL; it is not a clean audit. Both Root
and native cargo-deny still fail on the same9 Hickory findings. The rustix
finding is gone; no advisory configuration or waiver changed. Remaining Hickory
IDs are2026-0487/0489/0490/0491/0504/0505/0511/0512/0514.

Hickory requires a separately reviewed network dependency migration: released
libp2p0.57.1 selects DNS0.45/mDNS0.49 and permits patched Hickory0.26.3. That also
changes core/identity/swarm/multiaddr APIs and needs stored/wire identity and
transport tests. It is not part of this compatible subset. Workspace strict
Clippy debt, complete current-source suites and physical acceptance remain open.
