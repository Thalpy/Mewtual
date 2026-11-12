# Saved bootstrap source to an owned V2 genesis Current

Design for review; no runtime implementation or compiled evidence. This is one bounded
**genesis activation/restore transaction**, not a live send/governor or a general historical
Current reader. Root owns source persistence and Factory owns the closed SDK consumer.

Architecture design review is CLEAR after the group-pin, cross-signer/v1 fences, durable
SourceSaved and phase/owning-cross-crate handoff corrections. This permits the next exact
format/invoice/API design, not runtime activation. Physical slots/caps, whole scope-inventory
payment, actual error ownership and connected consumer still require review before code.

## Starting evidence and exact missing APIs

Existing `ArchiveFounderSourcePreparation` owns the original creator/plan and the closed
Bootstrap/Transferred source; Store owns complete encrypted source custody before SP. The large
worker's in-progress **SavedBootstrapSourceFiles** is an actual historical writer result after
its source, genesis record/manifest pages and file/parent barriers, not a cold Current token.
Its fields remain private to Store persistence. It must bind the original mount/root/target, invocation,
debit/SourcePrepared operation, complete SourceId/context and both exact public PageIds. A decoded
status, equal ciphertext or completed work future cannot construct it.

Cold continuation requires a durable SourceSaved terminal closure produced from those actual
worker barriers, not merely SP plus matching files. Prefer a small exact terminal journal
successor before returning the warm saved result. Without it (or an activation seed durably
written while the real completed result is retained), an exhausted successful last pass followed
by a crash cannot be reclassified positively for free. A later full read requires a newly spent
remaining pass; configuration/restart must not replenish an exhausted grant.

The next consumer needs three additions, implemented together:

1. Store's original saved-source capture/reader and a distinct root-kind2 pin/head/activation
   transaction. Warm capture consumes/retains the actual saved worker result; cold capture
   authenticates the complete selected durable source/SourceSaved closure under the new mount.
   The cold type is a NEW source capture, never an old warm arm/Fresh/SDK ticket.
2. Sync's V2-specific **opaque unbound retained genesis seed**, exposed across the crate
   boundary as DATA/resource state. Only App's private activation factory may promote it to
   the actual owned Current; arbitrary trusted Rust can construct DATA but obtains no source
   authority from it. Reuse Factory's actual
   `restore_server_owner_checkpoint_bounded(..., OwnerSdkRestoreAdmission)` and borrowed whole-S0
   field preflight, with exact V2 genesis/no-v1-tail grammar. Existing
   `restore_owner_initialization_with_sdk` requires v1 InitialOwnerEvidence and cannot be used
   as if it verified a v2 source. Ordinary restore and v1 checkpoint bytes remain unchanged.
3. An App **OwnedV2GenesisCurrent<T,R>** with private fields/constructor. It owns the retained
   paid graph/node, same mounted scope/head writer, typed V2 root and selected exact genesis.
   There is no public raw Server/seed adopter, `run`, `&mut Server`, status setter or projection
   token issuer. This is the first actual source for a later governor, not that governor.
   Its minimum ownership is the unbound node, actual ChatAdmissionBinding, protocol-V2 writer,
   captured group pin, exact saved head/source and original negative owner. Captures retain
   identity/negative/resource metadata, not Store/vault keys or a physical-work session lease.
   Store Drop cancels authority even if this owned graph is retained; a later mount cannot be
   adopted by matching path, bytes or labels.

Proposed concrete caller shape is `ArchiveFounderSourcePreparation::into_genesis_activation`
returning an owning `V2GenesisActivation<T,R>` (or the SAME source owner on early refusal), then
`activation.retry_current(&Store, rng) -> OwnedV2GenesisCurrent<T,R>`. Its private phases are
SourceCaptured, SeedRetained, ActivationPrepared, InstallingScope, HeadSavedPendingCurrent,
SavedCurrent, Transferred; failures retain
the actual phase/originals, never demote to a generic fresh initializer. Cold
`Store::prepare_v2_genesis_current(key)` returns an opaque exact initializing-repair or Current
capture before T/R/Clock construction. These names are proposed, not existing APIs.

