//! Actual-byte `.pixa` admission. Content validation grants no membership, receipt or reference
//! publication authority. Keep this grammar aligned with frontend pixa.ts/jam-recorder.ts.

use crate::AppError;
use catcoms_replication::studio::StudioPatch;
use serde_json::{Map, Value};
use std::collections::BTreeSet;

const TAKE_BYTES: usize = 524_288;
const MAX_EVENTS: usize = 20_000;

/// Borrowed encoded-frame spans, returned only after the entire package has been validated.
/// Raster consumers expand one frame at a time; no999-raster allocation occurs here.
pub struct ValidatedPixa<'a> {
    fps: u8,
    frames: Vec<&'a [u8]>,
}
impl std::fmt::Debug for ValidatedPixa<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ValidatedPixa").finish_non_exhaustive()
    }
}
impl<'a> ValidatedPixa<'a> {
    pub fn fps(&self) -> u8 {
        self.fps
    }
    pub fn frames(&self) -> &[&'a [u8]] {
        &self.frames
    }
}
fn invalid(reason: &str) -> AppError {
    AppError::Invalid(format!("invalid PIXA: {reason}"))
}
type Result<T> = std::result::Result<T, AppError>;

struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}
impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self
            .at
            .checked_add(n)
            .filter(|end| *end <= self.bytes.len())
            .ok_or_else(|| invalid("truncated framing"))?;
        let value = &self.bytes[self.at..end];
        self.at = end;
        Ok(value)
    }
    fn u16(&mut self) -> Result<usize> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().map_err(|_| invalid("u16"))?) as usize)
    }
    fn u32(&mut self) -> Result<usize> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().map_err(|_| invalid("u32"))?) as usize)
    }
}

/// Validate all lengths before decoding JSON/PIX. Strict canonical take equality rejects
/// duplicate keys, alternate number spellings, invalid UTF-8, event reorderings and old SFX drafts.
pub fn validate_pixa(bytes: &[u8]) -> Result<ValidatedPixa<'_>> {
    if bytes.len() > super::MAX_BOUNDED_BLOB_BYTES {
        return Err(invalid("whole byte budget"));
    }
    let mut c = Cursor { bytes, at: 0 };
    if c.take(4)? != b"PIXA" {
        return Err(invalid("magic"));
    }
    let fps = c.take(1)?[0];
    let count = c.u16()?;
    if !(1..=24).contains(&fps) || !(1..=999).contains(&count) {
        return Err(invalid("fps/frame count"));
    }
    let mut frames = Vec::new();
    let mut total = 0usize;
    for _ in 0..count {
        let len = c.u32()?;
        total = total
            .checked_add(len)
            .ok_or_else(|| invalid("frame bytes"))?;
        if len == 0 || len > super::PIX_MAX_BYTES || total > 8 * 1024 * 1024 {
            return Err(invalid("frame byte budget"));
        }
        frames.push(c.take(len)?);
    }
    let take_len = c.u32()?;
    if take_len == 0 || take_len > TAKE_BYTES {
        return Err(invalid("take byte budget"));
    }
    let take = c.take(take_len)?;
    let sfx = c.u16()?;
    if sfx > 4096 {
        return Err(invalid("SFX count"));
    }
    let entries = c.take(sfx * 8)?;
    if c.at != bytes.len() {
        return Err(invalid("trailing bytes"));
    }
    let patches = validate_take(take)?;
    for entry in entries.chunks_exact(8) {
        let frame = u16::from_le_bytes([entry[0], entry[1]]) as usize;
        let duration = u32::from_le_bytes(entry[4..8].try_into().map_err(|_| invalid("duration"))?);
        if frame >= count
            || entry[2] as usize >= patches
            || entry[3] > 127
            || !(1..=600_000).contains(&duration)
        {
            return Err(invalid("SFX bounds"));
        }
    }
    for frame in &frames {
        super::validate_pix(frame)?;
        if frame[4] != 191 || frame[5] != 143 {
            return Err(invalid("frame dimensions"));
        }
    }
    Ok(ValidatedPixa { fps, frames })
}

fn object(value: &Value) -> Result<&Map<String, Value>> {
    value.as_object().ok_or_else(|| invalid("take object"))
}
fn keys(value: &Map<String, Value>, expected: &[&str]) -> Result<()> {
    if value.len() != expected.len() || expected.iter().any(|key| !value.contains_key(*key)) {
        return Err(invalid("take fields"));
    }
    Ok(())
}
fn number(value: &Value, max: u64) -> Result<u64> {
    value
        .as_u64()
        .filter(|v| *v <= max)
        .ok_or_else(|| invalid("take integer"))
}
fn field<'a>(value: &'a Map<String, Value>, key: &str) -> Result<&'a Value> {
    value.get(key).ok_or_else(|| invalid("take missing field"))
}
fn num(value: &Map<String, Value>, key: &str, max: u64) -> Result<u64> {
    number(field(value, key)?, max)
}
fn identity(value: &Value) -> Result<&str> {
    value
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 256)
        .ok_or_else(|| invalid("take identity"))
}
fn list<'a>(
    value: &'a Map<String, Value>,
    key: &str,
    min: usize,
    max: usize,
) -> Result<&'a Vec<Value>> {
    field(value, key)?
        .as_array()
        .filter(|v| (min..=max).contains(&v.len()))
        .ok_or_else(|| invalid("take collection"))
}
fn canonical(value: &mut Value) -> Result<Vec<u8>> {
    fn sort(value: &mut Value) {
        match value {
            Value::Object(m) => {
                m.sort_keys();
                for v in m.values_mut() {
                    sort(v);
                }
            }
            Value::Array(a) => {
                for v in a {
                    sort(v);
                }
            }
            _ => {}
        }
    }
    sort(value);
    serde_json::to_vec(value).map_err(|_| invalid("take JSON"))
}

