//! Optional canonical trailer. Its absence means no retained sound evidence, not silence.
//! Existing art-only payloads stay byte-identical; explicit unlink/removal records use a trailer.

use super::*;
use crate::studio::snapshot::{count, malformed, Writer, MAX_VALUES};
use catcoms_wire::Decoder;

const SOUND_TRAILER: u8 = 1;

fn record(document: &LogicalDocument, v: &FrameValue<FlipnoteOp>) -> Result<Vec<u8>, ReplError> {
    let domain = DomainOp {
        nonce: v.source.nonce,
        doc_type: document.doc_type,
        logical_key: document.logical_key.clone(),
        body: v.value.encode()?,
    };
    if domain.id(&v.source.author) != v.source.op_id {
        return Err(ReplError::Malformed);
    }
    super::super::change::encode_record(&domain, &v.source.author, v.source.ts, None, None)
}

impl FrameSoundState {
    pub(super) fn measure(&self, document: &LogicalDocument) -> Result<(), ReplError> {
        self.write(
            document,
            &mut Writer::measure(crate::epoch::MAX_RECOVERY_SNAPSHOT_BYTES),
        )
    }
    pub(in crate::studio::frames) fn write(
        &self,
        document: &LogicalDocument,
        e: &mut Writer,
    ) -> Result<(), ReplError> {
        if self.records.is_empty() {
            return Ok(());
        }
        e.byte(SOUND_TRAILER)?;
        e.count(self.records.len())?;
        let mut ids = BTreeMap::new();
        for (key, r) in &self.records {
            if key.len() > MAX_KEY_BYTES {
                return Err(ReplError::EpochBound);
            }
            e.bytes(key.as_bytes())?;
            e.count(r.conflicts.len())?;
            let mut previous = None;
            for (n, v) in std::iter::once(&r.selected).chain(&r.conflicts).enumerate() {
                if n > 0 {
                    if v.source.op_id == r.selected.source.op_id
                        || previous.is_some_and(|p| p >= v.source.op_id)
                    {
                        return Err(ReplError::Malformed);
                    }
                    previous = Some(v.source.op_id);
                }
                if super::key(&v.value, &v.source.op_id)
                    .is_none_or(|(expected, _)| &expected != key)
                {
                    return Err(ReplError::Malformed);
                }
                let bytes = record(document, v)?;
                e.bytes(&bytes)?;
                if ids.insert(v.source.op_id, bytes).is_some() {
                    return Err(ReplError::Malformed);
                }
            }
        }
        Ok(())
    }
    pub(in crate::studio::frames) fn read(
        document: &LogicalDocument,
        d: &mut Decoder<'_>,
    ) -> Result<Self, ReplError> {
        if d.is_empty() {
            return Ok(Self::default());
        }
        if d.get_u8().map_err(malformed)? != SOUND_TRAILER {
            return Err(ReplError::Malformed);
        }
        let n = count(d, MAX_VALUES)?;
        // Empty trailers would give one art state two encodings.
        if n == 0 {
            return Err(ReplError::Malformed);
        }
        let mut state = Self::default();
        for _ in 0..n {
            let key = d.get_str().map_err(malformed)?;
            if key.len() > MAX_KEY_BYTES
                || state
                    .records
                    .last_key_value()
                    .is_some_and(|(old, _)| old.as_str() >= key)
            {
                return Err(ReplError::Malformed);
            }
            let conflicts = count(d, MAX_VALUES)?;
            let mut values = Vec::new();
            // Do not trust counts to reserve: bytes and record framing admit each value first.
            for _ in 0..=conflicts {
                let bytes = d.get_bytes().map_err(malformed)?;
                let r = decode_record(document, bytes)?;
                if r.anchor.is_some() || r.before.is_some() {
                    return Err(ReplError::Malformed);
                }
                values.push(FrameValue {
                    source: r.source,
                    value: r.operation,
                });
            }
            let selected = values.remove(0);
            state.insert(
                key.into(),
                FrameRegister {
                    selected,
                    conflicts: values,
                },
            )?;
        }
        state.measure(document)?;
        Ok(state)
    }
}
