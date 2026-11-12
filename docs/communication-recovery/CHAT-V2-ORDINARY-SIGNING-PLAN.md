# Exact ordinary Draft, signing plan and original raw custody

Local Core checkpoint, 7 November. The revised children and parent are now declared.
All 252 Core chat tests pass in 97.64s after a 19.17s compile, including the eleven
new Draft/signing regressions. Independent actual-source review is clear after the
capacity and test-lifetime corrections below. No actual device caller, Root member
Source, durable outbox, transport or native/UI dispatch is implemented or enabled here.
Strict all-target/all-feature Core Clippy passes in 12.99s after a private enum
naming correction; actual-source re-review confirms no public/wire change.
Complete backend/frontend, strict-root and native validation remain separate gates.

`ActionDraft` owns an immutable validated `ActionSpec`, full signer, canonical unsigned
action/v1 bytes and private original identity. It is unsigned DATA, not authority. Existing
signed actions and Drafts share the same private unsigned validation, semantic encoding and
minimal dependency-label functions. No dummy `SignedAction`, signature-check bypass,
different wire domain, mutable getter or serialized 'verified' status is introduced.
Draft construction also refuses spare text backing above MAX_TEXT_BYTES and parent capacity
above MAX_PARENTS before canonical allocation or any plan claim. These are Draft-only custody
limits; legacy SignedAction length validation and serialized bytes remain unchanged.
`complete_raw64` runs outside writers and verifies the actual retained signature using
the existing `SignedAction::sign_using` path with a closed no-key callback. Invalid raw bytes
never create a signed action. The immutable Draft and raw remain available on refusal.

Only `V2GeneralPrefixVerifier::prepare_ordinary_signing_plan` issues an opaque non-Clone
`V2OrdinarySigningPlan`. It shares the signed issuer's private unsigned authority kernel:
exact selected Admission/private verifier identity, full subject, derived realm delegation,
operation rights and canonical minimal direct parents plus target. Each dependency still
passes its real action signature and actual selected Admission binding. Historical signed
issuance separately verifies the original action signature before the shared kernel.
Pre-key issuance additionally requires that SAME actual Admission to be current and OPEN,
and binds the original group, channel view and source lifetime. No raw roster or negative
owner alone can issue the Plan. A Draft's monotone private claim permits only ONE Plan
lifetime, including after Plan Drop; equal bytes in a different Draft do not adopt it.

The Plan's original observation is a small Weak identity, exact Draft identity and fixed
binding DATA. It retains no model, SDK, source sender, key or Draft text. Positive use still
requires the actual future Root native/Source/UI generation and real device key comparison.
These checks must precede cached-raw reuse as well as first key entry. Root must move the
whole Plan and Draft into owning paid request custody before fallible signing work; it must
not replace them from a token/label, or prepare a second Draft for an uncertain same request.

`admit_signing` checks exact original Draft and full device key, takes the SAME original
group reader, then repeats the original negatives inside it. `begin_key` consumes that guard
and marks `KeyEntered` before returning the immutable signing bytes. All fallible Source/UI,
SDK lookup, fee, shape and request checks must already be finished. No filesystem, pool,
codec, SDK lookup, await or arbitrary callback belongs after final admission; only the
bounded real key operation belongs at that edge. Dropping/unwinding key entry without raw
leaves the original terminal phase: another admission cannot call the key again.

`cache_raw64` infallibly moves the returned raw bytes into the same Plan and returns the
still-admitted reader. Root/Sync retain that reader until their outer packet cache is safe,
then drop it before outside materialization. Raw is unverified DATA until completion verifies
its exact Draft/full key. Removal/fork may select/close immediately without blocking on a
writer; their existing group-writer completion waits for an already-admitted signer/raw
cache. A later Stop cannot erase that historical output, while all fresh admissions refuse.
Pure exact-original raw borrowing/completion remains legal after Stop and renews no rights.

Draft complete-result overlap has a separate named `private_layout_bytes` inventory; Plan,
identity/weak/guard slots have separate `metadata_layout_bytes`. The host must independently
fund every input/result/Plan copy before allocation and retain original admission/control
tail funding through escaping observations. Replay1MiB, SDK/provider, Root Source/UI,
ledger and outbox costs provide no implicit credit. These are source-level inventories,
not actual payment, allocator/RSS claims or proof that an existing configured cap fits.

`V2GeneralPrefixVerifier::ordinary_authorization_temporary_layout_bytes` separately names
outside-writer transient verification: at most65 real dependency signature/delegation/ID
checks run sequentially; canonical encoding/hash scratch is not65 simultaneous action
clones. It includes the required-ID Vec's possible128-slot capacity plus old64-slot growth
overlap, and at most65 covered Basis keys with conservative logical tree-node/backing
allowance. Actual Root must independently debit that transient invoice or explicitly pay a
proven exclusive phase in an actual account. Draft/Plan/replay fees provide no silent credit.

Four canonical Draft tests and seven actual general production-issuer tests are authored. They
cover unchanged Create/reply/Edit/Delete/Join bytes and IDs, bad raw signatures and unsigned
shape, small valid overallocated Create/Edit text and parent rejection, exact backing bounds,
and actual oversized owned legacy SignedAction compatibility; exact original Draft/full key/realm/dependency refusal; real selected Admission and
minimal signed dependencies for all operations and an actual64-parent plus target65-signature
boundary; once-only key/raw retry; both Remove versus
signing orderings and original drain; A1/A2, source/channel closure; terminal lost key return
and body-independent signed fork during admitted key work. No fixture authority constructor
issues any positive witness. These tests do not prove native authorization, real MlsDevice
dispatch, an at-rest save, multi-peer network delivery or cross-crash signer exactly-once.

The next actual bridge needs a distinct Root warm-member owner. The existing
`ChannelSync::with_registry_context` can lend the actual device without another generic
Sync signing API; `matches_registry_instance` checks the actual node incarnation. The real private device stays at `ChannelSync.device`; retain/borrow
that SAME whole warm Server/native owner rather than extracting or duplicating the graph.
These borrows supply neither source provenance nor payment. Root must authenticate the
independent V2 pin/governing prefix and exact local Admission,
original generation and command/UI credential before passing the admitted plan to the key.
No owner Header/I/B/WC or channel exporter is needed for signing-only. Existing Genesis-only
Source and v1 joined checkpoint cannot be relabelled to supply that production boundary.
Independent durable outbox and current-epoch envelope attempts remain later reviewed slices.

Amendment, 9 November: the founder-as-ordinary-member vertical uses the founder's whole
genesis node only under the narrow conditions in
[CHAT-V2-ORDINARY-MEMBER-SIGNER](CHAT-V2-ORDINARY-MEMBER-SIGNER.md): unchanged founder
`source()` check, pin-authenticated genesis anchor, selected O still the empty
generation-1 promotion (re-proved at every key cut), and lane exclusivity with founder
Create. That is not a general relabelling; any later governing prefix refuses.
