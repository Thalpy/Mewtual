# Whole native text Source retirement (under validation)

Chat V2 is disabled and incomplete. This increment provides Root cleanup of its
original Source collection. Native still has to retain that collection in its
canonical controller, schedule cleanup, dispose the originals outside guards,
and order the result against actual lock/close/publication. No native command,
SDK restore, joined authoring or network acceptance is supplied by this API.

`NativeTextSources::capture_retirement` permanently closes the original Scope
and packed preparation admission, then captures one prepaid work slot using
atomic/Weak operations only. Its outside-guard `finish` closes every actual RT
source counter and snapshots the original rows before body work. The existing
`prepare_retirement` is an outside-guard wrapper over both stages. Completion
also requires that same original preparation counter CLOSED+zero, even with no
rows. [Capture continuation and pending evidence](CHAT-V2-NATIVE-SOURCE-CAPTURE.md).
The capture contains at most 64 original runtime-cell pins and key drains. It
contains no Entry, affine SourceOwner or issuer. Its cell pins are **work
custody**, not negative-only observations: cells can still own SDKs and Physical.
The slot acknowledgment is last, after those pins leave. The fee-only control
contains no Source/physical backreference. The collection's existing control
invoice now explicitly includes both capture envelopes and that allocation.

`try_retire` makes a finite pass outside native/Root writers. Each cell is
monotonically marked retiring under the same mutex that loans its body. A
creation preparation captured earlier cannot borrow a body after this mark or
after the permanent Scope closure. Original key CLOSED+zero does not establish
that a worker returned: `running` must also be false before taking the body.

The actual whole founder driver closes its creator, Host owner, projection and
member domains and drains already admitted work. An independently retained
member-save source End keeps this close pending. A partial close or Busy restores
the same mutated body with `RuntimeLoan`; cleanup does not discard it or build
another SDK. Other rows can still make progress on that finite pass.

Before destruction, the cell caches the original TCP disposition. `NotPrepared`
is recorded only by examining a genuine present Runtime. Otherwise it retains
that Runtime's original fee-only `NativeTcpDrain`. An unused TCP preparation
still owns a prearmed constructor token, so it must be destroyed before waiting
for that drain. A missing body, missing TCP evidence or failed task cannot supply
either proof.

A dedicated destruction envelope owns the actual whole Runtime as its first
field. Its final field records body destruction only after Rust drops all actual
Runtime fields, also while unwinding a field destructor. It records normal
return and unwind distinctly. The outer loan then clears `running`; an ordinary
loan return or generic worker guard cannot manufacture body-destruction proof.
The original cell retains these facts across a lost worker or collection reply.

Completion requires every original key counter CLOSED+zero, no running loan, an
absent body with genuine destruction evidence, recorded SDK closure, and the
same TCP disposition complete. It does not mean the caller destroyed Sources or
the facade. Native must keep the original canonical collection until collecting
this conjunction, and destroy the originals outside all guards afterwards.
Timers may pace retries; they are never evidence of drainage. A replacement
collection, fresh mount, equal DATA or an empty runtime slot cannot clear old
custody. UI lock remains a narrower operation and does not destroy Sources.

Nine new regressions passed in the full 46-case affected source cohort (161.03
seconds, zero failures or ignored cases). Read-only design, actual-diff and short
follow-up reviews found no remaining blocker/high/medium/low. The review's
last-control-payment gap was added and passed. Tests cover the one-slot capture, an unentered whole
loan despite idle key counters, separately retained key work, missing body/TCP
evidence, closure after key preparation, actual retained RNG destructor unwind,
real original TCP custody and vault exclusion after a lost collection, and an
actual whole SDK blocked by its member source End. An empty capture outlives the
Sources/facade, retains its exact control charge, allows the actual vault to reopen,
then refunds that charge only at capture Drop. The member-End fixture bypasses
request admission solely to isolate its real close dependency; it proves no
native command or save authorization. Ambient-dependency/raw-filesystem and exact
inventory guard self-checks passed. Complete current-source Root/native/frontend,
lint/audit and native lifecycle evidence remain pending.

The TCP-only constructor's DNS/WebSocket/QUIC and mapper limits remain as stated
in [the transport contract](CHAT-V2-NATIVE-TCP-LIFETIME.md). Wider Chat V2 gaps
include checked native create/member commands, joined-member sources and Add,
multi-member archive, governing/text transport, live projection/admin history,
operational cold SDK restore and combined adversarial/network acceptance. This
Root cleanup increment does not close those requirements.
