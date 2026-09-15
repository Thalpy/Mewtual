//! Existing jam-patch:v1 validation, only for the Studio `set_patch` operation. No DSP or
//! sound publication is added. Keep the identity bytes identical to `jam-patch.ts`: its
//! declaration-order JSON is different from sorted-key Studio operation JSON.

use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{object, ReplError};

/// A bounded, validated synthesis recipe with its existing jam identity. Private fields prevent
/// mutation after validation; accepting opaque JSON here would bypass the later export contract.
#[derive(Clone, PartialEq, Eq)]
pub struct StudioPatch {
    value: Value,
    id: [u8; 32],
}

impl std::fmt::Debug for StudioPatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StudioPatch").finish_non_exhaustive()
    }
}

impl StudioPatch {
    /// Validate the exact recipe fields and numeric bounds from the existing jam contract.
    /// This is a tiny fixed schema (at most three oscillators); unknown fields always reject.
    pub fn new(value: &Value) -> Result<Self, ReplError> {
        let root = object(value)?;
        if root.len() != 6 || root.get("v").and_then(Value::as_u64) != Some(1) {
            return Err(ReplError::Malformed);
        }
        let oscillators = root
            .get("o")
            .and_then(Value::as_array)
            .ok_or(ReplError::Malformed)?;
        if !(1..=3).contains(&oscillators.len()) {
            return Err(ReplError::Malformed);
        }
        let mut encoded_osc = Vec::with_capacity(oscillators.len());
        for osc in oscillators {
            let [w, t, c, l] = numbers(
                osc,
                [("w", 0, 3), ("t", -24, 24), ("c", -50, 50), ("l", 0, 100)],
            )?;
            encoded_osc.push(format!("{{\"w\":{w},\"t\":{t},\"c\":{c},\"l\":{l}}}"));
        }
        let [a, d, s, r] = numbers(
            root.get("e").ok_or(ReplError::Malformed)?,
            [
                ("a", 0, 5000),
                ("d", 0, 5000),
                ("s", 0, 100),
                ("r", 0, 8000),
            ],
        )?;
        let [fm, fc, fq, fe] = numbers(
            root.get("f").ok_or(ReplError::Malformed)?,
            [
                ("m", 0, 2),
                ("c", 20, 18000),
                ("q", 0, 100),
                ("e", -100, 100),
            ],
        )?;
        let [lr, ld, lt] = numbers(
            root.get("l").ok_or(ReplError::Malformed)?,
            [("r", 1, 1200), ("d", 0, 100), ("t", 0, 2)],
        )?;
        let [xc, xd, xr] = numbers(
            root.get("x").ok_or(ReplError::Malformed)?,
            [("c", 0, 100), ("d", 0, 100), ("r", 0, 100)],
        )?;
        // JSON.stringify(validateJamPatch(...).patch) uses THIS declaration order for hashing.
        // Do not replace it with the sorted-key encoding of the enclosing Studio operation.
        let canonical = format!("{{\"v\":1,\"o\":[{}],\"e\":{{\"a\":{a},\"d\":{d},\"s\":{s},\"r\":{r}}},\"f\":{{\"m\":{fm},\"c\":{fc},\"q\":{fq},\"e\":{fe}}},\"l\":{{\"r\":{lr},\"d\":{ld},\"t\":{lt}}},\"x\":{{\"c\":{xc},\"d\":{xd},\"r\":{xr}}}}}", encoded_osc.join(","));
        Ok(Self {
            value: value.clone(),
            id: Sha256::digest(canonical.as_bytes()).into(),
        })
    }

    /// SHA-256 of the existing jam identity representation (not the Studio body representation).
    pub fn id(&self) -> [u8; 32] {
        self.id
    }

    /// Immutable validated fields, useful for canonical Studio serialization and future export.
    pub fn value(&self) -> &Value {
        &self.value
    }
}

fn numbers<const N: usize>(
    value: &Value,
    fields: [(&str, i64, i64); N],
) -> Result<[i64; N], ReplError> {
    let obj = object(value)?;
    if obj.len() != N {
        return Err(ReplError::Malformed);
    }
    let mut result = [0; N];
    for (out, (name, min, max)) in result.iter_mut().zip(fields) {
        let value = obj
            .get(name)
            .and_then(Value::as_i64)
            .ok_or(ReplError::Malformed)?;
        if !(min..=max).contains(&value) {
            return Err(ReplError::Malformed);
        }
        *out = value;
    }
    Ok(result)
}