fn validate_take(bytes: &[u8]) -> Result<usize> {
    let mut value: Value =
        serde_json::from_slice(bytes).map_err(|_| invalid("take JSON/Unicode"))?;
    let root = object(&value)?;
    keys(
        root,
        &[
            "v", "group", "call", "met", "parts", "lanes", "patches", "events",
        ],
    )?;
    if num(root, "v", 1)? != 1 {
        return Err(invalid("take version"));
    }
    identity(field(root, "group")?)?;
    identity(field(root, "call")?)?;
    let met = object(field(root, "met")?)?;
    keys(met, &["bpm", "bpb"])?;
    if num(met, "bpm", 240)? < 40 || num(met, "bpb", 8)? < 1 {
        return Err(invalid("take meter"));
    }
    let parts = list(root, "parts", 1, 16)?;
    let mut seen = BTreeSet::new();
    for part in parts {
        if !seen.insert(identity(part)?) {
            return Err(invalid("duplicate participant"));
        }
    }
    let lanes = list(root, "lanes", 0, 64)?;
    let mut seen = BTreeSet::new();
    for lane in lanes {
        let lane = object(lane)?;
        keys(lane, &["src", "sn"])?;
        let src = num(lane, "src", (parts.len() - 1) as u64)?;
        let nonce = field(lane, "sn")?
            .as_str()
            .filter(|s| {
                s.len() == 16
                    && s.bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            })
            .ok_or_else(|| invalid("lane nonce"))?;
        if !seen.insert((src, nonce)) {
            return Err(invalid("duplicate lane"));
        }
    }
    let patches = list(root, "patches", 0, 64)?;
    let patch_count = patches.len();
    let mut seen = BTreeSet::new();
    for patch in patches {
        let patch = StudioPatch::new(patch).map_err(|_| invalid("take recipe"))?;
        if !seen.insert(patch.id()) {
            return Err(invalid("duplicate recipe"));
        }
    }
    let events = list(root, "events", 0, MAX_EVENTS)?;
    let mut last: Vec<Option<(u64, u64)>> = vec![None; lanes.len()];
    for event in events {
        let e = object(event)?;
        if lanes.is_empty() {
            return Err(invalid("event without lane"));
        }
        let lane = num(e, "lane", (lanes.len() - 1) as u64)? as usize;
        let ms = num(e, "ms", 600_000)?;
        let q = num(e, "q", u32::MAX as u64)?;
        if last[lane].is_some_and(|(old_q, old_ms)| q <= old_q || ms < old_ms) {
            return Err(invalid("lane event order"));
        }
        last[lane] = Some((q, ms));
        if e.get("d").and_then(Value::as_u64) == Some(1) {
            keys(e, &["ms", "lane", "n", "d", "q"])?;
            num(e, "n", 9)?;
        } else if e.get("on").and_then(Value::as_u64) == Some(0) {
            keys(e, &["ms", "lane", "n", "on", "q"])?;
            num(e, "n", 127)?;
        } else {
            if e.contains_key("p") {
                keys(e, &["ms", "lane", "n", "on", "w", "p", "q"])?;
                if patch_count == 0 {
                    return Err(invalid("event recipe"));
                }
                num(e, "p", (patch_count - 1) as u64)?;
            } else {
                keys(e, &["ms", "lane", "n", "on", "w", "q"])?;
            }
            if num(e, "on", 1)? != 1 {
                return Err(invalid("event on"));
            }
            num(e, "n", 127)?;
            if !matches!(
                field(e, "w")?.as_str(),
                Some("sine" | "triangle" | "square" | "sawtooth")
            ) {
                return Err(invalid("event wave"));
            }
        }
    }
    // The frontend normalizes inter-lane order while preserving each lane's strict q sequence.
    value
        .get_mut("events")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| invalid("events"))?
        .sort_by_key(|e| (e["ms"].as_u64(), e["lane"].as_u64(), e["q"].as_u64()));
    if canonical(&mut value)? != bytes {
        return Err(invalid("noncanonical take"));
    }
    Ok(patch_count)
}

#[cfg(test)]
mod tests;
