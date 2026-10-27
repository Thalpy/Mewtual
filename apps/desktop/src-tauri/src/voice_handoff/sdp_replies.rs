//! Distinct version8 history. Confirmation's v1 codec cannot represent an SDP Pending
//! that already published an Action, or a genuinely collected negative with no Action.
//! This metadata grants no engine, input, packet, SDK or RTC effect authority.
use super::ledger::{PendingStatus, RowId, SdpDeliveryState, SdpHistory, Status};
use super::requests::{error_code, receipt_digest};
use catcoms_sync::voice_v2::chain::sdp_handoff::NativeRowPurpose;
use catcoms_sync::voice_v2::crypto_actions::CryptoOutcome;
use catcoms_sync::voice_v2::VoiceError;

pub(super) const HISTORY_BYTES: usize = 220;
pub(super) const MISSING_BYTES: usize = 20;
/// Distinct version8/tag4 permit, bound to the authenticated original SDP request.
/// Reuse only the fixed field encoder; never expose confirmation's version1 grammar.
pub(super) fn admitted(
    row: RowId,
    action: super::requests::Action,
    engine_bytes: usize,
    permit: u64,
) -> Result<[u8; super::replies::PERMIT_BYTES], VoiceError> {
    let mut bytes = super::replies::admitted(row, action, engine_bytes, permit)?;
    bytes[..2].copy_from_slice(&[8, 4]);
    Ok(bytes)
}
/// Missing is a snapshot miss. Only successful native close_sdp_original may set closed;
/// even then this records a purpose fence, never engine/SDK/storage or whole-PC drainage.
pub(super) fn missing(
    parent: RowId,
    purpose: NativeRowPurpose,
    closed: bool,
) -> [u8; MISSING_BYTES] {
    let mut bytes = [0; MISSING_BYTES];
    bytes[..2].copy_from_slice(&[8, 1]);
    bytes[2..18].copy_from_slice(&parent.logical());
    bytes[18] = u8::from(purpose == NativeRowPurpose::Open);
    bytes[19] = u8::from(closed);
    bytes
}
/// Version8 has no legacy decoder fallback, including its bounded refusal variants.
pub(super) fn refused(error: super::factory::Refused) -> [u8; 4] {
    use super::factory::Refused;
    match error {
        Refused::Closed => [8, 2, 0, 0],
        Refused::Stopped => [8, 2, 1, 0],
        Refused::Protocol(error) => [8, 2, 2, error_code(error)],
    }
}

