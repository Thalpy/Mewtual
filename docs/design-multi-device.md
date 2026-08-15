# Multi-device identity — design

Status: **draft for review.** One member ("user") on several devices, with per-device
message attribution, an exportable user key with a device cap, and owner-visible device
lists. This is a protocol-layer project on the scale of the 9-series persistence work:
phased, each slice adversarially reviewable. **No code should land from this doc until it
has been reviewed** — it touches admission, attribution, and revocation.

## Where we are (and the trap to avoid)

Today identity **is** the device: `DeviceId` is a content address of the device's
signature key (`catcoms-crypto/src/ids.rs`), `fingerprint(device_id)` is the member id
everywhere (profiles, roles, badges, presence, message authorship), and the MLS group
admits/removes *devices* (`contains_device`, `remove(&device_id)`). One member = one leaf.

**The trap:** "just export the device keypair and import it on the laptop." Two devices
sharing one MLS leaf fork the ratchet — forward secrecy breaks, epochs desync, commits
race — and the devices are cryptographically indistinguishable, so "sent from Phone" is
impossible anyway. This path is explicitly rejected; the exporter below never exports a
device key.

## Model: a user key certifies device keys (Signal/Matrix shape)

- **Device key**: exactly today's per-device signature key; one MLS leaf per device;
  every op/message stays device-signed (attribution comes free).
- **User key**: a new Ed25519 keypair. Its only jobs: sign **device certificates** and a
  small self-signed **user record**.
- **Device certificate**: `sig_user(user_pk ‖ device_id ‖ device_name ‖ ordinal ‖ cap)` —
  "device #2 of at most 3, named 'Phone', is me". `device_name` is bounded UTF-8 (≤ 24
  bytes), shown beside messages and in rosters.
- **User record**: `sig_user(user_pk ‖ rev ‖ cap ‖ revoked_device_ids[])` — a
  monotonically-versioned statement of the device cap and revocation list.

**Per-server user keys.** CatComs deliberately keeps identities unlinkable across servers
(per-server profiles; DM identities unlinkable to server identities). A single global user
key would become a cross-server correlator and break that. So user keys are **minted per
server** (and per DM), and the export bundle carries the servers you choose to provision.
Tradeoff: "one QR to move everything" becomes "one bundle with N entries" — acceptable;
the UI hides the plurality.

## The flows

**Provisioning (the export the user asked for).** On device A: choose servers to bundle →
choose the device cap N ("how many devices you plan to use", stored in the user record) →
export a **provisioning bundle**: per server, the user private key + current user record.
The bundle is passphrase-wrapped (Argon2id) — it IS the identity; treat like the vault.
On device B: import bundle → B mints its own device key → self-issues a device
certificate (signed by the user key it now holds) → for each server, runs the normal join
flow **except** admission is satisfied by "device certified by an already-admitted user
with spare cap" instead of an invite-ledger entry.

**Admission.** Reuses the owner-serialized add queue from
[`design-admin-invites.md`](design-admin-invites.md): the new device broadcasts a signed
`CTRL_DEVICE_ADD` (device cert + user record); the **owner alone** verifies
(user admitted? cert valid? `ordinal ≤ cap`? device not in `revoked[]`? rev fresh?) and
runs the MLS Add — single committer, no fork, offline-queued like admin invites. The
owner's verdict is protocol-enforced the same way R1 remove-gating is.

**Attribution.** Messages keep their device signature. The UI resolves
`device → (user, device_name)` via the certificate table and renders the user's
name/style + a small mono device tag ("· phone") when a user has >1 device. Badges,
profiles, roles re-key from device-fp to **user id** (= fingerprint of `user_pk`).

**Roster & owner visibility.** MLS members can already see every leaf (ratchet tree), so
device *existence* is member-visible by construction; the UI groups leaves under their
user with an expandable device list. The **owner** additionally sees per-device last-seen
and the cap ("2 of 3 devices used") in the role manager.

**Revocation, two distinct verbs.**
- *User revokes own device* (lost phone): bump `rev`, add the device to `revoked[]`,
  publish; owner (any admin?) enforces with an MLS Remove of that leaf. The revoked device
  held the user key? No — devices hold only their device key; the user key lives where the
  user keeps it (export bundle / originating device). A stolen *bundle* is full compromise:
  document loudly, offer re-key-user later.
- *Server kicks a user*: remove **all** the user's leaves + ledger-ban the user id (the
  replay-proof machinery from [`design-grant-revocation.md`](design-grant-revocation.md)
  extends from device ids to user ids).

## Migration (existing members)

On upgrade, each existing device locally mints a user key and self-certifies itself as
device #1 (cap = 1 until the user exports with a higher cap). Docs keyed by device-fp are
**dual-read**: a key that matches a known device maps to that device's user; new writes
use user ids. Profiles/roles/badges (badges land device-keyed this week — the re-key is a
mechanical map-key change, noted in its design). No flag day, no data loss.

## Threat notes

- **Provisioning bundle theft** = identity theft for the bundled servers. Passphrase-wrap
  (Argon2id), display a one-time fingerprint check phrase on both devices, and encourage
  deletion after import. Later: expiring bundles.
- **Cap games**: the cap binds inside the signed user record; the owner enforces
  `ordinal ≤ cap` at admission, so a compromised bundle can't silently farm 50 leaves
  beyond the user's declared cap.
- **Rev rollback**: user records are monotonic (`rev`); the owner rejects stale records —
  same replay posture as grant revocation.
- **Verification interplay**: the Verify dialog gains a per-device dimension — verifying a
  *user* means verifying their user-key fingerprint; a new device under a verified user
  shows "verified user · new device" (weaker claim, honest wording), never a silent ✓.

## Phases (each its own reviewed slice)

- **M1** `catcoms-crypto`: user keys, device certs, user records (+ vectors).
- **M2** admission: `CTRL_DEVICE_ADD` through the owner-serialized queue; cap/rev checks.
- **M3** dual-read re-key of profiles / roles / badges to user ids.
- **M4** UI: roster grouping + device tags on messages + owner device panel.
- **M5** revocation verbs (device revoke; user kick extends the ban ledger).
- **M6** export/import UX (passphrase-wrapped bundle, cap picker; QR/audio later — the
  invite-UX work already wants those channels).

Absorbs the existing "sticky/transferable ownership" backlog item naturally (ownership
becomes a user-id property in M3). Until M1 lands, the cheap standalone win remains
available: a device-name field shown on messages, one device per user as today.