The exact cross-crate boundary is proposed as follows (none is an existing API):

```text
Sync prepare_v2_genesis_seed(
    raw_s0: &[u8], genesis: &V2GenesisExpectationData,
    limits: V2GenesisSeedLimits, sdk: OwnerSdkRestoreAdmission
) -> Result<UnboundV2GenesisSeed, SyncError>

UnboundV2GenesisSeed::data(&self) -> V2GenesisSeedData<'_>
UnboundV2GenesisSeed::prepare_scope(
    &mut self, original: &impl ManagedChatScopeBinding
) -> Result<(), SyncError>
UnboundV2GenesisSeed::into_runtime<T,R>(self, T, R, Box<dyn Clock+Send>)
    -> UnboundV2GenesisNode<T,R>
```

The builder validates complete bounded V2 S0/public genesis and preallocates its non-generic
runtime scaffold, not just the SDK pair. It exposes no SDK/device/group/ChannelSync/Server/Fresh
extraction, run, authoring or proof output. `prepare_scope` is a DATA/negative attachment and
cannot manufacture actual row/head authority from a caller's trait implementation. The App
factory supplies its concrete original saved binding and repeats the actual final checks.
`into_runtime` is an infallible field move after all fallible work/preallocation; its public
unbound node remains DATA, even with a fake caller's attachment. Only App's private
`OwnedV2GenesisCurrent` constructor can couple that node to the actual saved head/writer/source.
No later governor may accept the unbound node as a Current source. If implementing the scaffold
reveals fallible work at final move, it must be moved earlier or return the SAME complete seed
and T/R/Clock in an owning error; it cannot silently lose/recreate them.

## Keep the node and source ownership concrete

First prepare a retained SDK pair/metadata seed under its independently acquired graph lease;
that graph remains unbound. It is a private child of the actual capture, not a freely adoptable
`(Server, proof)` tuple. SDK load accepts only the existing closed epoch0 founder shape for this
genesis transaction. The source's actual group/full signer/epoch0/sole founder/authenticator,
exact cached S0 digest, public Profile/Founder Admission/Header1/state and genesis record/manifest
must all match before any initialization row is written. No SDK state is reconstructed by
`Server::found`, re-signed or sampled again.

For warm activation, keep the original node/creator alive and negatively fenced while building
the paid seed. Move its actual T/R/Clock into the new bound node ONCE at final handoff; never
clone/install an earlier RNG state. A failure before that transfer leaves the original and
retained seed in the same initialization owner. The transfer API must be consuming and
infallible after its final checks/capacity reservations; errors cannot drop T/R then restart
founding. Old provider/graph/payload cleanup runs outside writers before its fees refund.
The warm handoff also moves the SAME original creator/plan negative owner into Current custody;
an observer or a newly sampled sender is insufficient. Implement the source wrapper's terminal
phase so its Drop cannot revoke the moved owner. Keep old write-arm/activation capabilities
closed by their exact saved journal/head transition, rather than leaving old tasks authorized
merely because the original lifetime is still alive. A cold Current owner instead has its NEW
actual mounted row/head lifetime; it never claims the warm creator's sender.

For cold restoration, a new actual mounted owner supplies new runtime T/R/Clock only after the
saved source/current classifier succeeds. It does not acquire an old creator life or Fresh
witness. The same seed decoder/field checks and a new real SDK graph lease apply. Initializing
repair remains private/unexposed; a Current read binds only the exact actual row/head/pin.
The durable SourceSaved receipt permits a NEW paid read/typed-SDK admission. It does not refund
or reuse any exhausted source-write arm. A held read/source reservation covers all large-file
verification; its original mount/current identity is repeated after that work. Root/source
receipts and read leases do not silently upgrade the v1 restore preflight to support V2.