/// Only the exact Session-authenticated, paid native snapshot reaches this producer.
/// The separate HTTP Control envelope pays the stack array and ciphertext until collection.
pub(super) fn history(value: &SdpHistory) -> Result<[u8; HISTORY_BYTES], VoiceError> {
    encode(
        value.row(),
        value.parent(),
        value.purpose(),
        value.status(),
        value.delivery_state(),
    )
}
fn encode(
    row: RowId,
    parent: RowId,
    purpose: NativeRowPurpose,
    status: &Status,
    delivery: SdpDeliveryState,
) -> Result<[u8; HISTORY_BYTES], VoiceError> {
    RowId::from_logical(row.logical())?;
    RowId::from_logical(parent.logical())?;
    if row == parent {
        return Err(VoiceError::Malformed);
    }
    validate(purpose, status, delivery)?;
    let mut bytes = [0; HISTORY_BYTES];
    bytes[..2].copy_from_slice(&[8, 0]);
    bytes[2..18].copy_from_slice(&row.logical());
    bytes[18..34].copy_from_slice(&parent.logical());
    bytes[34] = match purpose {
        NativeRowPurpose::Seal => 0,
        NativeRowPurpose::Open => 1,
    };
    bytes[35] = match status.pending {
        PendingStatus::NotPending => 0,
        PendingStatus::Open => 1,
        PendingStatus::Closing => 2,
        PendingStatus::Closed {
            actor_handoff: false,
        } => 3,
        PendingStatus::Closed {
            actor_handoff: true,
        } => 4,
    };
    bytes[36] = u8::from(status.drained);
    if let Some((serial, version, input)) = status.action {
        bytes[37] = 1;
        bytes[38..46].copy_from_slice(&serial.to_be_bytes());
        bytes[46..54].copy_from_slice(&version.to_be_bytes());
        bytes[54..86].copy_from_slice(&input);
    }
    bytes[86] = u8::from(status.engine.settled);
    if let Some(receipt) = &status.engine.receipt {
        bytes[87] = 1;
        bytes[88..120].copy_from_slice(&receipt.output_tag);
        match receipt.outcome {
            CryptoOutcome::Sealed { message, hash } => {
                bytes[120] = 1;
                bytes[121..153].copy_from_slice(&message);
                bytes[153..185].copy_from_slice(&hash);
            }
            CryptoOutcome::Opened => bytes[120] = 2,
            CryptoOutcome::Rejected(error) => {
                bytes[120] = 3;
                bytes[121] = error_code(error);
            }
            CryptoOutcome::Derived => unreachable!("closed SDP validation"),
        }
        bytes[185] = u8::from(receipt.retired);
        bytes[187..219].copy_from_slice(&receipt_digest(receipt));
    }
    bytes[186] = u8::from(status.acknowledged);
    bytes[219] = match delivery {
        SdpDeliveryState::None => 0,
        SdpDeliveryState::Prepared => 1,
        SdpDeliveryState::Admitted => 2,
        SdpDeliveryState::Applied => 3,
    };
    Ok(bytes)
}
fn validate(
    purpose: NativeRowPurpose,
    status: &Status,
    delivery: SdpDeliveryState,
) -> Result<(), VoiceError> {
    let Some((serial, version, input)) = status.action else {
        // No-action collection is separate from engine settlement. Only native's canonical
        // closed outcome + zero IO/worker/observer gate can construct a drained snapshot.
        if status.pending == PendingStatus::NotPending
            || status.engine.settled
            || status.engine.receipt.is_some()
            || status.acknowledged
            || delivery != SdpDeliveryState::None
            || (status.drained && !matches!(status.pending, PendingStatus::Closed { .. }))
        {
            return Err(VoiceError::Malformed);
        }
        return Ok(());
    };
    if serial == 0 || version == 0 || (status.drained && !status.engine.settled) {
        return Err(VoiceError::Malformed);
    }
    let Some(receipt) = &status.engine.receipt else {
        return if status.acknowledged || delivery != SdpDeliveryState::None {
            Err(VoiceError::Malformed)
        } else {
            Ok(())
        };
    };
    if receipt.action != serial
        || receipt.input_tag != input
        || (status.drained && (!status.acknowledged || !receipt.retired))
    {
        return Err(VoiceError::Malformed);
    }
    match (purpose, receipt.outcome, delivery) {
        (NativeRowPurpose::Seal, CryptoOutcome::Sealed { .. }, SdpDeliveryState::None)
        | (_, CryptoOutcome::Rejected(_), SdpDeliveryState::None) => Ok(()),
        (NativeRowPurpose::Open, CryptoOutcome::Opened, value)
            if value != SdpDeliveryState::None =>
        {
            Ok(())
        }
        _ => Err(VoiceError::Malformed),
    }
}

/// Synchronous reply encoder and bounded ciphertext backing, in addition to its caller.
/// The original history observation has returned before this receipt digest is hashed.
#[cfg(test)]
pub(super) fn serialization_layout_bytes() -> usize {
    std::mem::size_of::<SdpHistory>()
        + HISTORY_BYTES
        + std::mem::size_of::<sha2::Sha256>()
        + std::mem::size_of::<[u8; 32]>()
        + 2 * std::mem::size_of::<RowId>()
        + std::mem::size_of::<NativeRowPurpose>()
        + std::mem::size_of::<&Status>()
        + 96 // arguments, discriminants and fixed scalar temporaries
        + HISTORY_BYTES + 16 // independent encrypted reply allocation
}

