# Mewtual provider backing observation

This directory starts from the exact crates.io `openmls_memory_storage` **0.6.0** archive selected
by both workspaces. Archive SHA-256:
`ce927945a16eaa19302ed5ccef052f01ba68f2ba7ae67e4ac4ff2fc661445364`.
All eleven packaged source, manifest, lock, README, changelog, test and VCS files are retained.
The registry's local `.cargo-ok` extraction marker is not part of the upstream package.

The package's VCS source is OpenMLS commit
`3a3e35de3feeca8f6605143c464d5452ae584d43`, `memory_storage/`. The archive did not contain a
license file. `LICENSE` is the original MIT notice from
<https://raw.githubusercontent.com/openmls/openmls/3a3e35de3feeca8f6605143c464d5452ae584d43/LICENSE>.
The upstream copyright and MIT licence remain in force; this is not relicensed as project code.

The deliberate local changes are:

- Normalized `Cargo.toml` adds exact `hashbrown = 0.16.1`, with default features disabled. That
  version is already in both existing workspace locks. Offline dependency metadata now resolves
  this exact local provider and the shared Automerge patch in both workspaces, with no package
  version update. This is manifest/lock validation, not compilation or feature-path execution.
  No hashing implementation is selected through hashbrown's
  optional default hasher.
- `src/lib.rs` exposes `StorageMap<S = std::collections::hash_map::RandomState>` and substitutes
  that alias only for `MemoryStorage.values`. The optional test-utils decoder initializes the
  same map with `StorageMap::default()`. The generic parameter permits deterministic private
  preflight tests; the production provider always uses randomized `RandomState`.
- This provenance file and the missing upstream licence notice are added.

Every upstream StorageProvider operation, label, JSON serializer, lock and error behavior is
otherwise byte-for-byte retained. `persistence.rs` intentionally keeps its separate
`HashMap<String, String>` JSON container. Existing storage keys/values and MLS signing/wire formats
are unchanged. Iteration order was never canonical across different HashMap instances. The bounded
and ordinary encoders must produce identical bytes from the **same guarded live map**; old snapshots
must restore their logical identity/group/key state.

Why this narrow patch exists: `std::collections::HashMap::capacity()` reports insertion capacity,
which can fall after tombstone deletion while iteration still scans the original allocation.
The safe `hashbrown::HashMap::allocation_size()` implementation in **this exact pinned version**
returns the actual table Layout size, including buckets/control bytes and inline Vec headers. The
caller checks it under the original read guard before the first iteration, then independently
checks output/key/value lengths and actual Vec payload capacities while holding that same guard
through encoding. Payload capacity must fit a separately charged checked local cap of twice the
configured output cap; this is a policy refusal, never an assumption that the SDK allocates 2x.
A future hashbrown
upgrade must re-review that implementation and the adversarial tombstone tests.

Table backing excludes Vec payload allocations, SDK JSON/group heaps and whole-process memory.
Those need separate resource custody; this observer does not make a legacy restore automatically
bounded and does not authenticate a membership, canonical head or saved result. Production owner
activation still requires the bounded restore/SDK consumer, covering membership transaction and
scalable public archive. No tests or compilation are implied by this source/provenance record.

The canonical provider allowance covers private initialization/restore overlap only. Successful
handoff releases that transient reservation; active `OwnedOwnerServer` memory needs separate host
runtime admission before production activation. This patch does not create that active host.
