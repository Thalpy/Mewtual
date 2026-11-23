//! Exact private records for possible Welcome entry and its completed result.
use super::*;
use catcoms_replication::chat::cold_selected_history::{
    SelectedHistoryNodePage, MAX_SELECTED_HISTORY_NODE_PAGE_BYTES,
};
use catcoms_sync::{
    SealedV2JoinedOutcome, V2JoinDeliveryData, V2PendingJoinContextData,
    MAX_V2_GOVERNING_JOIN_DELIVERY_BYTES,
};

const ENTRY_TAG: &[u8] = b"catcoms/private-chat-v2-welcome-entry/v1\0";
const ENTRY_KEY: &str = "catcoms/private-chat-v2-welcome-entry-key/v1";
const OUTCOME_TAG: &[u8] = b"catcoms/private-chat-v2-welcome-outcome/v1\0";
const OUTCOME_KEY: &str = "catcoms/private-chat-v2-welcome-outcome-key/v1";
const VERSION: u16 = 1;
const LATER_ENTRY: u16 = 2;
const OUTCOME_WITH_HISTORY: u16 = 2;
const SEALED_OVERHEAD: usize = 40;
const DIGESTS: usize = 5 * 36;
const MAX_OUTCOME_CIPHERTEXT: usize = 512 * 1024 + 4096;
pub(in crate::store) const ENTRY_FRAME: usize =
    4 + ENTRY_TAG.len() + 2 + DIGESTS + 4 + MAX_V2_GOVERNING_JOIN_DELIVERY_BYTES + SEALED_OVERHEAD;
pub(in crate::store) const OUTCOME_FRAME: usize = 4
    + OUTCOME_TAG.len()
    + 2
    + DIGESTS
    + 4
    + 24
    + 4
    + MAX_OUTCOME_CIPHERTEXT
    + 4
    + MAX_SELECTED_HISTORY_NODE_PAGE_BYTES
    + SEALED_OVERHEAD;

fn context_fields(context: V2PendingJoinContextData, parent: &[u8]) -> [[u8; 32]; 5] {
    [
        context.vault,
        context.root,
        context.original_pending,
        context.pin,
        *blake3::hash(parent).as_bytes(),
    ]
}

fn seal_record(
    key: &[u8; 32],
    domain: &str,
    tag: &[u8],
    version: u16,
    labels: [[u8; 32]; 5],
    fields: &[&[u8]],
    rng: &mut dyn CryptoRngCore,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    let size =
        4 + tag.len() + 2 + DIGESTS + fields.iter().map(|field| 4 + field.len()).sum::<usize>();
    let mut e = Encoder::with_capacity(size);
    let bad = |_| refused();
    e.put_bytes(tag).map_err(bad)?;
    e.put_u16(version);
    for field in labels {
        e.put_bytes(&field).map_err(bad)?;
    }
    for field in fields {
        e.put_bytes(field).map_err(bad)?;
    }
    let plain = Zeroizing::new(e.finish());
    let key = Zeroizing::new(blake3::derive_key(domain, key));
    Ok(Zeroizing::new(frame(&seal(&key, &plain, rng)?)))
}

fn open_record(
    key: &[u8; 32],
    domain: &str,
    frame_bytes: &[u8],
    cap: usize,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    if !(SEALED_OVERHEAD + 1..=cap).contains(&frame_bytes.len()) {
        return Err(refused());
    }
    let key = Zeroizing::new(blake3::derive_key(domain, key));
    Ok(Zeroizing::new(unseal(&key, &unframe(frame_bytes)?)?))
}

fn decoder<'a>(
    raw: &'a [u8],
    tag: &[u8],
    labels: [[u8; 32]; 5],
) -> Result<(u16, Decoder<'a>), AppError> {
    let mut d = Decoder::new(raw);
    let bad = |_| refused();
    if d.get_bytes().map_err(bad)? != tag {
        return Err(refused());
    }
    let version = d.get_u16().map_err(bad)?;
    for label in labels {
        if d.get_bytes().map_err(bad)? != label {
            return Err(refused());
        }
    }
    Ok((version, d))
}

fn entry_version(delivery: &[u8]) -> Result<u16, AppError> {
    Ok(
        if V2JoinDeliveryData::later_frame_data(delivery).map_err(|_| refused())? {
            LATER_ENTRY
        } else {
            VERSION
        },
    )
}

pub(in crate::store) fn seal_entry(
    key: &[u8; 32],
    context: V2PendingJoinContextData,
    pending: &[u8],
    delivery: &[u8],
    rng: &mut dyn CryptoRngCore,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    seal_record(
        key,
        ENTRY_KEY,
        ENTRY_TAG,
        entry_version(delivery)?,
        context_fields(context, pending),
        &[delivery],
        rng,
    )
}

pub(in crate::store) fn open_entry(
    key: &[u8; 32],
    context: V2PendingJoinContextData,
    pending: &[u8],
    raw: &[u8],
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    let plain = open_record(key, ENTRY_KEY, raw, ENTRY_FRAME)?;
    let (version, mut d) = decoder(&plain, ENTRY_TAG, context_fields(context, pending))?;
    let bytes = d.get_bytes().map_err(|_| refused())?;
    d.finish().map_err(|_| refused())?;
    if entry_version(bytes)? != version {
        return Err(refused());
    }
    Ok(Zeroizing::new(bytes.to_vec()))
}