#[cfg(test)]
mod tests {
    #[test]
    fn engine_permit_matches_the_shared_browser_vector_and_refuses_wrong_serial() {
        let vector: serde_json::Value =
            serde_json::from_str(include_str!("sdp-engine-vectors.json")).unwrap();
        let row = super::RowId::from_logical(
            hex::decode(vector["row"].as_str().unwrap())
                .unwrap()
                .try_into()
                .unwrap(),
        )
        .unwrap();
        let action = super::super::requests::Action {
            serial: vector["identity"]["action"]
                .as_str()
                .unwrap()
                .parse()
                .unwrap(),
            version: vector["identity"]["version"]
                .as_str()
                .unwrap()
                .parse()
                .unwrap(),
            input_tag: [7; 32],
        };
        let bytes = vector["engineBytes"].as_u64().unwrap() as usize;
        assert_eq!(
            hex::encode(super::admitted(row, action, bytes, action.serial).unwrap()),
            vector["permit"].as_str().unwrap()
        );
        assert!(super::admitted(row, action, bytes, action.serial - 1).is_err());
    }
    use super::*;
    use catcoms_sync::voice_v2::crypto_actions::{CryptoResultReceipt, CryptoResultStatus};
    fn ids() -> (RowId, RowId) {
        let mut a = [0; 16];
        a[15] = 1;
        let mut b = a;
        b[15] = 2;
        (
            RowId::from_logical(a).unwrap(),
            RowId::from_logical(b).unwrap(),
        )
    }
    fn pending() -> Status {
        Status {
            pending: PendingStatus::Open,
            action: None,
            engine: CryptoResultStatus {
                settled: false,
                receipt: None,
            },
            acknowledged: false,
            drained: false,
        }
    }
    #[test]
    fn pending_publication_and_real_negative_collection_have_distinct_non_fabricated_history() {
        let (row, parent) = ids();
        let mut status = pending();
        let bytes = encode(
            row,
            parent,
            NativeRowPurpose::Open,
            &status,
            SdpDeliveryState::None,
        )
        .unwrap();
        assert_eq!((bytes[35], bytes[36], bytes[37], bytes[86]), (1, 0, 0, 0));
        status.action = Some((u64::MAX, 9_007_199_254_740_993, [7; 32]));
        let bytes = encode(
            row,
            parent,
            NativeRowPurpose::Open,
            &status,
            SdpDeliveryState::None,
        )
        .unwrap();
        assert_eq!((bytes[35], bytes[37]), (1, 1));
        assert_eq!(&bytes[38..46], &u64::MAX.to_be_bytes());
        status.action = None;
        status.pending = PendingStatus::Closed {
            actor_handoff: true,
        };
        status.drained = true;
        let bytes = encode(
            row,
            parent,
            NativeRowPurpose::Open,
            &status,
            SdpDeliveryState::None,
        )
        .unwrap();
        assert_eq!((bytes[35], bytes[36], bytes[37], bytes[86]), (4, 1, 0, 0));
        status.engine.settled = true;
        assert!(encode(
            row,
            parent,
            NativeRowPurpose::Open,
            &status,
            SdpDeliveryState::None
        )
        .is_err());
    }
    #[test]
    fn terminal_history_keeps_actual_receipt_digest_and_delivery_state_within_control_ceiling() {
        let (row, parent) = ids();
        for (purpose, outcome, delivery) in [
            (
                NativeRowPurpose::Seal,
                CryptoOutcome::Sealed {
                    message: [8; 32],
                    hash: [9; 32],
                },
                SdpDeliveryState::None,
            ),
            (
                NativeRowPurpose::Open,
                CryptoOutcome::Opened,
                SdpDeliveryState::Prepared,
            ),
            (
                NativeRowPurpose::Open,
                CryptoOutcome::Opened,
                SdpDeliveryState::Applied,
            ),
            (
                NativeRowPurpose::Open,
                CryptoOutcome::Rejected(VoiceError::Malformed),
                SdpDeliveryState::None,
            ),
        ] {
            let receipt = CryptoResultReceipt {
                action: u64::MAX,
                input_tag: [7; 32],
                output_tag: [6; 32],
                outcome,
                retired: true,
            };
            let mut status = pending();
            status.pending = PendingStatus::NotPending;
            status.action = Some((u64::MAX, 1, [7; 32]));
            status.engine = CryptoResultStatus {
                settled: true,
                receipt: Some(receipt.clone()),
            };
            status.acknowledged = true;
            status.drained = true;
            let bytes = encode(row, parent, purpose, &status, delivery).unwrap();
            assert!(bytes.len() <= 256);
            assert_eq!(&bytes[187..219], &receipt_digest(&receipt));
            status.engine.receipt.as_mut().unwrap().retired = false;
            assert!(encode(row, parent, purpose, &status, delivery).is_err());
        }
    }
    #[test]
    fn wrong_purpose_receipt_and_invented_delivery_or_ack_refuse() {
        let (row, parent) = ids();
        let mut status = pending();
        assert!(encode(
            row,
            parent,
            NativeRowPurpose::Open,
            &status,
            SdpDeliveryState::Applied
        )
        .is_err());
        status.action = Some((1, 1, [7; 32]));
        status.engine.receipt = Some(CryptoResultReceipt {
            action: 1,
            input_tag: [7; 32],
            output_tag: [6; 32],
            outcome: CryptoOutcome::Opened,
            retired: false,
        });
        for (purpose, delivery) in [
            (NativeRowPurpose::Seal, SdpDeliveryState::Prepared),
            (NativeRowPurpose::Open, SdpDeliveryState::None),
        ] {
            assert!(encode(row, parent, purpose, &status, delivery).is_err());
        }
        status.engine.receipt.as_mut().unwrap().input_tag = [0; 32];
        assert!(encode(
            row,
            parent,
            NativeRowPurpose::Open,
            &status,
            SdpDeliveryState::Prepared
        )
        .is_err());
        assert!(encode(
            row,
            row,
            NativeRowPurpose::Open,
            &status,
            SdpDeliveryState::Prepared
        )
        .is_err());
    }
    #[test]
    fn synchronous_metadata_and_reply_fit_original_control_payment() {
        // The enclosing service/HTTP future must be added independently when installed.
        let encode = std::mem::size_of::<SdpHistory>()
            + HISTORY_BYTES
            + std::mem::size_of::<sha2::Sha256>()
            + std::mem::size_of::<[u8; 32]>()
            + 2 * std::mem::size_of::<RowId>()
            + std::mem::size_of::<NativeRowPurpose>()
            + std::mem::size_of::<&Status>()
            + 96; // arguments, discriminants and fixed scalar temps
                  // Root observation/reconciliation finishes before the codec hashes a receipt.
                  // Count the retained output array even in the earlier observation stage, and an
                  // independent bounded ciphertext backing. The future HTTP producer must add its
                  // own complete outer future/IO inventory rather than treating this as that proof.
        let bytes = (super::super::ledger::sdp_history_layout_bytes() + HISTORY_BYTES).max(encode)
            + HISTORY_BYTES
            + 16;
        assert!(
            bytes <= 4096,
            "complete synchronous history/codec graph needs {bytes}"
        );
    }
    #[test]
    fn browser_control_vectors_keep_missing_close_and_refusal_distinct() {
        let vectors: serde_json::Value =
            serde_json::from_str(include_str!("sdp-history-vectors.json")).unwrap();
        let parent = RowId::from_logical(
            hex::decode(vectors["parent"].as_str().unwrap())
                .unwrap()
                .try_into()
                .unwrap(),
        )
        .unwrap();
        let errors = [
            VoiceError::Malformed,
            VoiceError::Bounds,
            VoiceError::Signature,
            VoiceError::Scope,
            VoiceError::Freshness,
            VoiceError::Unauthorized,
            VoiceError::Signing,
            VoiceError::Capacity,
            VoiceError::Replay,
            VoiceError::Reservation,
            VoiceError::RetryMismatch,
            VoiceError::RateLimited,
        ];
        let mut protocol = 0;
        for item in vectors["controls"].as_array().unwrap() {
            let expected = hex::decode(item["body"].as_str().unwrap()).unwrap();
            if item["kind"] == "missing" {
                let purpose = if item["purpose"] == "seal" {
                    NativeRowPurpose::Seal
                } else {
                    NativeRowPurpose::Open
                };
                assert_eq!(
                    missing(parent, purpose, item["close"].as_bool().unwrap()).as_slice(),
                    expected
                );
            } else {
                let error = match item["code"].as_str().unwrap() {
                    "closed" => super::super::factory::Refused::Closed,
                    "stopped" => super::super::factory::Refused::Stopped,
                    _ => {
                        let error = errors[protocol];
                        protocol += 1;
                        error.into()
                    }
                };
                assert_eq!(refused(error).as_slice(), expected);
            }
        }
        assert_eq!(protocol, errors.len());
    }
    #[test]
    fn browser_vectors_match_actual_native_history_and_receipt_digest() {
        let vectors: serde_json::Value =
            serde_json::from_str(include_str!("sdp-history-vectors.json")).unwrap();
        let row = RowId::from_logical(
            hex::decode(vectors["row"].as_str().unwrap())
                .unwrap()
                .try_into()
                .unwrap(),
        )
        .unwrap();
        let parent = RowId::from_logical(
            hex::decode(vectors["parent"].as_str().unwrap())
                .unwrap()
                .try_into()
                .unwrap(),
        )
        .unwrap();
        let serial = vectors["identity"]["action"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        let version = vectors["identity"]["version"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        for case in vectors["cases"].as_array().unwrap() {
            let purpose = if case["purpose"] == "seal" {
                NativeRowPurpose::Seal
            } else {
                NativeRowPurpose::Open
            };
            let mut status = pending();
            status.pending = match case["pending"].as_u64().unwrap() {
                0 => PendingStatus::NotPending,
                1 => PendingStatus::Open,
                2 => PendingStatus::Closing,
                3 => PendingStatus::Closed {
                    actor_handoff: false,
                },
                4 => PendingStatus::Closed {
                    actor_handoff: true,
                },
                _ => panic!("fixture"),
            };
            if case["action"].as_bool().unwrap() {
                status.action = Some((serial, version, [7; 32]));
            }
            status.engine.settled = case["settled"].as_bool().unwrap();
            if let Some(kind) = case["outcome"].as_str() {
                status.engine.receipt = Some(CryptoResultReceipt {
                    action: serial,
                    input_tag: [7; 32],
                    output_tag: [6; 32],
                    retired: case["retired"].as_bool().unwrap(),
                    outcome: match kind {
                        "sealed" => CryptoOutcome::Sealed {
                            message: [8; 32],
                            hash: [9; 32],
                        },
                        "opened" => CryptoOutcome::Opened,
                        "rejected" => CryptoOutcome::Rejected(VoiceError::Malformed),
                        _ => panic!("fixture"),
                    },
                });
            }
            status.acknowledged = case["acknowledged"].as_bool().unwrap();
            status.drained = case["drained"].as_bool().unwrap();
            let delivery = match case["delivery"].as_u64().unwrap() {
                0 => SdpDeliveryState::None,
                1 => SdpDeliveryState::Prepared,
                2 => SdpDeliveryState::Admitted,
                3 => SdpDeliveryState::Applied,
                _ => panic!("fixture"),
            };
            assert_eq!(
                hex::encode(encode(row, parent, purpose, &status, delivery).unwrap()),
                case["body"].as_str().unwrap(),
                "{}",
                case["name"]
            );
        }
    }
}
