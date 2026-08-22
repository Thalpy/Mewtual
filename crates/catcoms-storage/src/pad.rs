//! Size quantization: pad a plaintext to a bucket **before** it is sealed, so the ciphertext
//! length a forwarder measures is one of a couple of dozen values rather than a near-exact
//! readout of the payload.
//!
//! This is P10, and the half of `ARCHITECTURE.md` §2.8 that says "blob-fetch
//! padding/quantization". Two call sites, two ladders, one frame format:
//!
//! - [`SealedOp`](../../catcoms_replication/struct.SealedOp.html) seals every gossiped CRDT op.
//!   Gossipsub is **signed**, so a switchboard member (rung 2 of
//!   `design-zeroconf-reachability.md`) that forwards a message sees publisher, topic, sequence,
//!   timestamp and exact size. Measured against the shipping schema, a channel op is
//!   `~250 + text_len` bytes of plaintext, i.e. the wire size *is* the message length plus a
//!   constant. [`OP_PAD_FLOOR`] / [`OP_PAD_CEILING`] collapse that.
//! - [`seal_file`](crate::seal_file) seals every shared-file chunk. Files are already split at
//!   8 MiB, so a large file is uniform except for its tail; the leak is a file **smaller than one
//!   chunk** (a custom emoji, a chat image embed, a wiki illustration) whose single chunk size is
//!   the file size, plus that tail. [`CHUNK_PAD_FLOOR`] / [`CHUNK_PAD_CEILING`] collapse those and
//!   deliberately leave a *full* chunk untouched, because a full chunk is already uniform and
//!   padding it would cost 100% for nothing.
//!
//! ## Why this module lives in `catcoms-storage`
//!
//! It has nothing to do with storage; it is a byte-frame codec and its natural home is
//! `catcoms-wire`. It is here because `catcoms-storage` is the lowest crate both call sites can
//! share, and one implementation of [`unpad`] is worth more than a tidy module tree: a
//! second, subtly different unpadder is exactly the shape of bug this is meant to prevent.
//!
//! ## The frame
//!
//! ```text
//! [ body (n bytes) ][ zero fill (b - n bytes) ][ u32 big-endian n ]
//! ```
//!
//! where `b = padded_len(n)`. Three properties, all load-bearing:
//!
//! - **It is inside the AEAD.** Both call sites pad and *then* seal, so a forwarder cannot
//!   strip the fill: it never sees it, and removing bytes from the ciphertext fails the
//!   Poly1305 tag.
//! - **It is canonical.** [`unpad`] recomputes `padded_len(n)` and refuses a frame whose length
//!   is not exactly it, and refuses any non-zero fill byte. So there is exactly one valid
//!   encoding of a given body, the fill is not a covert channel, and a hostile pad is an error
//!   rather than a silent truncation.
//! - **It is deterministic.** No randomness, so it consumes no entropy, needs nothing from the
//!   injected `CryptoRngCore` seam, and cannot perturb the byte-identical-compaction property
//!   (which in any case operates over the *unpadded* `SignedOp` log; see the module docs on
//!   `catcoms_replication::op`).
//!
//! ## What it costs
//!
//! Padding is bandwidth, and on a relay every byte is charged twice (ingress and egress; see
//! `catcoms_net::relay_node`). The per-ladder doc comments below carry the measured numbers.
//!
//! **It does not move the relay's capacity arithmetic.** `RelayLimits::nominal_window_bytes`
//! sizes `max_circuits` against `node_budget_bytes` using `NOMINAL_CIRCUIT_BYTES_PER_SEC` = 16
//! KB/s, and that 16 KB/s is a **voice** figure (32 kbit/s each way, charged twice). Voice media
//! is WebRTC DTLS-SRTP in the desktop frontend: it is neither a sealed op nor a blob, so nothing
//! here touches it, and the yardstick the self-check uses is unchanged. It also stays the
//! pessimistic one: a member gossiping a padded op every second draws 590 x 2 = 1.2 KB/s, still
//! an order of magnitude under the nominal, and bulk blob traffic was already bounded by
//! `max_circuit_bytes` and the per-peer / per-prefix byte budgets rather than by the nominal
//! rate. So `RelayLimits::validate` needed no change, and deliberately did not get one.
//!
//! What padding does move is how far a **budget** goes. At the defaults, 1,000 members each
//! posting 100 ops an hour through one relay costs it 1,000 x 100 x 2 x ~200 B = 40 MB/hour of
//! extra ingress+egress, 0.45% of the 8 GiB hourly node budget. File traffic is the larger
//! share: a bucket step is up to (but rarely) 100% of a sub-chunk file, so a member's 1 GiB
//! hourly budget buys correspondingly fewer small-file fetches, while full 8 MiB chunks are
//! unaffected.

