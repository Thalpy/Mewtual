//! Non-evicting immutable root pins. Rows are trusted local configuration, not public MLS proof.
use super::*;
const DOMAIN: &str = "catcoms/chat-owner-pins/v1";
pub(super) const FILE: &str = "chat-owner-pins.bin";
const MAX_ROWS: usize = 65_536;
const MAX_BYTES: usize = 32 * 1024 * 1024;
const RESERVED_ROW_BYTES: usize = 384;
const MIN_ROW_BYTES: usize = 114;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Origin {
    PrivateFounder,
    TrustedHost,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Pin {
    pub(super) founder: [u8; 32],
    pub(super) profile: OwnerFactId,
    pub(super) origin: Origin,
    pub(super) source: [u8; 32],
}
#[derive(Default)]
pub(super) struct Runtime {
    pub(super) table: Option<BTreeMap<Vec<u8>, Pin>>,
    pub(super) dirty: bool,
    pub(super) managed: bool,
}
pub(super) fn refused() -> AppError {
    AppError::Invalid("owner evidence configuration is unavailable or unsupported".into())
}
pub(super) fn fits_rows(count: usize) -> bool {
    count <= MAX_ROWS
        && count
            .checked_mul(RESERVED_ROW_BYTES)
            .and_then(|bytes| bytes.checked_add(4 + DOMAIN.len() + 4))
            .is_some_and(|bytes| bytes <= MAX_BYTES)
}

fn encode(table: &BTreeMap<Vec<u8>, Pin>) -> Result<Vec<u8>, AppError> {
    if !fits_rows(table.len())
        || table
            .keys()
            .any(|group| group.is_empty() || group.len() > 256)
    {
        return Err(refused());
    }
    // The reserved bound is checked BEFORE materializing the complete plaintext. Max row369B
    // is covered by384B including variable group length; no counted row can evade byte accounting.
    let mut e = Encoder::new();
    e.put_str(DOMAIN).map_err(|_| refused())?;
    e.put_u32(table.len() as u32);
    for (group, pin) in table {
        e.put_bytes(group).map_err(|_| refused())?;
        e.put_bytes(&pin.founder).map_err(|_| refused())?;
        e.put_bytes(pin.profile.as_bytes()).map_err(|_| refused())?;
        e.put_u8(match pin.origin {
            Origin::PrivateFounder => 0,
            Origin::TrustedHost => 1,
        });
        e.put_bytes(&pin.source).map_err(|_| refused())?;
    }
    Ok(e.finish())
}
fn decode(bytes: &[u8]) -> Result<BTreeMap<Vec<u8>, Pin>, AppError> {
    if bytes.len() > MAX_BYTES {
        return Err(refused());
    }
    let mut d = Decoder::new(bytes);
    if d.get_str().map_err(|_| refused())? != DOMAIN {
        return Err(refused());
    }
    let count = d.get_u32().map_err(|_| refused())? as usize;
    if count > MAX_ROWS || count > d.remaining() / MIN_ROW_BYTES {
        return Err(refused());
    }
    let mut table = BTreeMap::new();
    for _ in 0..count {
        let group = d.get_bytes().map_err(|_| refused())?;
        if group.is_empty()
            || group.len() > 256
            || table
                .last_key_value()
                .is_some_and(|(last, _): (&Vec<u8>, &Pin)| last.as_slice() >= group)
        {
            return Err(refused());
        }
        let founder = d
            .get_bytes()
            .map_err(|_| refused())?
            .try_into()
            .map_err(|_| refused())?;
        let profile = OwnerFactId::from_bytes(
            d.get_bytes()
                .map_err(|_| refused())?
                .try_into()
                .map_err(|_| refused())?,
        );
        let origin = match d.get_u8().map_err(|_| refused())? {
            0 => Origin::PrivateFounder,
            1 => Origin::TrustedHost,
            _ => return Err(refused()),
        };
        let source = d
            .get_bytes()
            .map_err(|_| refused())?
            .try_into()
            .map_err(|_| refused())?;
        table.insert(
            group.to_vec(),
            Pin {
                founder,
                profile,
                origin,
                source,
            },
        );
    }
    d.finish().map_err(|_| refused())?;
    Ok(table)
}

impl ServerStore {
    /// Constructor-only load from the same authenticated vault-format result. Even a legacy
    /// wrapper cannot discard a previously installed nonempty owner registry after partial rollback.
    pub(in crate::store) fn open_owner_evidence_inventory(
        &self,
        format: catcoms_storage::VaultFormat,
    ) -> Result<(), AppError> {
        let mut roots = self.owner_evidence.pins.lock().map_err(|_| refused())?;
        if roots.table.is_some() || roots.dirty {
            return Err(refused());
        }
        let managed = format == catcoms_storage::VaultFormat::OwnerEvidence;
        // Exclusive Store::open owns the physical vault session. This one fixed stage can only
        // be pre-rename unpublished bytes; it is never the current registry or a canonical head.
        self.cleanup_owner_pins_stage_for_open(format)?;
        let table = self.read_owner_pins(managed)?;
        if !managed && !table.is_empty() {
            return Err(refused());
        }
        roots.table = Some(table);
        roots.managed = managed;
        Ok(())
    }
    fn read_owner_pins(&self, managed: bool) -> Result<BTreeMap<Vec<u8>, Pin>, AppError> {
        let path = self.dir.join(FILE);
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return if managed {
                    Err(refused())
                } else {
                    Ok(BTreeMap::new())
                };
            }
            Ok(metadata) if metadata.file_type().is_file() => {}
            _ => return Err(refused()),
        }
        let Some(raw) = catcoms_storage::blob::read_bounded(&path, MAX_BYTES + 40)? else {
            return if managed {
                Err(refused())
            } else {
                Ok(BTreeMap::new())
            };
        };
        let plain = Zeroizing::new(unseal(&self.keys.db_key()?, &unframe(&raw)?)?);
        decode(&plain)
    }
    /// Retry writes the WHOLE original possibly installed table under one global serializer.
    /// No epoch/role/alias change clears this debt and no visible-file-is-durable inference exists.
    pub(super) fn flush_owner_pins(
        &self,
        roots: &mut Runtime,
        original_mount: &RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        roots.dirty = true;
        let bytes = Zeroizing::new(encode(roots.table.as_ref().ok_or_else(refused)?)?);
        let sealed = seal(&self.keys.db_key()?, &bytes, rng)?;
        if original_mount.is_cancelled() {
            return Err(refused());
        }
        self.write_owner_pins_record(&frame(&sealed))?;
        roots.dirty = false;
        Ok(())
    }
}
