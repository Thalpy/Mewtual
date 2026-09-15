//! Bounded-allocation preflight of Automerge 0.10 raw change columns before its parser runs.
//!
//! A raw chunk can still encode millions of operations in a tiny RLE run. Byte caps alone do
//! not bound decoding work. This scanner counts runs without expanding them and checks string
//! slices before Automerge's string decoder allocates. New column specs require explicit review.

use crate::ReplError;

const MAX_CELLS: u64 = 1_048_576;
const MAX_PREDECESSORS: u64 = 262_144;
const MAX_EXPANDED_STRINGS: u64 = 16 * 1024 * 1024;
pub(crate) const MAX_ACTIONS: u64 = 65_536;
const SPECS: &[u64] = &[
    0x01, 0x02, 0x11, 0x13, 0x15, 0x34, 0x42, 0x56, 0x57, 0x70, 0x71, 0x73, 0x94, 0xa5,
];

struct Input<'a> {
    bytes: &'a [u8],
    at: usize,
}
impl<'a> Input<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, at: 0 }
    }
    fn take(&mut self, len: u64) -> Result<&'a [u8], ReplError> {
        let len = usize::try_from(len).map_err(|_| ReplError::EpochBound)?;
        let end = self.at.checked_add(len).ok_or(ReplError::EpochBound)?;
        let value = self.bytes.get(self.at..end).ok_or(ReplError::Malformed)?;
        self.at = end;
        Ok(value)
    }
    fn unsigned(&mut self) -> Result<u64, ReplError> {
        let mut value = 0u64;
        for i in 0..10 {
            let byte = self.take(1)?[0];
            if i == 9 && byte > 1 {
                return Err(ReplError::Malformed);
            }
            value |= u64::from(byte & 127) << (i * 7);
            if byte & 128 == 0 {
                return Ok(value);
            }
        }
        Err(ReplError::Malformed)
    }
    fn signed(&mut self) -> Result<i64, ReplError> {
        let mut value = 0u64;
        for i in 0..10 {
            let byte = self.take(1)?[0];
            if i == 9 && byte != 0 && byte != 127 {
                return Err(ReplError::Malformed);
            }
            value |= u64::from(byte & 127) << (i * 7);
            if byte & 128 == 0 {
                let bits = (i + 1) * 7;
                if bits < 64 && byte & 64 != 0 {
                    value |= u64::MAX << bits;
                }
                return Ok(value as i64);
            }
        }
        Err(ReplError::Malformed)
    }
    fn sized(&mut self) -> Result<&'a [u8], ReplError> {
        let len = self.unsigned()?;
        self.take(len)
    }
    fn done(&self) -> bool {
        self.at == self.bytes.len()
    }
}

fn charge(total: &mut u64, value: u64, limit: u64) -> Result<(), ReplError> {
    *total = total.checked_add(value).ok_or(ReplError::EpochBound)?;
    if *total > limit {
        return Err(ReplError::EpochBound);
    }
    Ok(())
}

