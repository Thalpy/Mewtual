//! Non-evicting vault-level retry fences survive Leave's deletion of per-server history.
//! They prove only future local retry fencing. They never claim whether a retired request
//! originally committed, and never delete/rewrite an unrelated server snapshot.
use super::*;
use crate::durable_chat::DurableChatIdentity;
use catcoms_sync::durable_chat::{ChatAction, MAX_CHAT_RECEIPTS};
use std::collections::BTreeMap;
const MAX_FENCE_BYTES: usize = 16 * 1024 * 1024;
const DOMAIN: &str = "catcoms/chat-stop-fences/v1";

fn malformed() -> AppError {
    AppError::Invalid("invalid or unsupported retired chat fences".into())
}
fn encode(rows: &BTreeMap<[u8; 16], DurableChatIdentity>) -> Result<Vec<u8>, AppError> {
    if rows.len() > MAX_CHAT_RECEIPTS {
        return Err(malformed());
    }
    // Check the exact wire budget before allocating the complete encoding. Every field uses a
    // u32 length prefix; target is present exactly for the two mutation variants.
    let encoded_len = rows
        .values()
        .try_fold(4 + DOMAIN.len() + 5, |sum, row| {
            sum.checked_add(
                20 + 36
                    + 36
                    + 16
                    + 4
                    + row.message_id.len()
                    + 1
                    + if row.target.is_some() { 36 } else { 0 },
            )
        })
        .ok_or_else(malformed)?;
    if encoded_len > MAX_FENCE_BYTES {
        return Err(AppError::Invalid(
            "retired chat fence capacity exhausted".into(),
        ));
    }
    let mut e = Encoder::new();
    e.put_str(DOMAIN).map_err(|_| malformed())?;
    e.put_u8(1);
    e.put_u32(rows.len() as u32);
    for row in rows.values() {
        e.put_bytes(&row.token).map_err(|_| malformed())?;
        e.put_bytes(&row.binding).map_err(|_| malformed())?;
        e.put_bytes(&row.context).map_err(|_| malformed())?;
        e.put_u128(row.channel);
        e.put_str(&row.message_id).map_err(|_| malformed())?;
        e.put_u8(match row.action {
            ChatAction::Send => 0,
            ChatAction::Edit => 1,
            ChatAction::Delete => 2,
        });
        if let Some(target) = row.target {
            e.put_bytes(&target).map_err(|_| malformed())?;
        }
    }
    let bytes = e.finish();
    if bytes.len() > MAX_FENCE_BYTES {
        return Err(AppError::Invalid(
            "retired chat fence capacity exhausted".into(),
        ));
    }
    Ok(bytes)
}
fn decode(bytes: &[u8]) -> Result<BTreeMap<[u8; 16], DurableChatIdentity>, AppError> {
    if bytes.len() > MAX_FENCE_BYTES {
        return Err(malformed());
    }
    let mut d = Decoder::new(bytes);
    if d.get_str().map_err(|_| malformed())? != DOMAIN || d.get_u8().map_err(|_| malformed())? != 1
    {
        return Err(malformed());
    }
    let count = d.get_u32().map_err(|_| malformed())? as usize;
    if count > MAX_CHAT_RECEIPTS {
        return Err(malformed());
    }
    let mut rows = BTreeMap::new();
    for _ in 0..count {
        let token = d
            .get_bytes()
            .map_err(|_| malformed())?
            .try_into()
            .map_err(|_| malformed())?;
        let binding = d
            .get_bytes()
            .map_err(|_| malformed())?
            .try_into()
            .map_err(|_| malformed())?;
        let context = d
            .get_bytes()
            .map_err(|_| malformed())?
            .try_into()
            .map_err(|_| malformed())?;
        let channel = d.get_u128().map_err(|_| malformed())?;
        let id = d.get_str().map_err(|_| malformed())?;
        if id.is_empty() || id.len() > 128 {
            return Err(malformed());
        }
        let action = match d.get_u8().map_err(|_| malformed())? {
            0 => ChatAction::Send,
            1 => ChatAction::Edit,
            2 => ChatAction::Delete,
            _ => return Err(malformed()),
        };
        let target = if action == ChatAction::Send {
            None
        } else {
            Some(
                d.get_bytes()
                    .map_err(|_| malformed())?
                    .try_into()
                    .map_err(|_| malformed())?,
            )
        };
        let row = DurableChatIdentity {
            token,
            binding,
            context,
            channel,
            message_id: id.into(),
            action,
            target,
        };
        if rows.insert(token, row).is_some() {
            return Err(malformed());
        }
    }
    d.finish().map_err(|_| malformed())?;
    Ok(rows)
}
impl ServerStore {
    fn chat_fences(&self) -> Result<BTreeMap<[u8; 16], DurableChatIdentity>, AppError> {
        let path = self.dir.join("chat-stop-fences.bin");
        match fs::symlink_metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
            Ok(meta) if meta.file_type().is_file() => {}
            _ => return Err(malformed()),
        }
        let raw = catcoms_storage::blob::read_bounded(&path, MAX_FENCE_BYTES + 40)?
            .ok_or_else(malformed)?;
        decode(&unseal(&self.keys.db_key()?, &unframe(&raw)?)?)
    }
    /// Parse/authentication failures refuse all fresh native positive admissions.
    pub fn has_chat_stop_fence(&self, identity: &DurableChatIdentity) -> Result<bool, AppError> {
        let rows = self.chat_fences()?;
        match rows.get(&identity.token) {
            Some(saved) if saved != identity => Err(AppError::Invalid(
                "CHAT_SEND_TOKEN_CONFLICT: retired token binds another request".into(),
            )),
            Some(_) => Ok(true),
            None => Ok(false),
        }
    }
    pub fn load_chat_stop_fences(&self) -> Result<Vec<DurableChatIdentity>, AppError> {
        Ok(self.chat_fences()?.into_values().collect())
    }
    /// Merge and rewrite even an identical visible fence. A prior rename with failed parent sync
    /// is not durable evidence; only this successful save can acknowledge the negative terminal.
    /// The host must hold its mounted-store mutex and absence/registration custody throughout.
    pub fn save_chat_stop_fence(
        &self,
        identity: &DurableChatIdentity,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        let mut rows = self.chat_fences()?;
        if let Some(saved) = rows.get(&identity.token) {
            if saved != identity {
                return Err(AppError::Invalid(
                    "CHAT_SEND_TOKEN_CONFLICT: retired token binds another request".into(),
                ));
            }
        } else if rows.len() >= MAX_CHAT_RECEIPTS {
            return Err(AppError::Invalid(
                "retired chat fence capacity exhausted".into(),
            ));
        }
        rows.insert(identity.token, identity.clone());
        let bytes = encode(&rows)?;
        let sealed = seal(&self.keys.db_key()?, &bytes, rng)?;
        self.write_chat_fences_record(&frame(&sealed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::durable_chat::{DurableChatRequest, DurableSendRequest};
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;
    fn identity(token: u8, text: &str) -> DurableChatIdentity {
        DurableChatRequest::Send(DurableSendRequest {
            token: [token; 16],
            expected_context: [1; 32],
            channel: 5,
            text: text.into(),
            reply_to: String::new(),
        })
        .retired_identity()
    }
    #[test]
    fn chat_fences_actual_rename_failure_is_pending_fenced_and_exact_retry_is_durable() {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(191);
        let store = ServerStore::open(dir.path(), b"test", &mut rng).unwrap();
        let first = identity(1, "original");
        store.save_chat_stop_fence(&first, &mut rng).unwrap();
        let path = dir.path().join("chat-stop-fences.bin");
        let previous = fs::read(&path).unwrap();
        let second = identity(2, "another");
        store
            .chat_fence_pre_rename_failure
            .store(true, Ordering::SeqCst);
        assert!(store.save_chat_stop_fence(&second, &mut rng).is_err());
        assert_eq!(fs::read(&path).unwrap(), previous);
        assert!(!store.has_chat_stop_fence(&second).unwrap());
        store.fail_next_chat_parent_sync();
        assert!(matches!(
            store.save_chat_stop_fence(&second, &mut rng),
            Err(AppError::CommittedButNotDurable(_))
        ));
        assert!(store.has_chat_stop_fence(&first).unwrap());
        assert!(store.has_chat_stop_fence(&second).unwrap());
        assert!(store
            .has_chat_stop_fence(&identity(2, "foreign binding"))
            .is_err());
        // A fresh authenticated read sees the actual renamed fence, not a helper success flag.
        drop(store);
        let store = ServerStore::open(dir.path(), b"test", &mut rng).unwrap();
        assert!(store.has_chat_stop_fence(&second).unwrap());
        store.save_chat_stop_fence(&second, &mut rng).unwrap();
        assert_eq!(store.load_chat_stop_fences().unwrap().len(), 2);
        store.remove_server(44).unwrap();
        assert!(
            store.has_chat_stop_fence(&second).unwrap(),
            "Leave must never remove global negative history"
        );
    }
    #[test]
    fn chat_fences_invalid_version_bytes_and_global_bounds_fail_closed() {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(192);
        let store = ServerStore::open(dir.path(), b"test", &mut rng).unwrap();
        let request = identity(1, "fenced");
        let mut bad = Encoder::new();
        bad.put_str(DOMAIN).unwrap();
        bad.put_u8(255);
        bad.put_u32(0);
        store
            .write_chat_fences_record(&frame(
                &seal(&store.keys.db_key().unwrap(), &bad.finish(), &mut rng).unwrap(),
            ))
            .unwrap();
        assert!(store.has_chat_stop_fence(&request).is_err());
        assert!(store.save_chat_stop_fence(&request, &mut rng).is_err());
        let mut count = Encoder::new();
        count.put_str(DOMAIN).unwrap();
        count.put_u8(1);
        count.put_u32(MAX_CHAT_RECEIPTS as u32 + 1);
        assert!(decode(&count.finish()).is_err());
        let mut rows = BTreeMap::new();
        for i in 0..=MAX_CHAT_RECEIPTS {
            let mut row = request.clone();
            row.token = (i as u128).to_be_bytes();
            rows.insert(row.token, row);
        }
        assert!(encode(&rows).is_err());
        rows.pop_last();
        for row in rows.values_mut() {
            row.message_id = "x".repeat(128);
            row.action = ChatAction::Edit;
            row.target = Some([2; 32]);
        }
        assert!(
            encode(&rows).is_err(),
            "combined byte cap is checked before encoding allocation"
        );
    }
}