use crate::StorageError;

/// Bytes the length footer costs, on **every** padded payload including one too large to pad.
///
/// A `u32` covers 4 GiB, comfortably above the 8 MiB chunk and the 16 MiB frame cap, and four
/// bytes is the whole unconditional cost of this scheme: a payload above its ladder's ceiling
/// grows by exactly this much and nothing else.
pub const PAD_FOOTER_BYTES: usize = 4;

/// Floor of the **CRDT op** ladder: no sealed op is smaller than this once padded.
///
/// The fixed cost of a `SignedOp` (doc type, doc id, 32-byte author key, 64-byte signature, and
/// the automerge change header with its dependency hashes) is about 250 bytes before a single
/// character of message text, so 512 is one octave of headroom above the floor of the
/// distribution. Measured against the shipping chat schema it covers **every message up to about
/// 190 characters**, plus every reaction toggle, pin, edit marker, topic set and profile tweak:
/// the great majority of ops become one indistinguishable size class.
///
/// What it costs: a short chat message goes from 395 to 590 bytes on the wire (+49%), a reaction
/// from 326 to 590 (+81%). In absolute terms that is under 270 bytes per op. A relay charges it
/// twice, so 1,000 members each posting 100 ops an hour costs the node budget 1,000 x 100 x 2 x
/// ~200 B = 40 MB/hour, which is 0.45% of the default 8 GiB hourly budget.
///
/// Raising it to 1024 would collapse messages up to ~700 characters into one class for about
/// 1 KB/message, still affordable on the gossip path; it was not taken because it also lands on
/// every op in a **catch-up bundle**, where it turns a 5,000-message backlog from 2.1 MB into
/// 5.5 MB rather than the 2.95 MB this floor costs.
pub const OP_PAD_FLOOR: usize = 512;

/// Ceiling of the **CRDT op** ladder: an op larger than this is not padded at all.
///
/// Above 1 MiB the doubling step is worth megabytes and buys nothing: an op that large is a bulk
/// document edit, not a message, and its size is already coarse. The ceiling is also what makes
/// the scheme safe against the size caps around it, because it bounds growth: a padded op is at
/// most `max(n, OP_PAD_CEILING) + PAD_FOOTER_BYTES`, so padding can never push a payload from
/// under `MAX_FRAME` (16 MiB, `catcoms_net`) or `MAX_CATCHUP_RESPONSE` (16 MiB, `catcoms_sync`)
/// to over it by more than the four-byte footer.
pub const OP_PAD_CEILING: usize = 1024 * 1024;

/// Floor of the **file chunk** ladder.
///
/// The smallest things that travel as chunks are custom emoji and small image embeds, a few KB
/// each. 4 KiB is one filesystem page and collapses everything below it, so an emoji, a tiny
/// icon and an empty file are indistinguishable.
///
/// What it costs: at most 4 KiB per sub-4-KiB file, once, at upload time.
pub const CHUNK_PAD_FLOOR: usize = 4 * 1024;

