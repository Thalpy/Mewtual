# Chat history across partitions and MLS epochs

Status, 20 October 2026: current source `985c1f1d` on `chat-update-implementation` passes
complete suites: root 3,079 (24 existing ignored), native 443+5 and frontend 2,254, alongside
formatting, strict root Clippy, root dependency checks, builds, actual startup and ten rendered flows.
The automatic three-bubble case passes focused checks and the complete native suite. The
[acceptance ledger](CHAT-FILES-VOICE-ACCEPTANCE.md) records exact evidence, intermediate failures
and the distinction between graceful saved reopen and abrupt-crash custody. The communication
programme remains open; these chat results do not enable live V2 or production media relay.

## The ledger and the conversation are different views

A chat channel is an `EncryptedDoc`: an Automerge change graph plus the original inner-signed
operations accepted by this replica. Ordinary chat does not use Studio's owner-settled checkpoint
epochs. Changing the MLS membership epoch rotates keys; it does not replace that graph or its log.

An insertion, edit or deletion adds a signed change. Deletion removes an element from the live
conversation, while the operation log and CRDT history retain the original content and the change
that deleted it. An edit replaces the displayed text, while earlier values and concurrent
alternatives remain in history. A locally sealed snapshot includes the document and signed log,
not just the currently displayed rows. An accepted operation is not necessarily already durable:
a covering local snapshot must finish before it survives a crash.

The ordinary receive path deduplicates signed operations and applies their real dependency graph.
A dependent edit/delete can arrive before its message and wait for its dependencies. Snapshots
also retain those pending changes. Replaying an older insertion does not remove a deletion or
undo a later causal edit. Reading a saved historical view is distinct from putting it back into
the live conversation.

This gives the requested additive base for well-formed changes by independent writers. It does
not mean every arbitrary change a malicious member signs has safe message semantics; the legacy
authorization limits below still apply.

## What happens at an epoch boundary

The live gossip path opens current-epoch packets with the current channel key. Just-superseded
packets can use a retained past key (default window: eight epochs). A packet older than that
window queues document catch-up. A future-epoch packet also queues membership commit recovery.

Catch-up re-encrypts the original signed operations under the current key. It does not re-author
them, change their timestamps, or attribute them to the bridge. A late member can receive old
history without acquiring the old epoch keys. Removing a member does not invalidate their
previously accepted operations. Filtering all historical authors against the current roster
would incorrectly erase legitimate history.

The requester merges what it receives with its own graph. A remote history snapshot never wins
just because it is newer or contains more messages. Every current authorized holder can serve
the accepted operations it learned from other authors, so a bridge can carry a bubble's history
onward after its original authors disconnect.

Convergence still needs an eventual authorized path and a surviving holder of the missing data.
Membership recovery also has finite archive limits. A member older than every available commit
chain cannot reconstruct missing membership authority from chat timestamps or a peer's assertion.
See [MESSAGE-FLOW](../MESSAGE-FLOW.md) for the actual network scheduler, paging and recovery limits.

## Edit and delete ordering

Wall-clock timestamps are presentation metadata, not permission to overwrite history. A slow
clock can issue a valid edit or deletion after observing a message even if its timestamp is lower.
A forged future timestamp must not grant authority to erase later activity.

The current causal rules are:

- A later edit that has observed earlier edits supersedes them in the live text register.
- Truly concurrent edits retain their alternatives. Automerge chooses the same displayed winner
  on replicas with the same graph; arrival order and `edited` timestamps do not choose it.
- Deleting an observed message hides that list element. A concurrent edit to its object does not
  resurrect it. An unrelated new message remains visible.
- A delayed dependent deletion waits for its original message dependencies. Duplicating either
  operation does not create another message or another deletion.

Visible chat rows remain timestamp-sorted, with a bounded local Lamport-style step when sending.
That is a display policy, not a perfectly accurate global clock. An offline bubble can merge into
an earlier part of the conversation. This does not discard its operations.

## Breaks found by this investigation

The first application regression run passed one case and failed six. The following corrections
preserve the existing signed wire and storage formats:

1. Concurrent first writers can create several live `messages` lists. Reads already displayed
   them all, but mutations searched only the root property's winning list. One shared resolver
   now finds a unique visible message across all lists for edits, deletes, reactions and pins.
   Duplicate IDs are refused before authorizing or posting an action; they cannot select a
   different author's object. Legacy id-less messages remain readable and non-actionable.
2. A concurrent non-list value at `messages` could win that property. Appending an ordinary new
   message would then create a replacement list and hide the valid observed lists. Append now
   reuses an existing valid list when one is available.
