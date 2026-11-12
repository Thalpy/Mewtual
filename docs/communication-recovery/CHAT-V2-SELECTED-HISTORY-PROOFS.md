# Selected governing-history proofs (12 November; Core increment, unexecuted)

This is negative-only Core support for a future durable governing archive. It
does not implement the archive, a cold source/restart factory, new native
commands, a production joined source or activation. Chat V2 remains off.

The joined-source coordination contract requires durable backing rather than
evicting fork evidence after 4096 messages. The existing opaque hot parent is
bound to the original verifier identity and cannot be reconstructed from fields
read from disk. This API leaves that check intact. It verifies untrusted cold
proof DATA against an additional, private commitment maintained by actual Core
selection, and returns no reconstructed parent or positive authority.

## Commitment and selection

The context hashes the length-prefixed group, Profile ID, tenure, pinned genesis
Header ID and genesis state commitment. It starts only after verified genesis
under the caller's independent HOST pin. No transient process identity enters
it: a new verifier must fully replay the same pinned, selected records to
reconstruct the same commitment. There is no frontier decoder/import setter.

Each leaf commits the actually selected parent Header ID and its selected
child Header ID. A private 64-peak frontier and cached root keep Core memory
bounded independently of message count. Empty, leaf, node and context use
separate `mewtual/chat/selected-history-*/v1` domains; they do not reuse the
admission/action acceptance-tree hashes or context. The tree is ordered and
folds incomplete right edges right-associatively. Count is checked separately
against the private tip, and parent sequence must equal proof index plus one.

Derivation precomputes checked count/carries, hashes, next frontier and at most
64 created nodes. `V2GeneralCandidate::selected_history_append_data` lends their
immutable DATA to the future writer. Failed derivation, abandoned candidates
and refused HOST selection leave the private frontier unchanged. Successful
`advance_host_selected` moves the original precomputed outputs without hashing,
allocating or doing IO. The existing one-candidate/undrained-retired-owner claim
still prevents accumulating whole candidate models. Retired frontier/node
owners return whole for disposal or paid archive handoff outside writers.

## Cold negative observation

`observe_proven_archived_header` accepts the requested tip, index, original
signed parent, original selected child and at most 64 sibling digests. The
requested tip is untrusted DATA: verification uses the verifier's private tip.
A different context refuses Scope, a changed count returns `StaleTip`, and a
wrong root/path/index refuses Dependency. None closes the source. `StaleTip`
does not authorize more work: the eventual Root reader must keep its original
deadline/cancellation/resource budget and bound retries.

The inclusion proof authenticates semantic IDs, not newly supplied signature
bytes. Both successor checks validate parent/child/incoming representations
before semantic equality can return a duplicate or a confirmed fork. A valid
selected duplicate is a no-op. A different valid sibling caches the actual
parent and both signed children before irreversibly fencing the original
view, without waiting for either sibling's body. The cached contradiction
remains readable after closure or a lost caller reply. No action, Admission,
frozen cut, save permission or replacement Current capability is issued.

## Resource and persistence boundaries

`metadata_layout_bytes` now includes the actual `Option<Hash>` frontier layout,
frontier boxes, candidate/retired node overlap, controls and one cached cold
contradiction. Existing App member-lane accounting already includes that Core
invoice. The separate observation scratch invoice covers Core proof/signature
encoding work; callers must also pay their input records and retained buffers.
Neither invoice claims allocator/RSS or physical archive capacity.

Root still has to implement all of the following before production eviction:

- Store exact governing records and append-created nodes under a registered,
  authenticated persistence format, with actual selected-write barriers.
- Preserve all membership fork evidence and old Append parents; a bounded hot
  cache may not silently discard the only evidence or impose a lifetime cliff.
- Export proofs using bounded authenticated reads. Count/range geometry from
  `acceptance_tree::proof_plan` may be reused, but its hash folding may not.
  At most 64 sibling hashes does not mean at most 64 node/page reads.
- Retain and replay known contradictions across restart before authoring.
  Core's in-memory cache is not a durable receipt or a cold restore mechanism.
- Restore historical Admission/frozen-cut provenance separately. This negative
  proof does not reconstruct those positive opaque owners from DATA.
- Stop or pause coherently on unavailable/corrupt pages and storage pressure,
  preserving originals; a missing page is not proof of absence or a fork.

## Authored coverage and pending execution

Thirteen new deterministic tests cover selected trees and independent recursive
roots, every path through small irregular trees, an old body-less sibling after
4101 actually selected Core steps, exact full replay with fresh verifier identity,
foreign pins, wrong paths/indices/roots/context, stale reads, all same-ID bad
signature positions, unselected siblings, failed/abandoned/refused candidates,
original removal/frozen custody, retired-owner drainage and u64 arithmetic.
The exporter is deliberately untrusted in-memory test data, not an App archive.

Standalone rustfmt and static gates are run locally; Cargo execution is routed
through CI at the user's request. The exact-count lane must execute 11 replay
cases, 2 arithmetic cases and 3 existing accessor controls per OS. These are
focused Core results, not full Chat V2 acceptance. Full suites, workspace strict
lint/audit and production archive/native/network acceptance remain required.
