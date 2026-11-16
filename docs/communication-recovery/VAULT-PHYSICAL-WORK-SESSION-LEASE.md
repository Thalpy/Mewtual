# OS-session lifetime for admitted physical work (authored prerequisite; 5 November)

The vault installation lock originally belonged only to ServerStore's VaultSessionGuard File.
An admitted blocking source write can outlive its Store/creator. Closing the last Store before
that syscall drains releases `.vault.session.lock`; a replacement mount can then mutate/repair
the same physical namespace concurrently with old admitted I/O. Cancellation cannot retract a
kernel syscall, so an observer alone cannot close this gap.

## Minimal resource-only shape

The guard's private File backing is now one Arc<File>, acquired exactly once by the existing
`acquire_vault_session`. A non-Clone `VaultPhysicalWorkLease` has only another reference to
that SAME Arc<File>, returned by `VaultSessionGuard::retain_for_physical_work`. No reopen,
File::try_clone, explicit unlock, path, File/Deref accessor, keys, Store, source, creator sender,
admission, mount or Current capability is exposed. Debug prints only the opaque type name.

The operating-system lock remains on the same original file object until the LAST guard or
physical-work lease drains. The normal no-work path remains exactly guard-drop -> File close.
The lease can be moved into one paid running physical job and remains through that job's final
write/rename/sync and file-handle drain. Multiple admitted jobs can each hold one explicit loan;
new mounts remain nonblocking VaultBusy until all actual jobs finish. No clone method is added.

Inert SourceDebt, cached source/intent/result metadata, observer and caller-wait objects must
NEVER retain this lease. Dropping Store/creator still immediately revokes their original negative
authority; the physical lease only prevents another OS session racing already-admitted bytes.
It grants no right to start new work, publish, install Current or reenter a closed Store.
Actual source workers still need their own prepaid arm/payload/output identity and final checks.

The extra allocation is one fixed Arc header plus File at successful session acquisition; each
loan is one pointer wrapper and an Arc reference increment. Running-work invoices must include
their wrapper. This is not a memory/RSS/global work-slot limit or an I/O-budget grant.

## Compatibility and enforcement scope

No wire, persistence format, lock filename, locking mode or key derivation changes. App open and
cold repair already acquire the lifetime guard before unseal/cleaners. They continue returning
VaultBusy while an admitted physical job owns its loan, even after the old mount is dropped.
Short `.vault.lock` key-vault verification/rewrap helpers preserve their separate existing behavior;
this change does not falsely claim they acquire the lifetime session.

The existing non-hostile live private-directory assumption remains. A hostile local actor replacing
or unlinking lockfiles is not prevented by an Arc loan. Process termination still closes the real
OS file handles; no persistent busy marker or cleanup/unlink logic is introduced.

## Regression shape and evidence

Three storage tests cover ordinary guard-only release, guard-before-loan and loan-before-guard
orders, multiple loans, sanitized Debug, real continued writes and the distinct short transaction.
One App test covers actual Store destruction. A bounded channel barrier suspends a worker
between real writes while it owns the original write File and session loan. The actual Store
and logical owner are dropped, a real ServerStore::open must return VaultBusy before unsealing,
then the worker continues/syncs/drops its file and lease; the successor opens successfully.
Receive deadlines and sender Drop make assertion/setup failure release the worker without a
permanent barrier wait. The worker owns one structural PhysicalWork value whose File fields
precede its lease; that single value is captured before spawn, so setup failure and panic unwind
also drain files before the lease. No sleep or mocked paid/authorized flag is used.

This prerequisite may remain isolated until Root's actual paid source-worker constructor consumes
the loan. A storage resource API/test alone is not evidence that every detached writer retains it.
Design and read-only actual-diff reviews are CLEAR. All four regressions are authored
UNRUN. Compilation, compiled tests and builds are paused; no execution/readiness claim is made.
