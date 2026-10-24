//! Native creative-blob seam. No renderer, Studio state or cache lives here. Reuses the inline
//! download slot/cancellation authority, so lock and transport-owned work keep the same bounds.

use super::*;
use catcoms_app::creative::{PublishedPix, MAX_BOUNDED_BLOB_BYTES, PIX_MAX_BYTES};

#[derive(Debug, Serialize)]
pub(crate) struct PixPublication {
    cid: String,
    bytes: usize,
}

impl From<PublishedPix> for PixPublication {
    fn from(value: PublishedPix) -> Self {
        Self {
            cid: value.cid,
            bytes: value.bytes,
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct BlobBytes {
    bytes_b64: String,
    bytes: usize,
}

fn decode_pix_input(encoded: &str) -> Result<Vec<u8>, String> {
    // IPC has already parsed its string. Refuse before base64 allocates the decoded blob.
    if encoded.len() > PIX_MAX_BYTES.div_ceil(3) * 4 {
        return Err("PIX blob exceeds 64 KiB".into());
    }
    let bytes = B64
        .decode(encoded)
        .map_err(|_| "invalid PIX base64".to_string())?;
    catcoms_app::creative::validate_pix(&bytes).map_err(|e| e.to_string())?;
    Ok(bytes)
}

fn parse_bounded_request(cid: &str, max_bytes: usize) -> Result<Cid, String> {
    if max_bytes > MAX_BOUNDED_BLOB_BYTES {
        return Err("blob limit exceeds 9 MiB".into());
    }
    if cid.len() != 64
        || !cid
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("CID must be 64 lowercase hex characters".into());
    }
    Cid::from_hex(cid).ok_or_else(|| "invalid CID".into())
}

/// Suppress stale success/bytes after lock-unlock or removing and reopening the same server id.
/// Hold both gates through conversion; do not cache plaintext for a later session.
async fn finish_response<T>(
    state: &AppState,
    server: u64,
    instance: u64,
    generation: u64,
    make: impl FnOnce() -> T,
) -> Result<T, String> {
    let _commit = require_ui_session_generation(state, generation).await?;
    let servers = state.servers.lock().await;
    if servers
        .get(&server)
        .is_none_or(|entry| entry.instance != instance)
    {
        return Err("the server changed while the blob request was in progress".into());
    }
    Ok(make())
}

/// Store PIX bytes before publishing a metadata reference. Returns `{cid, bytes}` after native
/// validation, promotion and file flush. This command alone does not save a flipnote document.
#[tauri::command]
pub(crate) async fn publish_pix(
    state: State<'_, AppState>,
    server: u64,
    bytes_b64: String,
) -> Result<PixPublication, String> {
    let generation = unlocked_ui_session_generation(&state).await?;
    let (lease, signal) = claim_internal_inline_download(&state)?;
    let bytes = decode_pix_input(&bytes_b64)?;
    let (actor, instance) = actor_instance_of(&state, server).await?;
    let cancellation = RequestCancellation::new(signal, Some(lease.request_keepalive()));
    let published = actor.publish_pix(bytes, Some(cancellation)).await?;
    finish_response(&state, server, instance, generation, || published.into()).await
}

/// Return `{bytes_b64, bytes}` or null if unavailable. Callers pass their record's declared size
/// as `maxBytes`, then require exact equality and decode their format before rendering. An
/// oversize/authentication/CID error rejects; this call never retries another provider.
#[tauri::command]
pub(crate) async fn request_blob_bounded(
    state: State<'_, AppState>,
    server: u64,
    cid: String,
    max_bytes: usize,
) -> Result<Option<BlobBytes>, String> {
    let generation = unlocked_ui_session_generation(&state).await?;
    let cid = parse_bounded_request(&cid, max_bytes)?;
    let (lease, signal) = claim_internal_inline_download(&state)?;
    let (actor, instance) = actor_instance_of(&state, server).await?;
    let cancellation = RequestCancellation::new(signal, Some(lease.request_keepalive()));
    let bytes = actor
        .request_blob_bounded(cid, max_bytes, Some(cancellation))
        .await?;
    finish_response(&state, server, instance, generation, || {
        bytes.map(|bytes| BlobBytes {
            bytes_b64: B64.encode(&bytes),
            bytes: bytes.len(),
        })
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn creative_bridge_failed_disk_attachment_cannot_report_saved_pixels() {
        use catcoms_rt::{Hub, ManualClock};
        use rand_chacha::ChaCha20Rng;
        use rand_core::SeedableRng;
        let root = tempfile::tempdir().unwrap();
        let state = AppState::default();
        *state.store.lock().await =
            Some(ServerStore::open(root.path(), b"correct horse", &mut OsCryptoRng).unwrap());
        // A regular file where the blob namespace directory belongs forces the real production
        // attachment helper down its best-effort memory fallback, without mocking that path.
        std::fs::write(root.path().join("blobs"), b"blocked").unwrap();
        let mut server = Server::found(
            Hub::new().join(PeerId::from_u64(91)),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(91),
            Box::new(ManualClock::new(1_000)),
            "alice",
        )
        .unwrap();
        attach_blob_store(&state, &mut server).await;
        let (actor, events, task) = spawn(server);
        let pix = vec![
            80, 73, 88, 49, 0, 0, 3, 1, 0, 0, 0, 2, 1, 1, 1, 3, 2, 2, 2, 0, 3, 3, 3, 0, 0,
        ];
        let cid = Cid::of(&pix);
        assert!(actor
            .publish_pix(pix, None)
            .await
            .unwrap_err()
            .contains("persistent"));
        assert!(actor
            .request_blob_bounded(cid, 25, None)
            .await
            .unwrap()
            .is_none());
        actor.shutdown().await;
        task.await.unwrap();
        drop(events);
    }

    #[tokio::test]
    async fn creative_bridge_suppresses_results_after_lock_or_server_replacement() {
        use catcoms_rt::{Hub, ManualClock};
        use rand_chacha::ChaCha20Rng;
        use rand_core::SeedableRng;
        let root = tempfile::tempdir().unwrap();
        let state = AppState::default();
        *state.store.lock().await =
            Some(ServerStore::open(root.path(), b"correct horse", &mut OsCryptoRng).unwrap());
        *state.session_resumable.lock().await = true;
        let generation = unlocked_ui_session_generation(&state).await.unwrap();
        let server = Server::found(
            Hub::new().join(PeerId::from_u64(91)),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(91),
            Box::new(ManualClock::new(1_000)),
            "alice",
        )
        .unwrap();
        let group_id = server.group_id();
        let device_id = server.device_id();
        let (actor, events, task) = spawn(server);
        state.servers.lock().await.insert(
            12,
            ServerEntry {
                actor: actor.clone(),
                instance: 40,
                group_id,
                device_id,
                invite: None,
                name: "test".into(),
                bootstrap: Vec::new(),
                bootstrap_owners: HashMap::new(),
                interface_routes: None,
                rendezvous: Vec::new(),
                mesh: None,
                is_dm: false,
                switchboard: false,
                record_seq: 0,
                persist: PersistCounters::default(),
            },
        );
        assert_eq!(
            finish_response(&state, 12, 40, generation, || 42)
                .await
                .unwrap(),
            42
        );
        state.servers.lock().await.get_mut(&12).unwrap().instance = 41;
        assert!(finish_response(&state, 12, 40, generation, || panic!(
            "stale bytes converted"
        ))
        .await
        .is_err());
        assert_eq!(
            finish_response(&state, 12, 41, generation, || 42)
                .await
                .unwrap(),
            42
        );
        state.session_lock_requested.store(true, Ordering::Release);
        assert!(finish_response(&state, 12, 41, generation, || panic!(
            "locked bytes converted"
        ))
        .await
        .is_err());
        state.session_lock_requested.store(false, Ordering::Release);
        state.ui_session_generation.fetch_add(1, Ordering::AcqRel);
        assert!(
            finish_response(&state, 12, 41, generation, || panic!("old session revived"))
                .await
                .is_err()
        );
        actor.shutdown().await;
        task.await.unwrap();
        drop(events);
    }

    #[test]
    fn creative_bridge_input_caps_and_wire_results() {
        let pix = vec![
            80, 73, 88, 49, 0, 0, 3, 1, 0, 0, 0, 2, 1, 1, 1, 3, 2, 2, 2, 0, 3, 3, 3, 0, 0,
        ];
        assert_eq!(decode_pix_input(&B64.encode(&pix)).unwrap(), pix);
        assert!(decode_pix_input("!").is_err());
        assert!(decode_pix_input(&"A".repeat(PIX_MAX_BYTES.div_ceil(3) * 4 + 1)).is_err());
        assert!(decode_pix_input(&B64.encode(vec![0; PIX_MAX_BYTES + 1])).is_err());
        let cid = Cid::of(&pix).to_hex();
        assert!(parse_bounded_request(&cid, MAX_BOUNDED_BLOB_BYTES).is_ok());
        assert!(parse_bounded_request(&cid, MAX_BOUNDED_BLOB_BYTES + 1).is_err());
        assert!(parse_bounded_request(&cid.to_uppercase(), 25).is_err());
        assert!(parse_bounded_request(&cid[..63], 25).is_err());
        let result: PixPublication = PublishedPix {
            cid: cid.clone(),
            bytes: pix.len(),
        }
        .into();
        assert_eq!(
            serde_json::to_value(result).unwrap(),
            serde_json::json!({"cid": cid, "bytes": 25})
        );
        assert_eq!(
            serde_json::to_value(BlobBytes {
                bytes_b64: "AA==".into(),
                bytes: 1
            })
            .unwrap(),
            serde_json::json!({"bytes_b64":"AA==", "bytes":1})
        );
    }
}
