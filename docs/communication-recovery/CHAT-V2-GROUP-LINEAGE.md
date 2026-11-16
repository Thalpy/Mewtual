# Group-wide negative admission lineage

The original archive founder's pre-row transaction now rejects ANY admission row or runtime
owner for the same complete group, across all full signers. This includes Initializing, Current,
Retired, and malformed orphan-owner metadata without a pin or canonical file. The first genesis
profile implements no migration or companion admission. Another group's existing history and
control work remain independent. This is negative exclusion, not a Current or saved-state proof.

`ChatAdmissionGroupBounds` builds the two complete `(group,[00;32])` / `(group,[ff;32])` endpoint
keys outside writers. `ChatAdmissionKey` orders group bytes before the full signer. The borrowed
inclusive range therefore finds the exact complete group, including both signer endpoints and
excluding prefix/neighbor groups. It allocates/copies no key while querying the actual Runtime;
the first entry of each loaded table/owners tree suffices for absence. No lifetime row scan,
pin/head/file read, RNG, lock, or caller-supplied complete/dirty status is hidden in the helper.

The actual archive source reserves `GROUP_BOUNDS_ALLOCATION_BYTES` from its original shared
catalog account BEFORE constructing those two Vecs. Its plan retains the debit LAST through
the keys/cursors. This enumerates max512 requested backing bytes plus actual bounds/fee/range
layouts. It neither pays the existing authenticated ScopeMap nor promises whole-host/RSS or
global native accounting. Capacity refusal preserves the actual creator and never samples a
scope nonce; retry reacquires resources without replacing the original group/source.

The helper is repeated in all original pre-row plan validation sites, including after earlier
capture and before original registration/effects. A later different-signer row cannot be
ignored because the exact local key remains vacant. Existing rows and sealed inventories are
retained unchanged on refusal. It does not manufacture a new root or treat Retired as empty.

The future `require_no_foreign` query permits the SAME managed, loaded dirty table only for
negative comparison; at most the first two table and owner entries are examined. It proves
neither that the caller's key exists/matches nor that an inventory is saved/clean. It remains
`cfg(test)` until the genuine Current scope consumer is declared with its separate original,
phase, owner and barrier gates; remove that cfg with the actual consumer, not with a fake call.

Seven regressions are authored UNRUN under the compilation/test/build pause: four negative
DATA matrices and three real creator/plan tests cover every phase, endpoints/prefixes, dirty
and missing inventories, malformed orphan owners, other-signer rows without pin/file, a row
appearing after capture, independent-group progress, exact one-byte-short payment and drop.
Design and final actual-diff adversarial static reviews are clear. Direct rustfmt, whitespace,
ambient and raw-filesystem checks pass. Compilation, full tests and the Current/ID3 physical consumer
remain separate requirements.
