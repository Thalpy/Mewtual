# Native authentication without retaining its physical slot

Status: code under focused validation on the disabled native path. Chat V2
remains off. This is a prerequisite for typed UI continuity, not its completion,
working text send, operational restore or SDK/native authorization.

The previous warm authentication capsule held the only original MetadataWork
slot until finalization ended. Finalization must eventually save pending UI
continuity on that same mount; holding the slot would make that retry busy.
The verifier now returns an independently paid cached-negative observation,
releasing the whole original work before finalization uses that observation.

## Capture, verify and finalization

Native pays one unique NativeUiInvocationMetadata before acquiring Store. Root's
prepare_native_authentication is infallible: it returns the credit, original Host,
original expected pool and optional original physical work in one opaque capsule.
Busy and version2 captures encode absent work. Closed state always refuses
outside the guard; a Host that closed before its index can still capture work.
The capture does not refund or
format/map an error while the native Store guard is held. All original custody
is retained until outside every guard, including refusals and unpolled tasks.

Verification runs on the blocking pool. It validates that unique credit against
the captured ORIGINAL pool before KDF or missing-work refusal, captures the exact
original gate and physical lifetime negatives, then verifies the same critical
key hierarchy as before. It destroys WHOLE MetadataWork outside every guard,
including the original slot acknowledgement after physical destruction, before
rechecking and returning NativeTextVaultAuthenticationCheck.

The actual native authentication helper now returns this Check and retains it
outside the existing finalizer's guards. is_live_cached reads only original
atomics. UI lock is not whole-vault closure; native's original generation, ACL,
voice-ticket and canonical admission checks remain independently mandatory.
The Check grants no UI, row, source, cryptographic or SDK authority.

## Narrow original negatives

The gate observation holds only Arc<Gate>, which already retains its original
payment. It retains neither Handler.cold nor physical/session evidence. The
physical-mount observation retains only the original Arc<Life> sampled directly
from that work's Store. It never looks up a replacement by labels, upgrades a
Weak, retains the revoking Owner/Sender/Notify or invokes a Drop callback.

Revocation Owner now owns a separate atomic-only Life allocation. Every existing
revoke/Drop path writes this SAME atomic before notifying, so a cached observer
cannot miss ChatMountRevoker or defer cancellation. The original Owner and watch
Sender still die at the original boundary while the cached pin survives. Warm
and all three cold funded-owner metadata formulas explicitly include Life plus
its Arc header and conservative alignment word; sizeof Owner alone would omit this additional heap allocation.

The Check is non-Clone, opaque in Debug and has no serde, key, Store or identity
getter. Unique invocation credit is last, after original metadata. Its fixed
4KiB ABI assertion covers capture/check/temporary metadata overlap. This invoice
does not claim to cover KDF working memory, native IPC copies or browser memory.

## Verification status

Four real selected3 Root regressions passed (latest16.27 seconds), alongside
all five revocation tests, including the two retained-Life cases. They cover
slot reuse while Check survives, real session release and strict3 reopen after
facade destruction, original-pool refusal before wrapper verification, Busy/
version2 credit retention, independent Host/gate/mount revocation and the original
Sender/Owner boundary. All19 actual native routing/lifecycle tests passed in
39.52 seconds using the new Check through the real native authentication helper.

Full frontend3441 tests, Svelte check (zero errors/warnings), web build,
ambient/store raw-fs gates, guard self-tests, root/native formatting, strict
native Clippy, separate native check, both dependency audits and actual native
build/startup/flow checks passed. The final conservative alignment word was
additionally checked by cargo check -p catcoms-app and the actual native build.
Root strict Clippy was run and fails with582 errors in the unfinished App
pipeline; no suppression or passing sign-off is claimed. Full Root/native CI
execution remains pending for this checkpoint; only the previously named slow
catch-up cost case may be deferred until final acceptance.

The design and actual-diff reviews are clear after correcting a fallible-capture
refund trap and independently pinning Sender closure with a retained Life pin.
The scoped checks above do not establish completed messaging or full acceptance.
Typed UI read/save, mount-bound pending snapshots and original-lock continuity
retries still need implementation. Operational creator/member/cold restore,
joined archive/source, transport/projection/admin and multi-peer acceptance also
remain open.
