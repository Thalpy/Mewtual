# Governed native storage: implementation design

This is the next implementation design, not an implemented constructor or
permission to enable Chat V2. The current production native opener still
refuses governance4. The completed MLS handoff in PR #131 has no Root consumer.
Read-only design re-review found no remaining intrinsic blocker/high after
the tuple bindings and repeated Stage-absence barrier below were clarified.
No runtime code changed; tests were not rerun for this design-only increment.

## Owning sequence

Complete the existing native1 to native2 to native3 startup chain, then declare
governance4 before creating a group or issuing a signing Source. Consume the
private `NativeTextVaultUpgrade::Phase::Selected3` while that continuation still
owns its original UI admission, Source admission and affine lifecycle closer.
Do not accept an arbitrary `NativeTextVault`: an exposed empty facade may
already have issued observations, so row count zero cannot prove non-exposure.

The new continuation retains the same session, keys, physical owner and
accounting. Once declaration begins, a failure retains that continuation; it
cannot return to ordinary `into_vault`, reopen the directory, substitute an
equal owner or silently restart declaration. Its startup profile also requires
an authenticated empty native index, settled native3 storage, no active JOIN,
no Source/group birth and no competing declaration. Existing exposed native3
owners keep refusing Add until a separately reviewed owning upgrade exists.

The steady-state handler is a distinct closed `GovernedLifecycle` variant
retaining the same actual lifecycle gate and payment. It requires namespaces
4,5,7 with native version3. Existing `Lifecycle` stays strict and refuses4.
Native descriptor/index3 and marker4 bytes need no version change while their
semantics stay identical; future row changes require their own decision.

## Parser and inspector integration

`archive_bootstrap::parse_configured_join_catalog` currently prioritizes the
generic governance policy. That parser only permits4; it must not bypass the
new steady handler's mandatory4 requirement. The exact configured parser must
cover both Current and Stage/JOIN reads. Staging accepts its authenticated
original native3 parent only under the unexposed declaration continuation,
never through a steady-state missing4 fallback.

Use actual closed-handler dispatch for lifecycle current/stage authentication,
the native inspector, WAL handler selection, cell admission and cached JOIN
idle checks. Keep the existing native2 to native3 upgrade grammar unchanged;
that migration finishes before4 exists.

Cold open must install both concrete handlers before older inspectors run.
Inventory and authenticate both namespaces and their possible debts before
the first repair. A pending4 declaration requires the exact settled native3
parent and unchanged native payload. Refuse simultaneous unrelated native WAL,
upgrade, JOIN or member-declaration debt. Settle4 first, then capture native3's
final whole-catalog evidence from selected4. Capturing that evidence before a
3-to-4 repair would make it stale. Reprove retained4 after native/JOIN waits.

## Declaration and recovery family

The existing governance original codec excludes namespace7. Add a separate
native declaration pair producer, validating the complete original3+5+7/v3
catalog and inserting4 while preserving every other payload byte and order.
Use a distinct seed version2 and cryptographic domain. Constructor family
selects the decoder; never fall back to seed1 after a version2 parse failure.
The old governance constructor and seed1 validators remain unchanged.

The native declaration must also cover the original native index and marker
tuple. A shared immutable namespace7 descriptor alone does not identify a
particular mutable index. Bind exact authenticated index/marker framed-byte
lengths and digests, their native7/v3 capability and the vault/root/namespace3
binding in seed2; retain their actual file/directory identities warm.
Revalidate those same leaves before effects and final exposure. After catalog
promotion, permit only this declaration's exact candidate catalog while keeping
the native leaves fixed; do not synthesize a new general inventory or use an
arbitrary caller-supplied replacement digest as proof.

Before the shared Stage fence, write and sync an attributable claim at the
known governance seed name, which older native readers already refuse. For a
first seed that never authenticated, cleanup is abort-only. It may leave the
actual original3 selected only after independently proving no marker4, no4 in
the authenticated catalog, the strict fresh-empty native3 baseline (including
generation/highwater/migration origin), complete absence of group/Source work
and competing native/JOIN debt, and an exact held empty Stage or its absence.
It cannot continue4, prove unchanged historical native hashes, reconstruct a
missing original3 or rewrite the catalog. Remove only the held empty Stage,
drop its handle, cover/reprove the directory, then remove the held torn claim
and cover/reprove again. Repeat the Stage-absence covering barrier on EVERY
retry, even when Stage is already absent; an earlier unlink is not a durable
acknowledgement. Never remove the claim before that barrier, which would strand
unattributable empty Stage after a crash. Preserve nonempty Stage, every ambiguous input and
every valid-MAC malformed/future seed. Every unlink needs a verified name-
absence check, including Windows delete-pending behavior. The complete-seed
path must recover exact cached outputs without drawing randomness. No new warm
retry may silently abandon an uncertain declaration and generate another one.

## Physical and cached proofs

The governed `checked_store` path must validate the actual mounted4 owner as
well as3. Avoid the self-locking4 validator while archiveRoot is already held;
use a receiver-specific under-root proof and final same-held-file checks.
Final cached tails remain free of IO, allocation, weak upgrades and account
locks. They use the captured original mounted4 identity/cancellation, never a
descriptor, readiness boolean or fresh lookup that could adopt replacement.

The full declaration/cold/marker/retired metadata overlap needs an explicit
independent invoice before allocation or file work. Retained native leaves
and outside-disposal tails must drain before their actual payment. SDK, Source
and native-command authorization remain separate obligations.

## Acceptance for this constructor

Use real files and the actual startup continuation for:

- same session/keys/closer and original physical identity through the move;
- capacity refusal before effects and exact retry after every declaration cut;
- first-claim, torn-seed, complete-seed, promotion and lost-return recovery;
- substituted, corrupted, missing and duplicate Current/Stage/seed/marker files;
- changed native index/marker under an unchanged namespace7 descriptor;
- conflicting debt and old-versus-new family refusal without fallback;
- byte-identical retention of4/5/7 through actual JOIN lifecycle;
- required4 in both steady parser paths and the original cached proof;
- a real predecessor binary refusing new vaults without mutating them, with
  positive native3 controls that demonstrate the old opener can still work.

No test should invent a selected3 receipt, native credential, original file
owner, Source or saved publication result. The constructor alone will still
not complete production Add/Header custody, joined sources, shared history,
transport, cold authoring restore or Chat V2 acceptance.
