# Exact retained first-Add recovery (source checkpoint)

This completes the closed first-Add host's retained-stage boundary. It is not the scalable v2
public header/archive, ordinary history merge, a new admission, or proof of a remote fsync.
Compiling/tests/builds remain paused. The connected source now implements the automatic recovery
below; its authored regressions are UNRUN and actual-diff review precedes integration. This is
source evidence, not an executed acceptance or production Chat V2 activation claim.

## Two actual on-disk layouts

The source must read and authenticate BOTH fixed names with the original mount, independently
captured root and prepaid bounds. Any nonfile, corrupt, unsupported or inconsistent entry refuses
with both retained. There is no fallback from a bad stage to the old canonical file.

1. Canonical v1/gen1 plus a fully framed v2/gen2 stage. The stage's parent head, parent generation
   and parent snapshot must equal the ACTUAL authenticated canonical metadata. Full group/signer/
   profile, local domain/nonce and admission revision also match. The external row must be the
   same Current incarnation before a positive promotion; Initializing cannot acquire this path.
2. Canonical v2/gen2 plus a duplicate v2 stage with exactly equal COMPLETE plaintext. This is the
   original candidate already renamed during an earlier retry, not a second operation. A different
   stage cannot be discarded because its generation/nonce happens to match. No old parent is
   claimed available in this case: the actual complete canonical v2 source is the recovery source.

The original process's pending Meta may survive after its writer drops. A cold preparation captures
head current/pending labels only when no live writer exists, then requires them to be absent or
the exact captured canonical/candidate metadata. It never clears an unknown pending operation.
Before a new publisher is registered, head custody rechecks those same identities and current row.
Retired classification remains offline and constructs no transport/MLS. No v4 negative/S1 rule is
widened; v4 initial-stage recovery remains gen1/Fresh+Initializing only. Known v6 may inherit the
positive owner-operation format only after its actual supported catalog gate has succeeded.

## Private validation before any move

The v2 stage gets complete borrowed body/tail/parent preflight before SDK restoration. A gen1
parent also gets the closed borrowed initialization preflight before its provider is loaded.
There is ONE transport-owning Stage Server, never a second runnable parent ChannelSync.

For layout 1, a pure private parent validator loads only the original bounded MLS device/group
plus bounded metadata. It verifies actual epoch0/one-owner/full signer and original four-fact
chain against the independent pin. The stage Server is restored once through the strict bounded
operation decoder. Before promotion, the validator checks its signed commit's before authenticator
against that ACTUAL parent group; preserves the exact original four signed facts; compares unchanged
routing bytes, file-wrap key, policy pin, local scope and empty durable state; and computes/compares
the pure parent-to-child tenure transition. The child still passes complete existing first-Add
request/KP-reference/credential/Welcome/commit/eight-fact/current-group validation. It does not rerun
fresh invitation/KP deadlines, mint another Add, replace an RNG or infer a fresh join from a roster.
Bounded latest advisory rendezvous/finalization metadata is retained from the staged view; it cannot
grant membership. Both snapshots refuse documents, earlier commits/results or unsupported history.

The parent validator has no transport/RNG/runtime getter or fresh/observed-membership witness.
It is consumed/dropped before final file I/O. SDK typed heap expansion and active host quotas remain
separate open gates. The read reservation pays four snapshot buffers and two provider invoices,
PLUS the SAME original reservation must charge the existing first-Add extra private
allowance BEFORE any parent SDK or owned outcome decode. Four snapshot buffers ALONE are not a
complete invoice: `raw -> unframe` owns a separate ciphertext Vec before `unseal` allocates plaintext.

| Phase | Retained / simultaneous objects | Payment |
| --- | --- | --- |
| Select the two sources | One canonical plaintext plus stage raw, copied ciphertext and stage plaintext; release the raw/ciphertext before continuing. | Four configured snapshot buffers. No SDK/result is decoded yet. |
| Validate parent/child | Parent plaintext (`<= max MLS +64KiB`), stage plaintext, temporary child MLS copy, small original facts/metadata, bounded provider pairs, owned child outcome/log and commit-encoding scratch. | Four snapshot buffers + two provider table/payload invoices + first-Add extra allowance. SDK typed heaps remain open. |
| Final file barriers | Stage plaintext plus at most one `raw`, copied ciphertext and unsealed scratch at a time, retained parent plaintext if layout1, child outcome/log. Scratch drops before rereading another file. | Four snapshot buffers; extra allowance covers parent (`<=8MiB+64KiB`), outcome/log (`<=8MiB+64KiB`), commit comparison (`<=4MiB`) and bounded small scratch. |
| Duplicate layout | Exact full plaintext comparison first, then discard the duplicate canonical plaintext; retain the candidate once. | Source selection uses four buffers; later phases use the same extra allowance, not two retained full gen2 images. |