pub(in crate::store) fn seal_outcome(
    key: &[u8; 32],
    context: V2PendingJoinContextData,
    entry: &[u8],
    result: &SealedV2JoinedOutcome,
    history: &[u8],
    rng: &mut dyn CryptoRngCore,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    let (nonce, bytes) = result.encrypted_parts();
    if bytes.len() > MAX_OUTCOME_CIPHERTEXT {
        return Err(refused());
    }
    SelectedHistoryNodePage::decode(history).map_err(|_| refused())?;
    seal_record(
        key,
        OUTCOME_KEY,
        OUTCOME_TAG,
        OUTCOME_WITH_HISTORY,
        context_fields(context, entry),
        &[nonce, bytes, history],
        rng,
    )
}

pub(in crate::store) struct OpenedOutcome {
    pub(in crate::store) sealed: SealedV2JoinedOutcome,
    pub(in crate::store) history: Option<Zeroizing<Vec<u8>>>,
}

pub(in crate::store) fn open_outcome(
    key: &[u8; 32],
    context: V2PendingJoinContextData,
    entry: &[u8],
    raw: &[u8],
) -> Result<OpenedOutcome, AppError> {
    let plain = open_record(key, OUTCOME_KEY, raw, OUTCOME_FRAME)?;
    let (version, mut d) = decoder(&plain, OUTCOME_TAG, context_fields(context, entry))?;
    let nonce: [u8; 24] = d
        .get_bytes()
        .map_err(|_| refused())?
        .try_into()
        .map_err(|_| refused())?;
    let bytes = d.get_bytes().map_err(|_| refused())?;
    if bytes.len() > MAX_OUTCOME_CIPHERTEXT {
        return Err(refused());
    }
    let history = match version {
        VERSION => None,
        OUTCOME_WITH_HISTORY => {
            let page = d.get_bytes().map_err(|_| refused())?;
            SelectedHistoryNodePage::decode(page).map_err(|_| refused())?;
            Some(page)
        }
        _ => return Err(refused()),
    };
    d.finish().map_err(|_| refused())?;
    Ok(OpenedOutcome {
        sealed: SealedV2JoinedOutcome::from_encrypted_parts(nonce, bytes).map_err(|_| refused())?,
        history: history.map(|page| Zeroizing::new(page.to_vec())),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;

    #[test]
    fn governed_joined_pin_welcome_entry_binds_its_delivery_version_and_pending() {
        let mut rng = ChaCha20Rng::seed_from_u64(0x6294);
        let key = [9; 32];
        let context = V2PendingJoinContextData {
            vault: [1; 32],
            root: [2; 32],
            original_pending: [3; 32],
            pin: [4; 32],
        };
        let pending = b"original pending";
        // The outer record classifies the header before the separate signed
        // delivery validator admits any Welcome work.
        for (tag, version, wrong) in [
            (
                b"mewtual/chat/first-join-delivery/v2\0".as_slice(),
                VERSION,
                LATER_ENTRY,
            ),
            (
                b"mewtual/chat/governing-join-delivery/v2\0".as_slice(),
                LATER_ENTRY,
                VERSION,
            ),
        ] {
            let mut e = Encoder::new();
            e.put_bytes(tag).unwrap().put_u16(2);
            let delivery = e.finish();
            let entry = seal_entry(&key, context, pending, &delivery, &mut rng).unwrap();
            assert_eq!(entry_version(&delivery).unwrap(), version);
            assert_eq!(
                &*open_entry(&key, context, pending, &entry).unwrap(),
                &delivery
            );
            assert!(open_entry(&key, context, b"replacement pending", &entry).is_err());
            let relabelled = seal_record(
                &key,
                ENTRY_KEY,
                ENTRY_TAG,
                wrong,
                context_fields(context, pending),
                &[&delivery],
                &mut rng,
            )
            .unwrap();
            assert!(open_entry(&key, context, pending, &relabelled).is_err());
        }
    }

    #[test]
    fn governed_joined_pin_welcome_outcome_reads_legacy_and_requires_the_new_history_page() {
        let mut rng = ChaCha20Rng::seed_from_u64(0x6293);
        let key = [9; 32];
        let context = V2PendingJoinContextData {
            vault: [1; 32],
            root: [2; 32],
            original_pending: [3; 32],
            pin: [4; 32],
        };
        let entry = [5; 80];
        let labels = context_fields(context, &entry);
        let nonce = [6; 24];
        let graph = [7; 4096];
        let mut record = |version, fields: &[&[u8]]| {
            seal_record(
                &key,
                OUTCOME_KEY,
                OUTCOME_TAG,
                version,
                labels,
                fields,
                &mut rng,
            )
            .unwrap()
        };

        let legacy = record(VERSION, &[&nonce, &graph]);
        let opened = open_outcome(&key, context, &entry, &legacy).unwrap();
        assert!(opened.history.is_none());
        assert_eq!(opened.sealed.encrypted_parts(), (&nonce, graph.as_slice()));
        assert!(open_outcome(&key, context, &[8; 80], &legacy).is_err());
        for invalid in [
            record(OUTCOME_WITH_HISTORY, &[&nonce, &graph]),
            record(OUTCOME_WITH_HISTORY, &[&nonce, &graph, b"foreign page"]),
            record(VERSION, &[&nonce, &graph, b"trailing field"]),
            record(3, &[&nonce, &graph]),
        ] {
            assert!(open_outcome(&key, context, &entry, &invalid).is_err());
        }
    }
}
