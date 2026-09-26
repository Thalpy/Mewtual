# Retained chat across a restarted bridge

`crates/catcoms-app/tests/temporal_bridge.rs` adds a bounded T37 integration fixture around
the existing production actor, authenticated history exchange and sealed `ServerStore`.
Execution results belong in the integrated verification record; adding this fixture alone
does not mark T37 or CR07 passed.

The scenario uses three independently generated MLS devices and three separate vault
directories. A admits B; B's explicit expiring helper capability carries C's admission to
A. The transport denies A–C connections and requests throughout, including admission.
Every gossip publication is discarded, so later convergence requires actual signed
request/response exchanges handled by each node's ordinary actor loop.

C saves its own state and goes offline before A creates a new channel. B learns that
channel's directory entry and history through ordinary discovery and reconciliation,
without a test-issued channel-open or channel-specific catch-up command. After B's sealed
checkpoint, A exits and B's actor is aborted without a shutdown snapshot. The next phase
uses a fresh Hub, transports, actor owners and vault sessions. Only B and C return, and
each loads its own sealed state. C must discover the previously unknown channel and
recover the original message IDs, authorship and content from B. C then saves and reopens
offline to establish its own retained copy.

The application already opens shared-directory channels independently of the selected UI
tab, retains signed operations in its snapshots, and serves them through member-verified
catch-up. This fixture connects those existing paths rather than adding an alternate
forwarding protocol or replacing original authorship with bridge-authored messages.

The scope is intentionally limited:

- Checkpoints are explicit host snapshot writes. The test does not establish automatic
  native receive-worker timing, a crash before the checkpoint, or fsync power-loss semantics.
- Actor abortion and fresh vault sessions occur inside one test process. They exclude
  live actor state and old network queues, but do not replace an OS process-kill test or
  three-device physical-network acceptance.
- The retained content is chat in a discovered channel. Blob custody, partial retention
  advertisements, opaque forwarding, multiple successive bridges, alternate-bridge
  failover, and membership/key changes during the offline window remain separate work.
- The one-time admission-helper grant is explicitly installed for the fixture. It does
  not demonstrate automatic pre-member forwarding or create standing socket authority.

Adversarial fixture checks: C's first vault predates both the new channel and its messages;
B must show the exact history before persisting; all original actor tasks and the Hub are
destroyed before recovery; the restored B can read history before C exists; C is proven
empty before its actor starts; and the request trace admits only chain edges. Snapshot
bytes never move from one device to another. Expected message projections remain solely
assertion values and are never supplied to a receiving actor or store.