Core S0 is not a complete serialized App Server. `Server::from_restored_sync` rebuilds transient
caches and takes a display name separately. This genesis source permits only original
`Server::found` defaults and the bounded display name authenticated by the creation debit;
bind that exact metadata/version in the head. Native numeric alias and later UI state are not
invented from the source. Any additional durable App field requires an explicit covering codec.

## Distinct root/head and one logical writer

V1/v2 Profile labels currently share the `OwnerFactId` Rust type; that does not make their
signed formats or root pins interchangeable. Require explicit protocol-kind discrimination and
the actual signed V2 Profile/genesis, rather than reinterpreting a v1 fact. Preserve existing `pins::Pin` and its
serialized v1 inventory. Store a separately tagged/key-separated immutable V2 root pin at a
fixed GROUP-derived location under the original archive Root (vault/domain plus complete group
hash, not caller path). Pin trust is group/Profile scoped, not group/local-signer scoped. Read
that exact location under the SAME admission -> pins serializer; no lifetime directory scan or
independent per-target root selection is allowed. A local target/head may reference the exact
group pin ID, never select another root. Return an explicit protocol-discriminated mounted-root capture. V1
and V2 admission/head acquisition share the SAME group/full-signer serializer and head-key
exclusion. Extend that runtime cell with a closed protocol discriminator; old v1 accessors
must refuse a V2 cell, never see it as an empty v1 head. Existing target fences stay in force.
Old v1 root selection/creation/restore must ALSO inspect the fixed V2 GROUP pin under that same
pins serializer, even for a different local signer whose target/head is absent. Conversely V2
creation refuses an existing v1 group pin. Checking only the per-signer head/target is not root
continuity. This compatibility change and both cross-signer directions belong in the first
Current implementation, before any V2 pin can become durable.

The new strict genesis head binds vault/group/full signer, root-kind2/Profile/founder/tenure,
SourceId plus raw-S0 digest and limits, both genesis PageIds, signed Header1/state identity,
planned scope nonce/revision and actual installed Current revision, original debit/name metadata,
and its generation1/no-parent role. Its complete sealed-frame ID is distinct from page/source
IDs. It lives in fixed per-target names, not a numeric snapshot alias. Head/seed/pin tags,
physical slots, complete byte caps and namespaces must be reviewed before implementation;
neither an older source nor an unknown stage may be cleaned as an absent current.
This first reader accepts ONLY the exact generation1 genesis role. A newer/unknown canonical
head or retained successor stage refuses/preserves; the still-retained bootstrap source cannot
be used as a fallback that rewinds an established group to genesis.
First pin installation is authorized only by the actual original saved-creation capture. A cold
Current reader requires that exact existing group pin; absence/corruption is not permission to
select a new pin from the source/head. Repair needs the complete original activation seed/pin
candidate, otherwise it remains held.

## Exact activation order and interruption ownership

1. Reserve the seed graph/provider/read/control/persistent allowances BEFORE decode or encoding.
   Capture actual saved-source provenance under original admission. FIRST activation proves
   no conflicting v1/v2 group root, local head or incompatible issued/JOIN scope. Exact seed
   repair instead requires its own existing pin/head/row objects or precisely owed missing
   effects; Current read requires its actual complete existing closure. Neither is blanket
   absence or permission to replace a foreign object. Decode/verify privately outside writers.
2. Cache complete sealed pin/head and a bounded activation repair seed ONCE before effects.
   The seed binds the exact source/debit/old vacant scope basis, planned nonce/revision and all
   new frame IDs; it is a concrete multi-effect transaction, not a list of success booleans.
3. Under admission -> pins -> canonical cell -> archive Root, repeat original source, mount,
   creator (warm), scope, target/files and exact pending identities after waits. Durably install
   the repair seed BEFORE the first scope inventory mutation. It may repair only this exact
   known parent/candidate; partial/unknown/foreign state stays preserved.
