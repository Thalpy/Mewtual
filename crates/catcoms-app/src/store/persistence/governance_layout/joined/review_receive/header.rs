//! Exact retained public headers and the single selected fork witness.
//! These decoders authenticate data without granting storage selection.
use super::*;
use catcoms_replication::chat::{
    owner_bodies::{ReviewStreamEntry, MAX_REVIEW_STREAM_ENTRY_BYTES},
    FounderGenesisAnchorData,
};
use catcoms_rt::CryptoRngCore;
use catcoms_wire::{Decoder, Encoder};

const HEADER: &[u8] = b"catcoms/private-chat-review-received-header/v1\0";
const FORK: &[u8] = b"catcoms/private-chat-review-received-fork/v1\0";
const SEAL_BYTES: usize = 40;
pub(super) const MAX_HEADER_BYTES: usize =
    SEAL_BYTES + 4 + HEADER.len() + 4 + MAX_REVIEW_STREAM_ENTRY_BYTES;
pub(super) const MAX_FORK_BYTES: usize =
    SEAL_BYTES + 4 + FORK.len() + 2 * (4 + MAX_REVIEW_STREAM_ENTRY_BYTES);

pub(super) struct ReceiverKey(Zeroizing<[u8; 32]>);

impl ReceiverKey {
    pub(super) fn new(vault_key: &[u8; 32], marker: &[u8; 32], original_pin: &[u8; 32]) -> Self {
        let mut private =
            blake3::Hasher::new_derive_key("catcoms/private-chat-review-receiver-key/v1");
        private.update(vault_key);
        private.update(marker);
        private.update(original_pin);
        Self(Zeroizing::new(*private.finalize().as_bytes()))
    }

    /// Store precisely the representation authenticated at ingress. The
    /// surrounding writer holds the locally selected membership proof.
    pub(super) fn cover_header_data(
        &self,
        pin: &FounderGenesisAnchorData,
        entry: &ReviewStreamEntry,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Zeroizing<Vec<u8>>, AppError> {
        verify_public(pin, entry)?;
        let mut plain = Encoder::with_capacity(MAX_HEADER_BYTES - SEAL_BYTES);
        plain.put_bytes(HEADER).map_err(|_| denied())?;
        plain.put_bytes(&entry.encode()).map_err(|_| denied())?;
        self.seal(plain, rng)
    }

    pub(super) fn open_header(
        &self,
        pin: &FounderGenesisAnchorData,
        raw: &[u8],
    ) -> Result<ReviewStreamEntry, AppError> {
        let plain = self.open(raw, MAX_HEADER_BYTES)?;
        let mut input = Decoder::new(&plain);
        if field(&mut input)? != HEADER {
            return Err(denied());
        }
        let entry = read_entry(pin, &mut input)?;
        input.finish().map_err(|_| denied())?;
        Ok(entry)
    }

    /// Retain both independently authenticated representations. The
    /// original writer proves the first was selected at this ordinal.
    pub(super) fn cover_fork_data(
        &self,
        pin: &FounderGenesisAnchorData,
        selected: &ReviewStreamEntry,
        incoming: &ReviewStreamEntry,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Zeroizing<Vec<u8>>, AppError> {
        verify_public(pin, selected)?;
        verify_public(pin, incoming)?;
        require_distinct_at_position(selected, incoming)?;
        let mut plain = Encoder::with_capacity(MAX_FORK_BYTES - SEAL_BYTES);
        for bytes in [FORK, &selected.encode(), &incoming.encode()] {
            plain.put_bytes(bytes).map_err(|_| denied())?;
        }
        self.seal(plain, rng)
    }

    pub(super) fn open_fork(
        &self,
        pin: &FounderGenesisAnchorData,
        raw: &[u8],
    ) -> Result<(ReviewStreamEntry, ReviewStreamEntry), AppError> {
        let plain = self.open(raw, MAX_FORK_BYTES)?;
        let mut input = Decoder::new(&plain);
        if field(&mut input)? != FORK {
            return Err(denied());
        }
        let selected = read_entry(pin, &mut input)?;
        let incoming = read_entry(pin, &mut input)?;
        input.finish().map_err(|_| denied())?;
        require_distinct_at_position(&selected, &incoming)?;
        Ok((selected, incoming))
    }

    pub(super) fn seal(
        &self,
        plain: Encoder,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Zeroizing<Vec<u8>>, AppError> {
        let plain = Zeroizing::new(plain.finish());
        Ok(Zeroizing::new(frame(&seal(&self.0, &plain, rng)?)))
    }

    pub(super) fn open(&self, raw: &[u8], max: usize) -> Result<Zeroizing<Vec<u8>>, AppError> {
        if !(SEAL_BYTES + 1..=max).contains(&raw.len()) {
            return Err(denied());
        }
        Ok(Zeroizing::new(unseal(&self.0, &unframe(raw)?)?))
    }
}

fn require_distinct_at_position(
    first: &ReviewStreamEntry,
    second: &ReviewStreamEntry,
) -> Result<(), AppError> {
    if first.position_data().sequence != second.position_data().sequence
        || first.id() == second.id()
    {
        return Err(denied());
    }
    Ok(())
}

fn verify_public(
    pin: &FounderGenesisAnchorData,
    entry: &ReviewStreamEntry,
) -> Result<(), AppError> {
    if entry.case_step_data().is_none() {
        return Err(denied());
    }
    entry.verify_for_pin(pin).map_err(|_| denied())
}

fn read_entry(
    pin: &FounderGenesisAnchorData,
    input: &mut Decoder<'_>,
) -> Result<ReviewStreamEntry, AppError> {
    let bytes = field(input)?;
    if bytes.len() != MAX_REVIEW_STREAM_ENTRY_BYTES {
        return Err(denied());
    }
    let entry = ReviewStreamEntry::decode(bytes).map_err(|_| denied())?;
    verify_public(pin, &entry)?;
    Ok(entry)
}

fn field<'a>(input: &mut Decoder<'a>) -> Result<&'a [u8], AppError> {
    input.get_bytes().map_err(|_| denied())
}