/// Ceiling of the **file chunk** ladder, and deliberately **equal to the app's plaintext chunk
/// size** (`catcoms_app::CHUNK_BYTES` = 8 MiB), which is asserted by a test there.
///
/// That equality is the whole design of the large-file case. A file bigger than one chunk is
/// already a run of identical 8 MiB chunks plus a tail, so the only thing a forwarder learns from
/// the run is the file size to within 8 MiB; the exact size is in the **tail**. With the ceiling
/// at exactly one chunk, a full chunk pads to itself (cost: the 4-byte footer) and every tail
/// pads up to a full chunk or to a smaller power of two, so the tail stops being an exact-size
/// fingerprint.
///
/// What it costs: worst case one bucket step on **one** chunk per file, i.e. under 8 MiB on a
/// file of any size, and 0 on every full chunk. A 7 MiB tail costs 1 MiB (+14% of the tail); a
/// 4.1 MiB tail costs 3.9 MiB, which is the honest worst case of a power-of-two ladder and is
/// still under 100% of one chunk on a file that is by then at least 12 MiB.
///
/// Raising it would pad a full chunk to 16 MiB: 100% overhead on the product's bulk traffic to
/// hide a size that is already constant. Rejected.
pub const CHUNK_PAD_CEILING: usize = 8 * 1024 * 1024;

/// The padded length of a `n`-byte body under the ladder `[floor, ceiling]`: the smallest power
/// of two at least `max(n, floor)`, or `n` unchanged when `n` exceeds `ceiling`.
///
/// `floor` and `ceiling` must be powers of two with `floor <= ceiling`; every ladder in this
/// module is, and the invariant is what guarantees the result never exceeds `ceiling` (so the
/// growth this scheme can cause is bounded, and no size cap above it can be newly breached).
#[must_use]
pub fn padded_len(n: usize, floor: usize, ceiling: usize) -> usize {
    debug_assert!(
        floor.is_power_of_two(),
        "ladder floor must be a power of two"
    );
    debug_assert!(
        ceiling.is_power_of_two(),
        "ladder ceiling must be a power of two"
    );
    debug_assert!(floor <= ceiling, "ladder floor must not exceed its ceiling");
    if n > ceiling {
        // Above the ceiling the payload is left exactly as it is. Doubling here would be
        // megabytes for no privacy (see `CHUNK_PAD_CEILING`), and leaving it alone is what keeps
        // the growth bound `padded_len(n) <= max(n, ceiling)` true.
        return n;
    }
    // `max(n, floor) <= ceiling` and `ceiling` is a power of two, so this cannot exceed the
    // ceiling and cannot overflow.
    n.max(floor).next_power_of_two()
}

/// Wrap `body` in a padded frame under the ladder `[floor, ceiling]`. The result is
/// `padded_len(body.len(), floor, ceiling) + PAD_FOOTER_BYTES` bytes long, always.
///
/// The caller must seal the result; padding that is not inside an AEAD is padding a forwarder
/// can strip.
pub fn pad(body: &[u8], floor: usize, ceiling: usize) -> Result<Vec<u8>, StorageError> {
    // The footer is a u32, so a body at or above 4 GiB has no representable length. Nothing in
    // the product comes near it (the largest padded payload is one 8 MiB chunk), but an
    // unrepresentable length must be an error, never a truncated one.
    let n = u32::try_from(body.len()).map_err(|_| StorageError::Malformed)?;
    let target = padded_len(body.len(), floor, ceiling);
    let mut out = Vec::with_capacity(target + PAD_FOOTER_BYTES);
    out.extend_from_slice(body);
    out.resize(target, 0);
    out.extend_from_slice(&n.to_be_bytes());
    Ok(out)
}

