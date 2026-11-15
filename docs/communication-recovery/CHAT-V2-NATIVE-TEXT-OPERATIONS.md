# Native text operations (implementation in progress)

Chat V2 remains off and incomplete. This branch continues the canonical native
controller from PR #123. Returning a stored founder index row is not sufficient
to return a usable server to the frontend: the existing server table and send
path require Legacy actors. Native text needs its own real row and dispatch.

## Original runtime association

`NativeTextCreationJob::select_stored_runtime` checks the retained whole driver,
the actual covered Active row through a fresh original index lookup, and the
original command/source before returning `NativeTextRuntimeSelection`. It does
not reconstruct an owner from a numeric server ID or recovered index DATA.

The selection owns only Scope metadata, a weak original RuntimeCell association,
the exact stored row and request bindings, and the original payment last. It
has no Entry/issuer, physical lease, SDK body or positive key permission.
`id_data` and `same_original` cannot prove current authority or readiness.
The actual runtime caches one canonical selection allocation. Every fresh
issuance repeats current/lookup checks, then clones that allocation; retries
cannot allocate unlimited records against one debit. Native must separately
bound/fund its invocation wrappers and copied DATA.

The initial adversarial review found and corrected that repeated-allocation
defect. It also identified an existing Entry payment-tail ordering gap after
body retirement. The Entry now closes its original key issuer synchronously
before dropping the runtime, and retains the original payment until afterward.
The real allocation-tail regression failed before that correction and passed
after it. All four selector cases passed, including foreign vaults with the
same numeric row ID and repeated selection without extra charged allocations.

## Member save implementation

`capture_member` retains a paid weak association, the original command and a
counted source preparation. Outside native guards, `finish` checks the fresh
covered index row, prepares the original Entry's member cut and drops the Entry
pin before borrowing the actual whole runtime. Phase one calls the real
NativeRequired signer, caches the original outcome and restores that runtime
before returning. Late cancellation preserves the presealed original for an
exact retry; it does not sign again.

`finish_admission` performs lookup/key-metadata preparation and allocates its
owning refusal frame before the native caller awaits its commit guard.
`try_admit` is fail-fast and performs no IO or new pool admission. Its success
and refusal frames retain actual native custody before every runtime/key/cell
tail. A refused weak upgrade or a busy cell returns that whole custody rather
than releasing a temporary last reference while the caller still holds commit.
`MemberSaveNativeAdmission::from_boxed_custody` accepts an already allocated
native guard slot; it is ownership transfer, never authentication.

All 12 member regressions passed on Windows: actual native signing,
unchanged retries, changed-body conflicts, early/late cancellation, close,
foreign row/account, normal/unrun admission, actual post-borrow refusal,
commit-point unwind and recovery, missing or busy original cell, and whole
retirement remaining pending after file-custody release. The last case directly
observes the actual outstanding loan and its restoration. These Root fixtures use a real
owned mutex guard and the actual RuntimeCell notification, but synthetic UI
credentials: they establish custody ordering, not native AppState authorization.
The complete affected source cohort passed all 78 cases (297.56 seconds), zero
failures/ignored. The outbox cohort passed all 38 cases (4.97 seconds). Only the
unchanged Native4 artifact producers are excluded from the normal source filter;
their dedicated compatibility gate remains separate. The unchanged frontend
passed all 3,446 cases, Svelte reported no errors/warnings and production build
passed. No native command factory or usable server row exists yet.

## Stored-original reader (under validation)

The internal member-outbox engine now reads saved originals by ordinal, including
the settled prefix that publication debt omits. It admits 1–8 entries per page
under an independent 5 MiB logical invoice retained after all plaintext. The
first page captures an exclusive upper ordinal; subsequent pages reuse it so
appends cannot extend that finite scan. Cursors are context-local DATA, with
strict range checks. start==end is a valid terminal empty page. Statuses reflect
each page's selected head; they are not a cross-page status snapshot or evidence
of remote receipt.

