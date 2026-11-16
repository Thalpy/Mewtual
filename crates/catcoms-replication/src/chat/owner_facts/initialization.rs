//! Borrowed syntax/resource preflight for the fixed first-owner checkpoint fact subset.
//! No signature verification, pin, historical witness or owned decoded fact is produced here.
use super::*;

impl SignedOwnerFact {
    /// Check the complete original framing and counted fields without heap allocation.
    ///
    /// Only Profile, Admission, RealmGrant, Basis and JoinBootstrap occur in the actual first
    /// founder/fresh-KP member constructors. Acceptance/Removal return `Bound`: a caller must
    /// preserve them for a broader recovery path, never discard them as an empty checkpoint.
    /// This does NOT verify the signature or authorize any action. Ordinary `decode`/`verify`
    /// and the independently pinned authority verifier remain required after resource admission.
    pub fn preflight_initialization(bytes: &[u8]) -> Result<(), ChatError> {
        if bytes.len() > MAX_SMALL_FACT_BYTES {
            return Err(ChatError::Bound);
        }
        let mut outer = Decoder::new(bytes);
        if outer.get_bytes()? != ENVELOPE_DOMAIN {
            return Err(ChatError::Malformed);
        }
        let semantic = outer.get_bytes()?;
        let _signature = get_signature(&mut outer)?; // fixed stack copy, no cryptographic claim
        outer.finish()?;
        let mut decoder = Decoder::new(semantic);
        if decoder.get_bytes()? != FACT_DOMAIN {
            return Err(ChatError::Malformed);
        }
        let kind = decoder.get_u8()?;
        match kind {
            1..=4 | 7 => {}
            5 | 6 => return Err(ChatError::Bound),
            _ => return Err(ChatError::Malformed),
        }
        let group = decoder.get_bytes()?;
        if group.is_empty() || group.len() > MAX_GROUP_ID_BYTES {
            return Err(ChatError::Bound);
        }
        if decoder.get_u16()? != PROFILE {
            return Err(ChatError::Scope);
        }
        if kind != 1 {
            get_digest(&mut decoder)?;
        }
        get_digest(&mut decoder)?; // issuer, not a trusted founder or authority witness
        match kind {
            1 => {
                if decoder.get_str()? != TRUST_RULE {
                    return Err(ChatError::Authority);
                }
                get_digest(&mut decoder)?;
                get_digest(&mut decoder)?;
                if decoder.get_u8()? != 0 || decoder.get_str()? != RESOURCE_PROFILE {
                    return Err(ChatError::Authority);
                }
            }
            2 => {
                get_digest(&mut decoder)?;
                match decoder.get_u8()? {
                    0 => {
                        get_digest(&mut decoder)?;
                    }
                    1 => {
                        get_digest(&mut decoder)?;
                        decoder.get_u64()?;
                        decoder.get_u64()?;
                        get_digest(&mut decoder)?;
                        get_digest(&mut decoder)?;
                    }
                    _ => return Err(ChatError::Malformed),
                }
                get_digest(&mut decoder)?;
            }
            3 => {
                decoder.get_u128()?;
                get_digest(&mut decoder)?;
                get_digest(&mut decoder)?;
                Rights::from_bits(decoder.get_u16()?)?;
            }
            4 => {
                decoder.get_u128()?;
                for _ in 0..3 {
                    get_digest(&mut decoder)?;
                }
                let count = usize::from(decoder.get_u16()?);
                if count > MAX_BASIS_COVERAGE {
                    return Err(ChatError::Bound);
                }
                if count * 36 > decoder.remaining() {
                    return Err(ChatError::Malformed);
                }
                let mut previous = None;
                for _ in 0..count {
                    let digest = get_digest(&mut decoder)?;
                    if previous.is_some_and(|old| old >= digest) {
                        return Err(ChatError::Malformed);
                    }
                    previous = Some(digest);
                }
            }
            7 => {
                for _ in 0..4 {
                    get_digest(&mut decoder)?;
                }
            }
            _ => unreachable!("closed kind selected above"),
        }
        decoder.finish()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
