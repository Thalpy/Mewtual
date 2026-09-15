//! Nonvisual event encoding. The event loop calls this only inside the existing final Studio
//! session/incarnation fence; source/recovery reads use the same custody as ordinary Save.
use super::*;
use catcoms_app::studio::StudioSettlementState as S;

pub(crate) fn payload(server: u64, target: StudioTarget, state: S) -> Value {
    let (tag, key, object) = match target {
        StudioTarget::Index { channel } => (15, channel, None),
        StudioTarget::Flipnote { object, .. } => (16, object, Some(hex::encode(object))),
    };
    json!({"server":server,"docType":tag,"logicalKey":hex::encode(key),
    "channel":u128::from_be_bytes(target.channel()).to_string(),"object":object,
    "state":match state {
        S::Open=>"open", S::Closing=>"closing", S::Settled=>"settled", S::Fault=>"fault",
        S::RecoveryAvailable=>"recoveryAvailable", S::RecoveryEvictionPending=>"recoveryEvictionPending",
        S::RefreshRequired=>"refreshRequired",
    }})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_settlement_event_contract_preserves_scope_and_every_implemented_state() {
        let states = [
            (S::Open, "open"),
            (S::Closing, "closing"),
            (S::Settled, "settled"),
            (S::Fault, "fault"),
            (S::RecoveryAvailable, "recoveryAvailable"),
            (S::RecoveryEvictionPending, "recoveryEvictionPending"),
            (S::RefreshRequired, "refreshRequired"),
        ];
        for (state, name) in states {
            let index = payload(7, StudioTarget::Index { channel: [255; 16] }, state);
            assert_eq!(index["state"], name);
            assert_eq!(index["docType"], 15);
            assert_eq!(index["logicalKey"], "ff".repeat(16));
            assert_eq!(index["channel"], u128::MAX.to_string());
            assert!(index["object"].is_null());
            assert!(
                index.get("provisional").is_none(),
                "invalidation is not an edit acknowledgement"
            );
            let art = payload(
                7,
                StudioTarget::Flipnote {
                    channel: [255; 16],
                    object: [3; 16],
                },
                state,
            );
            assert_eq!(art["docType"], 16);
            assert_eq!(art["logicalKey"], "03".repeat(16));
            assert_eq!(art["object"], "03".repeat(16));
            assert_eq!(art["server"], 7);
        }
    }
}