`OWNER_FIRST_ADD_EXTRA_PRIVATE_BYTES` is a checked fixed policy invoice above those simultaneous
closed maxima; reserve/update it under the SAME original pool guard and keep its exact Drop debt.
No new pool/mount, `len` heuristic or postallocation admission substitutes for it. Read/unseal
scopes release scratch before the next file. The layout1 parent is the closed initial grammar,
not an arbitrary second full history snapshot.

The parent/child linkage above relies on the locally authenticated ORIGINAL complete candidate
from the trusted covering producer: that producer checked `add_member_observed.matches_current`
before freezing. Cold checks revalidate its exact owner-signed commit/credential/Welcome claims
and actual private group state plus before-auth parent binding. They do NOT reapply the encrypted
own commit to the parent or prove a portable cryptographic MLS transition from cross-label
equality. This is sealed-local recovery under the independent native pin, not public finality or
proof that a malicious owner truthfully described its private MLS transition.

## Same-process repair and actual barriers

The original pending owner already retains its exact frozen candidate and writer. If a stage exists,
its actual authenticated plaintext must equal the constructed exact candidate. Canonical bytes must
equal either its original captured parent or that same candidate; unknown bytes refuse. This path
keeps the original authority/deadline/attempt and never samples a replacement writer.

Both same-process and cold paths run under original Current admission -> exact head custody. They
sync the actual opened parent (layout 1), sync the exact opened stage, repeat original negative
checks at move admission, promote only the fixed stage, and sync/re-read the resulting exact
canonical file plus child directory. Windows handles close before rename under those same locks.
Layout 2 first establishes the exact canonical barrier, then removes ONLY the validated duplicate
stage and syncs its parent. An error retains whole uncertainty, never returns Saved, and a retry
repeats the actual barriers. A stage already promoted by a prior exact retry may be absent only
when the current canonical bytes equal the retained candidate; absence alone grants nothing.

Only after complete exact barriers does head current become gen2 and pending clear. Same-process
install then moves the original private governing fields and reconciles negative scopes; cold
restore registers a fresh runtime writer under the same independently Current incarnation. A new
authenticated request uses its OWN original current writer/Responder to replay the retained outcome.
Neither saved result retains a dead request wait or adopts a later native actor.

## Required authored regressions (unexecuted until validation resumes)

- Actual TempSynced retained candidate: same original owner retries automatically with identical
  frozen bytes/one Add; no response before the complete barrier.
- Drop/reopen with actual gen1+gen2 stage: strict parent validation, actual promotion, original
  result after exact retry and selected-KP completion; no second Add or consumed nonce.
- Gen2+identical stage settles the duplicate; altered/foreign/truncated stage, wrong parent head/
  snapshot/scope/revision or changed stable field preserves both and refuses.
- Parent/stage/canonical sync, move and duplicate-delete parent failures each retain an exact
  retryable source; a visible renamed file alone cannot create Saved/current runtime.
- Original retirement/lock wins before admission, live-writer conflict, stale prepared recovery
  after a different head, and Retired classification never promote or revive old permission.
- Existing v4 S1 tests remain unchanged; v5/v6 gen2 does not enter source-absent negative recovery.

The current App TempSynced regression covers original-writer retry, cold parent+child reopen,
full duplicate settlement, corrupt ciphertext, authenticated unchanged-key alteration, wrong
parent head, live-writer refusal and repeated actual stage-sync refusal. The pre/post-rename
fixture now repeats the exact canonical sync after a prior rename rather than rewriting an
already-present candidate. More move/delete fault combinations and physical process-kill
evidence remain part of final acceptance; ordinary unsupported history remains an explicit gate.
