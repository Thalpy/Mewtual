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
fn validate_identity(identity: &DurableChatIdentity) -> Result<(), AppError> {
    if identity.message_id.is_empty()
        || identity.message_id.len() > 128
        || (identity.action == ChatAction::Send) != identity.target.is_none()
    {
        return Err(malformed());
    }
    Ok(())
}
fn encode(rows: &BTreeMap<(u64, [u8; 16]), DurableChatIdentity>) -> Result<Vec<u8>, AppError> {
    if rows.len() > MAX_CHAT_RECEIPTS {
        return Err(malformed());
    }
    for ((_, token), row) in rows {
        validate_identity(row)?;
        if token != &row.token {
            return Err(malformed());
        }
    }
    // Check the exact wire budget before allocating the complete encoding. Every field uses a
    // u32 length prefix; target is present exactly for the two mutation variants.
    let encoded_len = rows
        .values()
        .try_fold(4 + DOMAIN.len() + 5, |sum, row| {
            sum.checked_add(
                8 + 20
                    + 36
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
    for ((server, _), row) in rows {
        e.put_u64(*server);
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
fn decode(bytes: &[u8]) -> Result<BTreeMap<(u64, [u8; 16]), DurableChatIdentity>, AppError> {
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
        let server = d.get_u64().map_err(|_| malformed())?;
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
        if rows.insert((server, token), row).is_some() {
            return Err(malformed());
        }
    }
    d.finish().map_err(|_| malformed())?;
    Ok(rows)
}
impl ServerStore {
    fn chat_fences(&self) -> Result<BTreeMap<(u64, [u8; 16]), DurableChatIdentity>, AppError> {
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
    pub fn has_chat_stop_fence(
        &self,
        server: u64,
        identity: &DurableChatIdentity,
    ) -> Result<bool, AppError> {
        let rows = self.chat_fences()?;
        match rows.get(&(server, identity.token)) {
            Some(saved) if saved != identity => Err(AppError::Invalid(
                "CHAT_SEND_TOKEN_CONFLICT: retired token binds another request".into(),
            )),
            Some(_) => Ok(true),
            None => Ok(false),
        }
    }
    pub fn load_chat_stop_fences(&self, server: u64) -> Result<Vec<DurableChatIdentity>, AppError> {
        Ok(self
            .chat_fences()?
            .into_iter()
            .filter_map(|((id, _), identity)| (id == server).then_some(identity))
            .collect())
    }
    /// Merge and rewrite even an identical visible fence. A prior rename with failed parent sync
    /// is not durable evidence; only this successful save can acknowledge the negative terminal.
    /// The host must hold its mounted-store mutex and absence/registration custody throughout.
    pub fn save_chat_stop_fence(
        &self,
        server: u64,
        identity: &DurableChatIdentity,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        validate_identity(identity)?;
        let mut rows = self.chat_fences()?;
        if let Some(saved) = rows.get(&(server, identity.token)) {
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
        rows.insert((server, identity.token), identity.clone());
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
        store.save_chat_stop_fence(9, &first, &mut rng).unwrap();
        let path = dir.path().join("chat-stop-fences.bin");
        let previous = fs::read(&path).unwrap();
        let second = identity(2, "another");
        store
            .chat_fence_pre_rename_failure
            .store(true, Ordering::SeqCst);
        assert!(store.save_chat_stop_fence(9, &second, &mut rng).is_err());
        assert_eq!(fs::read(&path).unwrap(), previous);
        assert!(!store.has_chat_stop_fence(9, &second).unwrap());
        store.fail_next_chat_parent_sync();
        assert!(matches!(
            store.save_chat_stop_fence(9, &second, &mut rng),
            Err(AppError::CommittedButNotDurable(_))
        ));
        assert!(store.has_chat_stop_fence(9, &first).unwrap());
        assert!(store.has_chat_stop_fence(9, &second).unwrap());
        assert!(store
            .has_chat_stop_fence(9, &identity(2, "foreign binding"))
            .is_err());
        // A fresh authenticated read sees the actual renamed fence, not a helper success flag.
        drop(store);
        let store = ServerStore::open(dir.path(), b"test", &mut rng).unwrap();
        assert!(store.has_chat_stop_fence(9, &second).unwrap());
        store.save_chat_stop_fence(9, &second, &mut rng).unwrap();
        assert_eq!(store.load_chat_stop_fences(9).unwrap().len(), 2);
        store.remove_server(44).unwrap();
        assert!(
            store.has_chat_stop_fence(9, &second).unwrap(),
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
        assert!(store.has_chat_stop_fence(9, &request).is_err());
        assert!(store.save_chat_stop_fence(9, &request, &mut rng).is_err());
        let mut count = Encoder::new();
        count.put_str(DOMAIN).unwrap();
        count.put_u8(1);
        count.put_u32(MAX_CHAT_RECEIPTS as u32 + 1);
        assert!(decode(&count.finish()).is_err());
        let mut rows = BTreeMap::new();
        for i in 0..=MAX_CHAT_RECEIPTS {
            let mut row = request.clone();
            row.token = (i as u128).to_be_bytes();
            rows.insert((9, row.token), row);
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
    #[test]
    fn retired_chat_fences_bind_numeric_server_and_allow_independent_equal_tokens() {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(196);
        let store = ServerStore::open(dir.path(), b"test", &mut rng).unwrap();
        let first = identity(1, "first scope");
        let second = identity(1, "other scope");
        store.save_chat_stop_fence(999, &first, &mut rng).unwrap();
        assert!(!store.has_chat_stop_fence(12, &first).unwrap());
        assert!(!store.has_chat_stop_fence(12, &second).unwrap());
        store.save_chat_stop_fence(12, &second, &mut rng).unwrap();
        assert!(store.has_chat_stop_fence(999, &first).unwrap());
        assert!(store.has_chat_stop_fence(12, &second).unwrap());
        assert!(store.has_chat_stop_fence(12, &first).is_err());
        assert_eq!(store.load_chat_stop_fences(12).unwrap(), vec![second]);
        assert_eq!(store.load_chat_stop_fences(999).unwrap(), vec![first]);
    }
    #[test]
    fn invalid_public_retired_identity_cannot_replace_readable_fence_history() {
        use crate::durable_chat::{DurableMutationAction, DurableMutationRequest};
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(198);
        let store = ServerStore::open(dir.path(), b"test", &mut rng).unwrap();
        let valid = identity(1, "retained");
        store.save_chat_stop_fence(9, &valid, &mut rng).unwrap();
        let previous = fs::read(dir.path().join("chat-stop-fences.bin")).unwrap();
        for id in [String::new(), "x".repeat(129)] {
            let invalid = DurableChatRequest::Mutation(DurableMutationRequest {
                token: [2; 16],
                expected_context: [1; 32],
                channel: 5,
                message_id: id,
                target: [1; 32],
                action: DurableMutationAction::Delete,
            })
            .retired_identity();
            assert!(store.save_chat_stop_fence(9, &invalid, &mut rng).is_err());
            assert_eq!(
                fs::read(dir.path().join("chat-stop-fences.bin")).unwrap(),
                previous
            );
            assert!(store.has_chat_stop_fence(9, &valid).unwrap());
        }
    }
}