/// Recover the body from a frame produced by [`pad`] under the same ladder.
///
/// Fails closed on anything that is not exactly what [`pad`] would have produced: a frame too
/// short to hold a footer, a declared length that does not fit, a frame length that is not the
/// bucket the declared length maps to, or a single non-zero fill byte. It never panics and never
/// returns a truncated body, so a hostile peer that has (somehow) produced a valid AEAD tag over
/// a malformed frame gets an error rather than a partial payload or a covert channel.
pub fn unpad(frame: &[u8], floor: usize, ceiling: usize) -> Result<&[u8], StorageError> {
    if frame.len() < PAD_FOOTER_BYTES {
        return Err(StorageError::Malformed);
    }
    let split = frame.len() - PAD_FOOTER_BYTES;
    let mut footer = [0u8; PAD_FOOTER_BYTES];
    footer.copy_from_slice(&frame[split..]);
    let n = u32::from_be_bytes(footer) as usize;
    if n > split {
        return Err(StorageError::Malformed);
    }
    // Canonical length: the frame must be exactly the bucket this body maps to. Without this a
    // sender could pick any larger bucket and signal in the choice.
    if split != padded_len(n, floor, ceiling) {
        return Err(StorageError::Malformed);
    }
    // Canonical fill: zero, so the pad carries no bits.
    if frame[n..split].iter().any(|b| *b != 0) {
        return Err(StorageError::Malformed);
    }
    Ok(&frame[..n])
}

#[cfg(test)]
mod tests {
    use super::*;

    const F: usize = OP_PAD_FLOOR;
    const C: usize = OP_PAD_CEILING;

    #[test]
    fn a_padded_body_round_trips_to_exactly_the_original_bytes() {
        for n in [0usize, 1, 2, 31, 255, 511, 512, 513, 1000, 4096, 65_537] {
            let body: Vec<u8> = (0..n).map(|i| (i % 251) as u8).collect();
            let frame = pad(&body, F, C).unwrap();
            assert_eq!(
                unpad(&frame, F, C).unwrap(),
                &body[..],
                "body of {n} bytes must survive the round trip byte for byte"
            );
        }
    }

    #[test]
    fn the_bucket_boundaries_are_exact_including_both_edges() {
        // Everything at or below the floor lands ON the floor, and the floor itself does not
        // step up (an off-by-one here doubles the cost of every small op).
        assert_eq!(padded_len(0, F, C), 512);
        assert_eq!(padded_len(1, F, C), 512);
        assert_eq!(padded_len(511, F, C), 512);
        assert_eq!(padded_len(512, F, C), 512);
        // ...and one byte past it is the next bucket, not the same one.
        assert_eq!(padded_len(513, F, C), 1024);
        assert_eq!(padded_len(1023, F, C), 1024);
        assert_eq!(padded_len(1024, F, C), 1024);
        assert_eq!(padded_len(1025, F, C), 2048);
        // The top bucket is the ceiling exactly, and it is a fixed point.
        assert_eq!(padded_len(C - 1, F, C), C);
        assert_eq!(padded_len(C, F, C), C);
        // One byte over the ceiling is not padded at all (never `2 * C`).
        assert_eq!(padded_len(C + 1, F, C), C + 1);
        assert_eq!(padded_len(C * 4, F, C), C * 4);
    }

    #[test]
    fn the_frame_length_is_the_bucket_not_merely_bigger() {
        // The property padding actually has to have: the observed length is one of the ladder's
        // values, so it reveals the bucket and nothing finer.
        for n in [0usize, 1, 100, 511, 512, 513, 900, 1024, 1025] {
            let frame = pad(&vec![7u8; n], F, C).unwrap();
            let observed = frame.len() - PAD_FOOTER_BYTES;
            assert!(
                observed.is_power_of_two() && observed >= F,
                "a {n}-byte body framed to {observed}, which is not a ladder value"
            );
            assert_eq!(observed, padded_len(n, F, C));
        }
        // Two very different bodies inside one bucket are indistinguishable on the wire.
        assert_eq!(
            pad(&[0u8; 1], F, C).unwrap().len(),
            pad(&[0u8; 500], F, C).unwrap().len()
        );
    }

    #[test]
    fn an_empty_body_and_a_one_byte_body_both_frame_to_the_floor() {
        assert_eq!(pad(&[], F, C).unwrap().len(), F + PAD_FOOTER_BYTES);
        assert_eq!(pad(&[9], F, C).unwrap().len(), F + PAD_FOOTER_BYTES);
        assert_eq!(unpad(&pad(&[], F, C).unwrap(), F, C).unwrap(), b"");
    }

