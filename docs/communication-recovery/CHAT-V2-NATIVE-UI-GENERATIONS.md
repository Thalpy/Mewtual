# Canonical native UI generations (in progress; not native activation)

The native production branch implements a private Root UI-generation kernel.
It is not yet consumed by the owning NativeV2Vault/founder operation or Tauri.
It authenticates no window, user, row, group, membership or SDK operation. Chat V2
remains disabled.

## Ownership and memory

One affine UiAdmissionOwner owns the original RT Host binding outside physical
work. Whole-vault close sets that SC-only Host fence before index admission is
closed or any registry/operation guard is acquired. UI lock retains the mounted
Host and retires its canonical generations instead. Workers receive weak commands
and paid negative records, never an affine binding or source-close owner.

NativeUiInvocationBudget reserves4096 logical bytes for EACH transient UI
candidate before original-vault capture. The metadata credit is non-Clone and
consumed once. Infallible capture pins only original Host metadata, the captured
generation and the fee. Finish allocates/checks the UI counter outside all guards.
Competing candidates are independently funded; the loser remains in its caller's
outer slot for destruction after guard release. Native must independently fund
its argument buffers, task/IPC state and source operation; this fee is UI metadata.

Canonical admission checks the original registry, actual reservation pool and
captured generation again. At most32 generations retain canonical close/drain
slots; this is unrelated to the number of members or messages. An open generation
is shared by its commands. GC never lowers the highest admitted generation, so
retired/pruned generations cannot reopen if host atomics are rolled back.

Host payment follows actual Host pins. Generation payment follows the actual
Counter through prepared cuts, Ends and drains. A record holds the fee and a drain;
the Counter holds only the fee, avoiding a record cycle. CLOSED+zero ends admitted
key work, not metadata/worker/SDK/physical custody. Unentered pins can retain the
payment after canonical slot GC. These are application invoices, not RSS bounds.

## Retirement and failures

Every issued generation already owns its retirement record. Lock needs no new
fee or slot, even at both generation and memory caps. A fixed stack batch moves
close owners out before any notification or wait. Started returns custody only
when at least one distinct owner/payment moved; AlreadyRetiring returns no record
custody or extra waiting task. Native must coalesce those callers in its owned
retirement task. KeyWorkDrained is not a completed UI lock or physical drain.

Repeated replies must not clone temporary record references: concurrent final-End
and GC can otherwise make that temporary release a last payment under the caller's
outer native guard. The implementation first moves owners/inspects negatives and
clones records only for a returned Started batch. GC similarly returns its actual
records for destruction outside guards. Error formatting occurs after registry
and poisoned guards have dropped. No lock path allocates a Box for batch storage.

Eight actual-Root tests cover independent competitors/canonical sharing, original
registry/pool/generation rejection, transient quota, full32 generation slots and
exhausted memory still permitting retirement, real payment retention in unentered
pins, same-generation refusal after GC, owner-drop before finish and synchronous
Wakers observing an unlocked registry. The additional concurrent final-End/GC
regression uses the real reservation's drop observer and the production collector;
a repeated caller cannot become the last payer under its outer guard. The eight-case run also passes after replacing the large tagged return with
fixed stack result storage.
RT's18 lifecycle cases also pass. Read-only review corrected the retirement race
and locked error allocation; no B/H/M/LOW remained in the reviewed kernel.

## Required consumers and remaining acceptance

The actual native factory must supply real AppState SC atomics, capture generation
before parsing, resolve its permitted main window and original mounted backend/row
before dispatch, and retain the original stable row identity across awaits. A
numeric ID or DATA receipt must never select a replacement backend. Native-owned
phase2 work must cache outcomes before custody release, coalesce bounded lock
waiters and drain outside every native/registry/Store/Root guard. Public combined
RT open_session is outside-guards only. None of those Tauri paths is implemented
by this kernel.

The native index cell/Pending/network/founder consumer, mounted/backend sum, cold
operational ownership, Leave/backup, joined pin/Add/archive/transport and UI/admin/
combined acceptance remain unfinished. Native index2 currently models new founder
births/activation only; a joined backend needs an explicit reviewed storage
contract, not reuse of Legacy or an unversioned extra variant. UI-authored key
checks must not accidentally disable authorized background protocol work on lock.

Strict App Clippy is not green. Its latest pre-follow-up inventory has574 errors,
including the unconsumed kernel and existing integration debt. The measured large
enum issue is fixed with stack result storage; no lint suppression
or fake public caller is permitted. Complete exact-source suites, strict checks,
dependency findings and actual native integration review remain required.
