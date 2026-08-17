# Multi-device identity; design (v2.2)

Status: **M1–M6 implemented (M2 & M3 adversarially reviewed, BLOCKING findings fixed).**
M1 primitives (`f6b7386`); M2 ceremony (`fe618d3`); M3 admission + M4 UI (`ba8a8d1`); M6
QR/audio carry channels (`bcd5e17`); M5 device revocation lands with this doc update.
**The one deliberate deferral:** `MasterHandoff`'s primitive is committed but **inert**;
*consuming* it (per-group master state + monotonic `master_seq` enforcement + a transfer
command) is a scoped follow-on, since the common flows (add a device, revoke a device,
kick a member, recover a lost origin via owner re-admission) do not need it. So "the master
is transferable" is designed and the crypto exists, but the live transfer path is future
work, called out here rather than implied complete.

v1 (user-keypair + device cap)
was reviewed by the project owner on 2026-08-15 and simplified: **one device per grant,
single-use**, the **origin device is the identity root** (no separate user keypair), one
**all-server grant bundle** per ceremony, and an explicit **grant-confirmation popup on
the origin device** as the human gate. v2.1 added pre-confirm gating, the transferable
master, and three-tier loss recovery. v2.2 records the M2 adversarial-review outcomes:

- **The pre-mint comparator in the offline ceremony is the device code** (8-hex, 32
  bits), shown on both screens; the new device *cannot* compute the SAS before the
  bundle arrives (the SAS binds the origin id, which it doesn't yet know). The **SAS is
  the post-delivery check**: `open_grant_bundle` recomputes it from authenticated sealed
  contents and the human confirms it matches what the origin's popup showed. The popup
  labels both honestly. (Review finding: the earlier UI headlined the SAS at the gate,
  where it could not be compared.)
- **Certificates are group-bound**: `group_id` is inside the signed payload, so a cert
  minted for one server can never admit into another; M3 verifies scope
  cryptographically, mirroring `InviteToken`. Certificates carry **no expiry**; M3
  enforces `issued_ts` freshness at admission instead (decision recorded here).
- **The gate lives in the backend**: `pairing_read` stores the decoded request as THE
  pending ceremony; `pairing_mint` takes no blob and mints only from that stored view
  (closing a read→mint TOCTOU), and decline burns the nonce.
- The popup **discloses scope** (every server and DM the grant will cover) before
  accept, and the transport passphrase has an enforced 8-character floor (the bundle is
  designed to travel; the vault's interactive-tier KDF alone is not enough for a
  trivially guessable passphrase).

## Where we are (and the trap to avoid)

Today identity **is** the device: `DeviceId` is a content address of the device's
signature key (`catcoms-crypto/src/ids.rs`), `fingerprint(device_id)` is the member id
everywhere (profiles, roles, badges, presence, message authorship), and the MLS group
admits/removes *devices* (`contains_device`, `remove(&device_id)`). One member = one leaf.

**The trap:** "just export the device keypair and import it on the laptop." Two devices
sharing one MLS leaf fork the ratchet; forward secrecy breaks, epochs desync, commits
race; and the devices are cryptographically indistinguishable, so "sent from Phone" is
impossible anyway. Rejected permanently; nothing below ever exports a device key.

## Model: the origin device certifies companion devices, one at a time

- **Device key**: exactly today's per-device signature key; one MLS leaf per device; every
  op/message stays device-signed (attribution comes free).
- **Identity root**: the member's **original device** (per server; see unlinkability).
  There is no separate user keypair: a **device certificate** is
  `sig_origin(origin_id ‖ new_device_id ‖ device_name ‖ issued_ts)`, minted by the origin
  device during a grant ceremony, for exactly **one** new device.
- **Identity for docs**: unchanged; profiles / roles / badges / message attribution stay
  keyed by the **origin fingerprint**. A companion device's ops carry its own signature;
  the client maps companion → origin via the certificate table and renders the origin's
  name/style plus a mono device tag ("· phone") when a member has >1 device.
  **Consequence: no doc re-keying phase exists at all** (v1's M3 is deleted).
- **Chain depth is 1**: only the **master** device may certify; a companion can never
  mint devices (a stolen companion must not be an identity factory, and depth-1 keeps
  verification chain-free). Reviewed and reaffirmed 2026-08-15.
- **The master is transferable, not distributable**: the current master may sign a
  monotonic handoff (`sig_master(master_seq+1 ‖ new_master_device_id)`) moving grant
  authority to one accepted device; the safe form of "elected master". Same
  replay-proof posture as grant revocation (owners reject stale `master_seq`).
  Companion **self-election is rejected**: surviving devices crowning a master without
  the old master's signature is exactly the takeover a stolen companion wants.
  (Veto-delay election schemes are noted as possible future work, not v1.)

## The grant ceremony (one QR, all servers)

1. **New device** generates its device key, then shows a short **pairing request**:
   its public key + a random pairing nonce, as QR / copy-paste blob.
2. **Origin device** ingests the request (scan / paste) and derives a **short
   authentication string (SAS)**; e.g. 6 digits or 3 words; from
   `KDF(new_device_pk ‖ pairing_nonce ‖ origin_id)`. Both devices display the SAS.
3. **The grant popup (the human gate)** appears on the **origin** device; and gates
   everything: until it is accepted, **no bundle exists, no `CTRL_DEVICE_ADD` may be
   emitted, and no server learns a pairing was ever attempted**. The new device holds
   only its own keypair and a single-use nonce.
   The popup reads:
   *"Grant device access? New device `<petname>`; code `738 214`; does the new device
   show the same code?"* plus **context, clearly labelled as context**: the transport
   address the request arrived from ("192.168.1.22; same network as you") and recency.
   **The SAS match is the gate; the IP line is advisory only**; IPs are NAT-shared,
   VPN-scrambled, and claimable, so they inform the human but never authenticate.
   Decline ends the ceremony; the pairing nonce is single-use either way.
4. On confirm, the origin mints one **grant bundle** covering **every server the origin
   chooses** (default: all): per server, a device certificate + the same bootstrap
   material a bound invite carries (rendezvous/relay hints). The bundle is
   passphrase-wrapped (Argon2id) if it travels as a blob; over QR in one room it may ride
   the pairing channel directly.
5. **Admission, per server**: the new device presents its certificate through the
   **owner-serialized add queue** ([`design-admin-invites.md`](design-admin-invites.md));
   a `CTRL_DEVICE_ADD` whose validity condition is "certificate signed by an
   already-admitted member's origin device, not revoked, issued_ts fresh" instead of an
   invite-ledger entry. Single committer, no fork, offline-queued like admin invites.

Unlinkability across servers is preserved: each server's certificate is signed by *that
server's* origin identity; the bundle containing them all exists only on the member's own
two devices, never on the wire as one object.

## Revocation, two verbs (unchanged from v1 in spirit)

- **Member revokes own companion** (lost phone); **implemented (M5)**: the origin signs a
  `DeviceRevocation` and publishes it into the `Devices` doc's revocation map; the owner, on
  reconcile, enforces an MLS Remove of that leaf. A revocation is honoured only when its
  origin matches the companion's **registered** origin, so a member can revoke only its own
  devices (and holds only its own origin key regardless). Re-admission is then doubly
  blocked; the certificate's ledger nonce is already spent, and the device is in the revoked
  set. Companion devices hold no grant authority, so a stolen companion can post until
  revoked but can never mint siblings.
- **Server kicks a member**; **implemented (M5)**: `remove_member` removes the member's
  origin leaf **and cascades to all of that member's companion leaves**, so no linked device
  keeps speaking for a removed member. Owner-serialized like every removal. (The origin-id
  ledger-ban from [`design-grant-revocation.md`](design-grant-revocation.md) still applies to
  the member's re-invitation, unchanged.)
- **Destroyed/lost master; three tiers, in order**:
  1. **The vault is the identity, not the hardware.** The master's signing key lives in
     the passphrase-sealed vault on disk; restoring a vault backup on new hardware
     restores the master (grant authority included) with zero protocol involvement. The
     export/backup UX must say this loudly; vault backup is the primary recovery story.
  2. **Planned migration**: the voluntary master handoff above (old device still alive).
  3. **True loss (no backup)**: companions keep working (leaves and certs stand) but no
     new grants and no revocations can be signed. Per server, the **owner re-roots the
     member**; out-of-band verification (the Verify dialog exists for exactly this) and
     owner-serialized re-admission under a fresh master. For servers the member *owns*,
     ownership transfer (existing backlog item) is the only remaining door; said plainly
     rather than papered over.

## Threat notes

- **Stolen grant bundle**: at most **one** device, only until the popup is declined;
  and the popup + SAS ceremony happens *before* the bundle is minted, so a stolen bundle
  alone (without a completed ceremony) admits nothing it wasn't minted for. Passphrase
  wrapping covers the blob-in-transit case.
- **SAS is the authenticator**; the popup's IP/recency line is context. Never compare
  IPs as proof; say so in the UI copy.
- **Ceremony replay**: pairing nonce is single-use; certificates carry `issued_ts` and
  the owner rejects stale ones (same monotonic posture as grant revocation).
- **Attribution**: companion ops are attributable to the exact device (own key) and to
  the member (cert chain); better forensics than v1's shared-user-key signing.
- **R6 residual** (honest-client policy layer) applies to cert checks exactly as it does
  to roles; the op log keeps everything attributable.

## Phases (each its own reviewed slice)

- **M1** `catcoms-crypto`: device certificates, pairing request/nonce, SAS derivation
  (+ golden vectors, single-use semantics tests). Pure primitives, no I/O.
- **M2** pairing transport + the grant ceremony: request/confirm flow between the two
  devices (reusing invite rendezvous machinery), the origin-side popup surface, bundle
  mint (all-server), passphrase wrap.
- **M3** admission: `CTRL_DEVICE_ADD` through the owner-serialized queue; cert/freshness/
  revocation checks; companion → origin mapping table gossiped like other metadata.
- **M4** UI: roster grouping (member row expands to devices), device tags on messages,
  owner device panel, device-name management.
- **M5** revocation verbs + lost-origin UX + export-time warnings.
- **M6** QR/camera + audio channels for the pairing request (shared with the invite-UX
  backlog; copy-paste blob ships first).

v1's M3 (doc re-keying) is deleted by design; badges/profiles/roles stay origin-keyed.