Each item cross-checks its ordinal and token entry, verifies the stored original
with the existing action codec and reads its authenticated disposition. An
uncertain owner or missing/corrupt file refuses the entire page. Reading never
recovers, creates or repairs files. All 38 outbox cases passed, including six
new real-file history regressions.

The consuming `NativeTextMemberJob::prepare_member_history` now selects the same
whole NativeRequired runtime and Current as saving. Host preparation can
initialize a genuine authenticated empty outbox or recover previously admitted
work; it never signs, settles/re-preseals custody, or admits a message merely to
read. A presealed unsaved original stays invisible to stored history. The
existing minimum resource policy covers history (5 MiB), an old prepared append
(6 MiB) and recovery (6 MiB) together; a compile-time assertion pins that overlap.
Additional simultaneously retained pages are separately paid and can cause Busy.

The paid output owns originals only. The whole runtime returns before the page
leaves; original command/source and complete account+row negatives are checked
around the work. Native must separately fund serialization and validate actual
original generation, mount and row at reply delivery. All five new Root history
tests passed in that 78-case cohort, including actual commit-unwind recovery
through history before save reconciliation. The strengthened exact action and
ordinal receipt assertion passed its focused rerun (one case, 8.94 seconds).
These APIs contain only this member's
saved originals, not a complete group ledger or installed native UI.

Both dependency audits and the ambient/raw-filesystem/exact-count self-checks
passed. Broader Root/native suites, native checking and current lint remain
pending for this checkpoint. The earlier controller CI passed Root and
native Linux; Windows native failed the SDK cleanup fixture described separately
in [SDK cleanup evidence](CHAT-V2-SDK-CLEANUP-ACCEPTANCE.md). Its corrected exact
case passed locally; no corrected full Windows native result is claimed yet.

## Required warm vertical slice

These parts must be implemented together before the frontend sees successful
native server creation:

- Root member prepare/admit/read methods select the exact retained runtime.
  No raw Store, Current, Host or SourceOwner getter is permitted. NativeRequired
  origin and the original per-purpose member cut remain enforced.
- Real TextV2 row custody supplies listing/opening, ordinary Create, durable
  own-message reading and retirement. Legacy actor methods remain Legacy-only;
  a dummy actor cannot stand in for an unavailable native backend.
- History comes from genuine stored originals. The existing outbox debt reader
  excludes settled history and cannot be presented as a complete message page.
  Displayed local saves must not claim replication, owner acceptance or receipt.

Native capture checks main-window ACL and captures original generation, mount
and row before scheduling. Founding binds a stable bounded request ID to the
complete canonical settings. A changed-body/token conflict refuses; uncertainty
never silently replaces a token or redraws/re-signs a retained original.

Member phase one restores its runtime loan and releases entered key Ends before
awaiting native UI commit. Final preparation is prepaid and retains no affine
Entry/issuer. Under the actual owned commit guard, final runtime/source checks
and admission are fail-fast and use the exact EntryOwner cancellation composed
with the complete original account authority.

An admitted file wrapper owns MemberSaveJob first and its runtime loan/keys
afterward. The consumed member job publishes its saved/unresolved outcome and
releases actual native custody before runtime restoration/notification. Unrun,
unwind and refusal paths preserve this order. A refusal returns whole native
custody before the loan/preparation; it cannot drop a local loan via an early
error while the guard remains held. Runtime remains busy through actual IO,
and whole retirement still requires both loan return and the save-source End.
This is not a universal no-wait claim: the admitted existing member job performs
IO, completion-cell locking and source-End retirement before releasing custody.

The first warm slice does not establish joined/Add governance, remote delivery,
reply/edit/delete dependency lookup, operational cold restore or multi-peer
acceptance. Those remain required for complete text Chat V2. No production flag
is enabled by these interfaces.
