# Retained bootstrap signatures

The actual Root founder source now retains its Profile/Header signing state before
either key call. This is a prerequisite for native key admission, not native
admission itself. Chat V2 remains disabled.

Previously `SourcePhase::PendingPrivateStage` called Sync's stateless
`prepare_owner_v2_founder`, then consumed its result in `freeze`. A refusal after
Profile signing, or an unwind before successful freeze, could discard the signed
records while leaving that same pending source eligible to prepare again. Native
session checks between Profile/Header would make that loss especially important.

Sync now supplies a non-Clone `RetainedFounderSigning` from the actual original
ChannelSync. It retains receiver-only original Fresh/scope observations, the epoch
authenticator and separate Profile/Header slots. Each slot binds purpose, full
signer and exact canonical signing bytes before entering the primitive. Returned
raw64 is cached immediately. Entered without a returned raw remains terminal
uncertainty; error or unwind never resets it. Cached raw rebuilds signed DATA
through ordinary signature verification, never another key call.

The actual origin, scope allocation and authenticator are checked before each
signature callback and before returning prepared DATA. Equal public labels cannot
adopt a replacement node or reservation. Invalid cached signatures remain retained
and refuse reconstruction. These observations still grant no Store, current
membership, native session or publication authority.

Root installs a separately paid `Signing` phase before calling this producer. It
borrows that phase through both preparation and consuming freeze, replacing it
only after the complete boxed FrozenFounder exists. Refusal/unwind can discard
temporary prepared DATA while retaining the raw cache and fee. A retired source
still refuses retry; retention never reopens its negative controls. This intentionally
replaces the earlier freeze-error behavior that discarded the source's partial
signing state into HeldConsumingRefusal.

The extra 64KiB logical invoice covers raw slots, original observations, metadata
and return overlap. Existing creator/provider/frame/snapshot fees remain separate.
The original stateless public Sync API remains compatible for library callers.
The guarantee is once per retained original; Root owns only one such signer for
its source phase.

Focused tests exercise post-Profile and post-Header refusal, opaque primitive
unwind, foreign nodes, equal-label reservation replacement, corrupted cached raw,
stateless compatibility, Root pre-freeze unwind, retired-source refusal and
signing-cache pressure on the same staged owner. Actual primitive-entry counts
are pinned in the Sync tests. Root composition tests retain the same source phase
and verify genuine output; they do not independently instrument the lower key.

Local Windows execution passes six Sync and three Root retained regressions,
eleven surrounding Root source cases, fifteen stateless Sync cases and eight warm
factory cases. Strict Sync Clippy, formatting, ambient and raw-fs gates pass. The
complete App lint inventory reports zero diagnostics on this increment's new or
added Rust lines; inherited workspace strict-Clippy debt remains. The focused CI
lane pins the six/three retained cohorts separately. Complete current-source
Root/native/frontend validation remains pending; this is not activation evidence.

Still required: irreversible NativeRequired origin; original synchronous Tauri
capture and concrete device/SDK/Profile/Header/member key issuers; actual Ends and
lock drainage; native index birth/admission; backend installation and cold/joined
ownership. **Both combined bootstrap preparation APIs must be unavailable to
NativeRequired** until distinct concrete Profile/Header cuts are connected. A
retained cache or ready flag must never stand in for native authorization.
