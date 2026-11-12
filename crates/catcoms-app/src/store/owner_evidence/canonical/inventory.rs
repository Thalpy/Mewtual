//! Bounded exact namespace inventory and one fixed staging slot per retained logical head.
use super::*;

impl ServerStore {
    pub(super) fn inventory_owner_heads(
        &self,
        runtime: &mut Runtime,
        budget: OwnerEvidenceBudget,
    ) -> Result<(), AppError> {
        let directory = self.owner_canonical_directory()?;
        let mut inventory = BTreeMap::new();
        let mut examined = 0usize;
        for entry in fs::read_dir(&directory).map_err(|e| AppError::Io(e.to_string()))? {
            examined = examined.checked_add(1).ok_or_else(refused)?;
            if examined > budget.max_heads.checked_mul(2).ok_or_else(refused)? {
                return Err(refused());
            }
            let entry = entry.map_err(|e| AppError::Io(e.to_string()))?;
            let name = entry.file_name();
            let name = name.to_str().ok_or_else(refused)?;
            let identifier = if let Some(key) = name.strip_suffix(".bin") {
                decode_key(key)?
            } else if let Some(key) = name.strip_suffix(".replace.tmp") {
                decode_key(key)?
            } else {
                return Err(refused());
            };
            let metadata =
                fs::symlink_metadata(entry.path()).map_err(|e| AppError::Io(e.to_string()))?;
            if !metadata.file_type().is_file()
                || metadata.len() > (budget.snapshot + WRAPPER_FEE + SEALED_FEE) as u64
            {
                return Err(refused());
            }
            // A stage can be the only retained bytes after interrupted first initialization.
            // Do not erase it before the later negative-recovery classifier checks absence.
            // It reserves the same full current+replacement allowance as a canonical file;
            // exact staged-candidate recovery is separate from ordinary canonical restore.
            inventory
                .entry(identifier)
                .or_insert_with(|| Arc::new(head::HeadCell::new(identifier)));
            if inventory.len() > budget.max_heads
                || (inventory.len() as u64)
                    .checked_mul(budget.head_fee()?)
                    .is_none_or(|fee| fee > budget.persistent_bytes)
            {
                return Err(refused());
            }
        }
        // Never install a partially inventoried or damaged namespace as an empty archive.
        runtime.heads = inventory;
        Ok(())
    }
}
fn decode_key(value: &str) -> Result<[u8; 32], AppError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(refused());
    }
    let mut key = [0; 32];
    hex::decode_to_slice(value, &mut key).map_err(|_| refused())?;
    Ok(key)
}
