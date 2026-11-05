# Critical catalog read pool (5 November; authored, unexecuted)

Current extension: an explicitly configured private Pending-only JOIN1 handler now has a larger
actual shared invoice and original fixed-file producer. Normal/Empty readers keep this document's
Empty-only behavior and refuse the new catalog-v2 tag. No ARCHIVE2, network/ready or native
activation follows. See [Pending checkpoint](CHAT-PENDING-JOIN-CATALOG-CHECKPOINT.md); tests remain
UNRUN during the compilation pause. The earlier Empty prerequisite below remains unchanged.


This is a narrow App-layer resource owner for configured catalog opening. It does not authorize
a namespace, create owner/Fresh/Current/Saved evidence, activate native Chat V2, or measure process
RSS/SDK/KDF heaps. The current v6 handler remains generation1/EMPTY only. No compiler, tests or
builds have run during the user pause. Five new regressions are authored and UNRUN; direct
formatting/whitespace checks are not executable validation.

## API and accounting

`CriticalCatalogReadPool` is a clonable handle to ONE `Arc<Mutex<State>>` containing immutable
caller-provided hard byte capacity and checked charged-byte count. Clone shares that counter;
it cannot copy credits. Construction rejects zero/unsupported arithmetic. A private
`CatalogReadReservation` is non-Clone and owns a strong reference to that SAME pool plus
its exact charged amount. It cannot outlive its account or make capacity disappear with a Store.
Private `try_reserve` checks addition and hard capacity before allocation. Poison/overflow/refusal
cannot grant capacity. Drop releases once only after owned buffers/copies are gone; a poisoned
or underflowing account remains closed instead of manufacturing free capacity.

The actual configured entry is `ServerStore::open_with_catalog_read_pool(dir, secret, rng, pool)`.
It retains that explicit pool handle for subsequent actual catalog readers. Ordinary `open` stays
on its existing fixed512-byte Empty-only policy and MUST refuse future nonempty namespaces; no
saved budget field or unknown catalog content can choose a larger trusted quota. A future native
host must provide its actual shared pool before using the configured opener. That process-global
owner, UI generation and host admission are Root-owned later integration gates, not this slice.

The present configured Empty reader uses the already reviewed enumerated8KiB logical metadata
working allowance: bounded raw/unframed/unsealed data for both sources, file handles, key/domain
copies and fixed codec state. It admits that reservation BEFORE the first raw frame allocation.
Both fixed sources are classified under the same reservation. No plaintext is retained for an
Empty namespace. If future supported JOIN1/ARCHIVE2 handlers retain dependent copies or metadata,
their actual larger invoice must be admitted before allocation and remain with those objects;
the transient fee cannot be returned while a surviving copy still relies on it.

This is explicit logical accounting for named App catalog objects, not a generic byte multiplier
or a claim that ciphertext length bounds SDK heaps. The larger JOIN1 payload/body grammar and
its exact read/encoding/provider/candidate overlap invoice still need their own reviewed consumer.
ARCHIVE2's descriptor remains at most4KiB, with mutable roots in separate fixed authenticated
records; it does not cause catalog rewrites on ordinary message append.

## Ordering and failure

Physical-session/orphan checks and actual wrapper authentication retain their current order.
Before allocating catalog bytes, read only bounded fixed-name type/length metadata to determine
whether a source may exist; absence requires no frame allocation. If either may exist, a configured
opener must acquire its original pool reservation before reading/unframing/unsealing. Both current
AND stage parse before any old inventory gate/cleaner. Unknown/nonempty/corrupt source refuses
with all bytes preserved. Pool exhaustion is resource refusal, never evidence that a journal or
source is absent, and never permission to initialize Empty or create a new DEK.

The reservation remains alive through source classification and any actual dependent copies.
For the present Empty-only transaction all catalog buffers have dropped before the constructor
returns; the fee can then drop. A still-open file handle is retained only until its actual sync/
root/move barriers finish. Caller and cloned pool handles remain separate from the non-Clone fee.
No writer/join/archive support is enabled by supplying a larger pool.

## Authored regression packet (UNRUN)

* Same pool clone sees exhaustion while a fee lives; distinct caller pools do not alias. Exact
  capacity, overflow, zero, poison and one-time Drop handling fail closed.
* Real configured Empty-catalog open reserves the original pool before byte allocation; a held
  fee makes a competing configured open refuse without changing any wrapper/catalog/inventory.
  Releasing the fee admits the same original source with no fake Saved/status argument.
* Two different physical vaults opened through clones of ONE pool compete for that same budget.
  A bounded after-reservation/before-read barrier proves actual reader overlap and safe release.
  The second vault is authenticated before blocking the first reader, so the assertion measures
  the shared account rather than timing Argon2 under load. Constructor exhaustion is separately
  exercised through the real public configured opener before its first read-buffer entry.
* Default open preserves its prior Empty-only behavior; configured open still refuses every
  nonempty/unknown stage/current and never erases them or enters an older cleaner.

Read-only design and actual-diff reviews are clear for this narrow boundary. All new regressions remain UNRUN until the
user resumes compilation; direct rustfmt/whitespace checks are not runtime validation.
