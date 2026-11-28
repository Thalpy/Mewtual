//! Point-index rows for the private founder decision journal. A row
//! locates exact selected data. It cannot prove which root was selected.
use super::*;
use catcoms_replication::chat::owner_bodies::ReviewCaseId;

const DOMAIN: &[u8] = b"catcoms/private-chat-review-index-value/v1\0";
const INTENT_KEY: &str = "catcoms/private-chat-review-intent-key/v1";
const CASE_KEY: &str = "catcoms/private-chat-review-case-key/v1";
const VALUE_BYTES: usize = 4 + DOMAIN.len() + 20 + 4 * 36 + 8;
const _: () = assert!(VALUE_BYTES <= MAX_VALUE_BYTES);

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::store) struct ReviewIndexValueData {
    pub(in crate::store) intent: [u8; 16],
    pub(in crate::store) request: [u8; 32],
    pub(in crate::store) case: ReviewCaseId,
    pub(in crate::store) sequence: u64,
    pub(in crate::store) entry: [u8; 32],
    pub(in crate::store) record: [u8; 32],
}

catcoms_rt::redacted_debug! { ReviewIndexValueData; }

impl ReviewIndexValueData {
    pub(in crate::store) fn intent_key(cid: TextContextIdData, intent: [u8; 16]) -> [u8; 32] {
        key(INTENT_KEY, cid, &intent)
    }

    pub(in crate::store) fn case_key(cid: TextContextIdData, case: ReviewCaseId) -> [u8; 32] {
        key(CASE_KEY, cid, case.as_bytes())
    }

    pub(super) fn require_key(
        self,
        context: PrivateIndexContextData,
        expected: [u8; 32],
    ) -> Result<()> {
        let actual = match context.kind {
            PrivateIndexKindData::ReviewIntent => Self::intent_key(context.cid, self.intent),
            PrivateIndexKindData::ReviewCase => Self::case_key(context.cid, self.case),
            _ => return Err(PrivateIndexError::Context),
        };
        if expected != actual {
            return Err(PrivateIndexError::Context);
        }
        self.require_shape()
    }

    fn require_shape(&self) -> Result<()> {
        if self.sequence == 0
            || self.intent == [0; 16]
            || self.request == [0; 32]
            || self.case.as_bytes() == &[0; 32]
            || self.entry == [0; 32]
            || self.record == [0; 32]
        {
            return Err(PrivateIndexError::Malformed);
        }
        Ok(())
    }

    pub(super) fn encode(self) -> Result<Zeroizing<Vec<u8>>> {
        self.require_shape()?;
        let mut encoded = Encoder::with_capacity(VALUE_BYTES);
        for field in [
            DOMAIN,
            &self.intent,
            &self.request,
            self.case.as_bytes(),
            &self.entry,
            &self.record,
        ] {
            encoded
                .put_bytes(field)
                .map_err(|_| PrivateIndexError::Bound)?;
        }
        encoded.put_u64(self.sequence);
        Ok(Zeroizing::new(encoded.finish()))
    }

    pub(super) fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != VALUE_BYTES {
            return Err(PrivateIndexError::Bound);
        }
        let mut reader = Decoder::new(bytes);
        if reader
            .get_bytes()
            .map_err(|_| PrivateIndexError::Malformed)?
            != DOMAIN
        {
            return Err(PrivateIndexError::Context);
        }
        let value = Self {
            intent: reader
                .get_bytes()
                .map_err(|_| PrivateIndexError::Malformed)?
                .try_into()
                .map_err(|_| PrivateIndexError::Malformed)?,
            request: b32(&mut reader)?,
            case: ReviewCaseId::from_bytes(b32(&mut reader)?),
            entry: b32(&mut reader)?,
            record: b32(&mut reader)?,
            sequence: reader.get_u64().map_err(|_| PrivateIndexError::Malformed)?,
        };
        reader.finish().map_err(|_| PrivateIndexError::Malformed)?;
        value.require_shape()?;
        Ok(value)
    }
}

fn key(domain: &str, cid: TextContextIdData, lookup: &[u8]) -> [u8; 32] {
    let mut hash = blake3::Hasher::new_derive_key(domain);
    hash.update(cid.as_bytes());
    hash.update(lookup);
    *hash.finalize().as_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_index_rows_bind_the_original_intent_case_and_namespace() {
        let cid = TextContextIdData::from_bytes([11; 32]);
        let row = ReviewIndexValueData {
            intent: [12; 16],
            request: [13; 32],
            case: ReviewCaseId::from_bytes([14; 32]),
            sequence: 15,
            entry: [16; 32],
            record: [17; 32],
        };
        let intent = PrivateIndexContextData {
            cid,
            kind: PrivateIndexKindData::ReviewIntent,
        };
        let case = PrivateIndexContextData {
            cid,
            kind: PrivateIndexKindData::ReviewCase,
        };
        let intent_key = ReviewIndexValueData::intent_key(cid, row.intent);
        let case_key = ReviewIndexValueData::case_key(cid, row.case);
        assert_ne!(intent_key, case_key);
        assert_eq!(
            ReviewIndexValueData::decode(&row.encode().unwrap()),
            Ok(row)
        );
        assert!(row.require_key(intent, intent_key).is_ok());
        assert!(row.require_key(case, case_key).is_ok());
        assert!(row.require_key(intent, case_key).is_err());
        assert!(row.require_key(case, intent_key).is_err());
        assert!(row
            .require_key(
                PrivateIndexContextData {
                    cid: TextContextIdData::from_bytes([18; 32]),
                    ..intent
                },
                intent_key,
            )
            .is_err());
        let mut padded = row.encode().unwrap();
        padded.push(0);
        assert!(ReviewIndexValueData::decode(&padded).is_err());
        assert!(ReviewIndexValueData { sequence: 0, ..row }
            .encode()
            .is_err());
    }
}