/// Scan a bounded kind-1 chunk. `max_actions` is seven for registry operations and the generic
/// P1 ceiling for other domain operations/seeds; typed schemas still impose their narrower rules.
pub(crate) fn check(bytes: &[u8], max_actions: u64) -> Result<(), ReplError> {
    if bytes.len() < 10 || bytes[..4] != [0x85, 0x6f, 0x4a, 0x83] || bytes[8] != 1 {
        return Err(ReplError::Malformed);
    }
    let mut frame = Input::new(&bytes[9..]);
    let payload = frame.sized()?;
    if !frame.done() {
        return Err(ReplError::Malformed);
    }
    let mut header = Input::new(payload);
    let deps = header.unsigned()?;
    if deps > 20_001 {
        return Err(ReplError::EpochBound);
    }
    header.take(deps.checked_mul(32).ok_or(ReplError::EpochBound)?)?;
    if header.sized()?.len() != 32 {
        return Err(ReplError::EpochAuthority);
    }
    header.unsigned()?; // sequence
    header.unsigned()?; // starting operation counter
    header.signed()?; // advisory timestamp
    header.sized()?; // advisory message, checked against actual remaining input
    let actors = header.unsigned()?;
    if actors > 20_001 {
        return Err(ReplError::EpochBound);
    }
    for _ in 0..actors {
        if header.sized()?.len() != 32 {
            return Err(ReplError::EpochAuthority);
        }
    }
    let count = header.unsigned()?;
    if count > SPECS.len() as u64 {
        return Err(ReplError::EpochBound);
    }
    let mut columns = Vec::with_capacity(count as usize);
    let mut previous = None;
    for _ in 0..count {
        let spec = header.unsigned()?;
        if !SPECS.contains(&spec) || previous.is_some_and(|old| old >= spec) {
            return Err(ReplError::Malformed);
        }
        previous = Some(spec);
        columns.push((spec, header.unsigned()?));
    }
    let mut cells = 0;
    let mut strings = 0;
    let mut predecessors = 0;
    let mut value_bytes = 0;
    let mut actions = 0;
    for (spec, len) in columns {
        let data = header.take(len)?;
        let typ = spec & 7;
        if typ == 7 {
            continue;
        }
        let mut column = Input::new(data);
        while !column.done() {
            if typ == 4 {
                charge(&mut cells, column.unsigned()?, MAX_CELLS)?;
                continue;
            }
            let run = column.signed()?;
            let count = if run == 0 {
                column.unsigned()?
            } else {
                run.checked_abs().ok_or(ReplError::EpochBound)? as u64
            };
            charge(&mut cells, count, MAX_CELLS)?;
            if spec == 0x42 {
                charge(&mut actions, count, max_actions)?;
            }
            if run == 0 {
                continue;
            }
            let (literals, repeats) = if run < 0 { (count, 1) } else { (1, count) };
            // Literal loops are bounded by both the expanded-cell budget and encoded input;
            // repeated runs are charged once and never allocated or expanded by this scanner.
            for _ in 0..literals {
                match typ {
                    0 | 1 | 2 | 6 => {
                        let value = column.unsigned()?;
                        if spec == 0x70 {
                            charge(
                                &mut predecessors,
                                value.checked_mul(repeats).ok_or(ReplError::EpochBound)?,
                                MAX_PREDECESSORS,
                            )?;
                        }
                        if typ == 6 {
                            charge(
                                &mut value_bytes,
                                (value >> 4)
                                    .checked_mul(repeats)
                                    .ok_or(ReplError::EpochBound)?,
                                bytes.len() as u64,
                            )?;
                        }
                    }
                    3 => {
                        column.signed()?;
                    }
                    5 => {
                        let text = column.sized()?;
                        charge(
                            &mut strings,
                            (text.len() as u64)
                                .checked_mul(repeats)
                                .ok_or(ReplError::EpochBound)?,
                            MAX_EXPANDED_STRINGS,
                        )?;
                    }
                    _ => return Err(ReplError::Malformed),
                }
            }
        }
    }
    if actions == 0 {
        return Err(ReplError::Malformed);
    }
    // Remaining bytes are Automerge's opaque extra bytes; the enclosing raw-byte cap covers
    // them. Seeds separately require no extras, while P1 domain validators may be stricter.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unsigned(mut value: u64, out: &mut Vec<u8>) {
        loop {
            let byte = (value & 127) as u8;
            value >>= 7;
            out.push(byte | if value == 0 { 0 } else { 128 });
            if value == 0 {
                break;
            }
        }
    }
    fn raw(columns: &[(u64, Vec<u8>)]) -> Vec<u8> {
        let mut payload = vec![0, 32];
        payload.extend([1; 32]);
        payload.extend([1, 1, 0, 0, 0]);
        unsigned(columns.len() as u64, &mut payload);
        for (spec, data) in columns {
            unsigned(*spec, &mut payload);
            unsigned(data.len() as u64, &mut payload);
        }
        for (_, data) in columns {
            payload.extend(data);
        }
        let mut out = vec![0x85, 0x6f, 0x4a, 0x83, 0, 0, 0, 0, 1];
        unsigned(payload.len() as u64, &mut out);
        out.extend(payload);
        out
    }

    #[test]
    fn raw_runs_cannot_expand_past_the_semantic_action_limit() {
        // SLEB 8 followed by repeated action 1 is small on the wire but too large for registry.
        assert!(matches!(
            check(&raw(&[(0x42, vec![8, 1])]), 7),
            Err(ReplError::EpochBound)
        ));
        assert!(check(&raw(&[(0x42, vec![7, 1])]), 7).is_ok());
        assert!(check(
            &raw(&[(0x42, vec![0x80, 0x80, 0x80, 0x80, 0x08, 1])]),
            MAX_ACTIONS
        )
        .is_err());
    }

    #[test]
    fn predecessor_totals_and_string_allocations_are_checked_before_decode() {
        let mut huge = vec![1];
        unsigned(MAX_PREDECESSORS + 1, &mut huge);
        assert!(matches!(
            check(&raw(&[(0x42, vec![1, 1]), (0x70, huge)]), 7),
            Err(ReplError::EpochBound)
        ));
        // A one-gigabyte declared string with no body must not allocate a one-gigabyte buffer.
        let mut string = vec![1];
        unsigned(1 << 30, &mut string);
        assert!(matches!(
            check(&raw(&[(0x15, string), (0x42, vec![1, 1])]), 7),
            Err(ReplError::Malformed)
        ));
        // Unknown/compressed column specs fail before interpreting their contents.
        assert!(check(&raw(&[(0x4a, vec![1, 1])]), 7).is_err());
    }
}