    #[test]
    fn a_body_over_the_ceiling_grows_by_exactly_the_footer() {
        let body = vec![3u8; C + 7];
        let frame = pad(&body, F, C).unwrap();
        assert_eq!(frame.len(), body.len() + PAD_FOOTER_BYTES);
        assert_eq!(unpad(&frame, F, C).unwrap(), &body[..]);
    }

    #[test]
    fn a_hostile_or_malformed_frame_fails_closed() {
        // Too short to hold a footer.
        assert!(unpad(&[], F, C).is_err());
        assert!(unpad(&[0, 0, 0], F, C).is_err());
        // A declared length longer than the frame: must not truncate, must not panic.
        let mut over = vec![0u8; F + PAD_FOOTER_BYTES];
        over[F..].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(unpad(&over, F, C).is_err());
        // A declared length that fits but is not this frame's bucket (an over-padded frame,
        // which is where a covert channel would live).
        let mut wrong_bucket = vec![0u8; 2048 + PAD_FOOTER_BYTES];
        wrong_bucket[2048..].copy_from_slice(&10u32.to_be_bytes());
        assert!(unpad(&wrong_bucket, F, C).is_err());
        // A frame length that is not a ladder value at all.
        let mut ragged = vec![0u8; 700 + PAD_FOOTER_BYTES];
        ragged[700..].copy_from_slice(&10u32.to_be_bytes());
        assert!(unpad(&ragged, F, C).is_err());
        // Non-zero fill: a correct length, a correct bucket, but bits hidden in the pad.
        let mut noisy = pad(b"hello", F, C).unwrap();
        noisy[100] = 0xFF;
        assert!(unpad(&noisy, F, C).is_err());
        // The very last fill byte, adjacent to the footer (the off-by-one in the zero scan).
        let mut edge = pad(b"hello", F, C).unwrap();
        edge[F - 1] = 1;
        assert!(unpad(&edge, F, C).is_err());
        // The first fill byte, adjacent to the body (the other off-by-one).
        let mut edge2 = pad(b"hello", F, C).unwrap();
        edge2[5] = 1;
        assert!(unpad(&edge2, F, C).is_err());
    }

    #[test]
    fn a_truncated_or_extended_frame_is_refused() {
        let frame = pad(b"body bytes", F, C).unwrap();
        let mut short = frame.clone();
        short.pop();
        assert!(unpad(&short, F, C).is_err());
        let mut long = frame.clone();
        long.push(0);
        assert!(unpad(&long, F, C).is_err());
    }

    #[test]
    fn padding_is_deterministic() {
        // No RNG, so the same body always frames to the same bytes: nothing here can perturb a
        // byte-identical compaction, and the frame consumes no entropy.
        let a = pad(b"the same body", F, C).unwrap();
        let b = pad(b"the same body", F, C).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn the_chunk_ladder_leaves_a_full_chunk_alone_and_flattens_the_tail() {
        let (f, c) = (CHUNK_PAD_FLOOR, CHUNK_PAD_CEILING);
        // A full chunk is already uniform: it must not be padded at all.
        assert_eq!(padded_len(CHUNK_PAD_CEILING, f, c), CHUNK_PAD_CEILING);
        // ...and a tail lands on the same value, so the tail stops being an exact-size tell.
        assert_eq!(padded_len(7 * 1024 * 1024, f, c), CHUNK_PAD_CEILING);
        // Small sub-chunk files collapse into a handful of classes.
        assert_eq!(padded_len(0, f, c), 4 * 1024);
        assert_eq!(padded_len(6_000, f, c), 8 * 1024);
        assert_eq!(padded_len(200_000, f, c), 256 * 1024);
        // The growth bound the size caps depend on.
        for n in [0usize, 1, 6_000, 200_000, 7 * 1024 * 1024, c, c + 1, c * 3] {
            assert!(padded_len(n, f, c) <= n.max(c));
        }
    }
}