3. A member could sign a legacy change using another device's modern 32-byte Automerge actor,
   occupy its next sequence first and make a genuine operation fail on some replicas. Ingress
   now checks that a 32-byte actor equals the signed author, after signature/parsing checks and
   before touching the graph or accepted log. This applies to legacy document ingress generally,
   not only chat. Historical implementation-generated actors of other lengths remain compatible;
   a real pre-rebind 16-byte restore fixture is tested.
4. A signed history page containing a bad inner operation could commit its continuation cursor
   before the complete page was imported. The next request could skip unprocessed valid tail
   operations. Failed imports now preserve the original cursor/completion state. Any verified
   prefix invalidates older completeness answers and keeps recovery queued before the error
   returns. Retrying the original position deduplicates that prefix.

The actor check is a limited admission tightening, not retroactive repair. Previously accepted
poisoned graphs/snapshots are not rewritten. An old relay with an invalid archive can remain an
unusable source; preserving the cursor avoids skipping its valid tail but does not invent a valid
dependency for rejected history. The bounded scheduler must continue looking for other sources.

## Remaining security and audit limits

Three important limits remain. Tests explicitly characterize the latter two where applicable;
passing a characterization is not passing security acceptance.

- Legacy raw channel changes do not enforce all message-author and moderator rules at receive
  time. The product checks them when an honest client issues a command, but a modified member can
  submit arbitrary CRDT mutations. The signature identifies the change signer; the mutable
  message `author` field is not itself authenticated authorship. This is threat-model residual R6.
- Two copies of one device state can sign different changes at the same actor/sequence. Each
  replica can keep the first and reject the other, giving different graphs. Matching the signer
  does not solve same-device equivocation. Unauthenticated legacy actor namespaces also remain
  a compatibility limit. These require a reviewed writer-incarnation/conflict protocol.
- A legacy inner signature binds document type/ID, author key and change bytes, but not the
  originating group or original membership epoch. A dual-group member can rewrap an unchanged
  signature into another group's matching document ID. It is not proof that its author belonged
  to that group when the operation was created.

A trustworthy moderator history therefore needs more than a new UI. A future version must bind
immutable message identities and typed insertion/edit/delete actions to their actual signer,
group and causal target, with verifiable historical authorization. It must specify deterministic
conflict handling, coexistence with old clients and preservation of old history. It must not
rewrite another author's signed bytes or require a removed author to become a current member.
This design is not implemented by the lookup or actor fixes.

All current members holding the same encrypted history can inspect its old content using a
modified client. A moderator-only UI does not make the underlying records moderator-only.
Restricting audit content to moderators would require a separate encrypted access policy and
an explicit retention policy. A visible deletion cannot recall copies already held by peers.
No moderator audit UI or stronger confidentiality guarantee is claimed here.

## Tests and honest coverage

`catcoms-app/src/chat_history_tests.rs` exercises 20/50 independent first writers, identical
timestamps, every losing-list action, ambiguous IDs, malformed root alternatives, concurrent
edits, backward clocks, reordered/duplicated changes, deletion replay, pending-delete restart,
actual Server commands and exact newly signed operation counts. Its MLS case crosses ten Adds,
then removes an author, reseals the unchanged original signatures, restarts with missing
dependencies and compares heads, visible messages and retained operation hashes.

`catcoms-replication/tests/chat_history_adversarial.rs` checks modern writer impersonation in
both orders through live/catch-up admission and restart, authentic old-style restore compatibility,
and the explicit same-device/group-binding limitations. `catcoms-sync/src/catchup_page_tests.rs`
checks rejected pages against signed responses, prefix custody, cursor/completeness state and
later corrected pages.

`six_client_recovery/chat_bubbles.rs` now adds actual native TCP recovery. Three isolated pairs
write from the same saved base with gossip dropped. One pair removes a member and advances MLS,
another concurrently edits messages that the owner deleted or pinned, and a third retains a
previously unseen post signed before its author was removed. Automatic recovery joins their
original signed-operation union through a replacement bridge, then serves two returning members
while the original A/B authors are offline. Assertions cover explicit visible edit/delete results,
pins and reactor identities, the operation dependency graph and heads, membership/epoch, and
solo saved reopen. No history or manual catch-up result is injected into a live recipient.

These are deterministic semantic/crypto/scheduler tests, not a 50-device physical network test.
The ten-epoch test uses real MLS and replication but explicitly delivers commits/catch-up; it
does not establish automatic networking across an evicted key window. The acceptance ledger
records the completed runs and their source/coverage boundaries. The new TCP scenario uses six
devices, one removal/epoch, small unpaged history and graceful freeze/save barriers. Its isolated
pairs run sequentially to establish independent causal branches; it does not simulate 50 active
network endpoints or prove abrupt-crash custody. Multiple membership transitions, paged bubble
history and abrupt crashes remain broader acceptance work.