4. Install the planned Initializing row from this captured original saved creation, then the
   immutable V2 pin and exact head, crossing every actual file/parent barrier. Do not call the
   generic fresh initializer or renew a creation grant. Keep the existing scope row grammar:
   `Origin::Fresh` is historical original-creation provenance, not a runtime Fresh constructor;
   exact provenance is the independently verified original fresh-attempt ID and exact nonce/
   revision is the saved plan. A new private concrete saved-source admission consumer must
   construct that original row through the SAME runtime serializer, bypassing no generic check
   with a boolean. Its supported archive source replaces the old path's negative target fence;
   old generic positive callers continue to refuse this target. Exact-v4 negative recovery stays
   unchanged and cannot retire this v6 origin. V2 identity comes from the typed root/head, not
   inference from this shared local-scope row.
5. Write/sync exact scope Current LAST after the covering head/source/pin closure. Recheck the
   SAME writer/current head/nonce/revision and original negatives; bind/install the retained
   node and hand off OwnedV2GenesisCurrent without an intervening await. Keep the creator's
   original negative life until the owned Current takes over, then retire exact transient slots
   through an actual saved handoff. A saved row alone never creates a head writer.
6. Warm retry reuses the retained candidate/seed/result. Cold repair authenticates all fixed
   current/stage/seed sources and exact scope lineage, performs missing barriers, and issues a
   NEW actual saved activation result only after those effects. It never recreates a warm SDK
   debit, original Fresh or new founder. An unrelated/newer Current/Retired nonce refuses.

Before outward handoff, any ambiguous rename/barrier keeps the entire activation owner closed.
Enter InstallingScope BEFORE the first scope mutation: runtime/dirty-row changes may occur even
when that call returns an error. Never demote that owner to the prior vacant phase. The old
warm SourceSaved/VACANT validator remains strict; it cannot be reused after Initializing exists.
A new concrete activation guard retains the original saved-source capture plus actual saved
activation seed and exact private row preparation, then accepts ONLY its own expected
Initializing/current transition. The physical writer constructs those saved types after their
real barriers. There is no `allow_scope` mode or caller-supplied phase/status to weaken the old
validator. HeadSavedPendingCurrent likewise keeps the actual saved pin/head while scope save is
uncertain; only actual Current barriers advance it to SavedCurrent.
Post-Current lost local return can recover only the same actual saved result or a new exact
cold Current reader, never a fresh initialization. No public frames/network/projection are
exported by this transaction. Future native registration must capture the real UI generation,
mounted Store and exact registry incarnation before awaits and publish through their original
commit barrier. An App Current object does not claim that native registration occurred.

## Resource and regression gate

The source can be8MiB+metadata; it is outside the512KiB archive candidate invoice. Warm seed
decode overlaps the old original SDK graph, cached S0, source read/open buffers, new provider
and new typed graph. Charge each independently; graph leases remain LAST in both device/group.
Bounded public/genesis validation and App metadata have separate scratch. Existing scope
inventory saves can reach32MiB, so this is NOT an all-4KiB transaction: their real existing
encoding/sealing/backing overlap also needs an explicit admission. SDK mutation remains refused
after Complete; live50-member operations and accepted-index/governor support are separate work.

Required actual tests: original SourceSaved -> Current and exact cold reopen; every seed/pin/
head/scope barrier including lost return; warm RNG used once; incomplete/wrong raw S0 or page/
Profile/signature/parent/App name; v1/v2 kind collision; same-label foreign mount/current writer;
removal/lock during decode/final wait; partial/unknown stages preserved; independent graph/source
fee lifetimes and second-group progress. Reopened genesis is not Fresh, and every legacy/v1
mutator or raw run escape must remain closed on this new owner. All tests remain to be authored
and executed; this document creates no Current type or readiness evidence.
