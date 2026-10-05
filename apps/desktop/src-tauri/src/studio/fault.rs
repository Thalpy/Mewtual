//! The fault half of the one `StudioControlResponse` converter (design 5.8). The two commands,
//! `studio_fault_read` and `studio_fault_repair`, and their security rows are registered in a
//! separate commit owned by Agent 4; until then nothing here is reachable from the renderer.
use super::*;
use catcoms_app::store::StudioRepairOutcome as Outcome;
use catcoms_app::studio::{
    RepairDisposition, RepairHold, StudioControlResponse as Response, StudioFaultCandidate,
    StudioFaultScope, StudioFaultView, StudioRepairBlocker,
};

fn scope(v: StudioFaultScope) -> Value {
    match v {
        StudioFaultScope::Source => json!({"kind":"source"}),
        StudioFaultScope::RegistryBucket(bucket) => {
            json!({"kind":"registryBucket","bucket":bucket})
        }
    }
}

fn candidate(v: &StudioFaultCandidate) -> Value {
    json!({"receipt":hex::encode(v.receipt_hash),"closedEpoch":v.closed_epoch.to_string(),
    "closeRecord":hex::encode(v.close_record_hash),"seedChange":hex::encode(v.seed_change_hash),
    "inheritedEpoch":v.inherited_epoch.map(|e|e.to_string()),"locallyInstalled":v.locally_installed})
}

fn disposition(v: RepairDisposition) -> &'static str {
    match v {
        RepairDisposition::Transitioned => "transitioned",
        RepairDisposition::Retargeted => "retargeted",
        RepairDisposition::Screened => "screened",
    }
}

fn outcome(v: Outcome) -> Value {
    let (name, hold) = match v {
        Outcome::Repaired => ("repaired", None),
        Outcome::Screened => ("screened", None),
        Outcome::Installed => ("installed", None),
        Outcome::AwaitingSeed => ("awaitingSeed", None),
        Outcome::AlreadyRepaired => ("alreadyRepaired", None),
        Outcome::RecoveryPending => ("recoveryPending", None),
        Outcome::StorageRefused => ("storageRefused", None),
        Outcome::Held(hold) => (
            "held",
            Some(match hold {
                RepairHold::SequenceNotNewer => "sequenceNotNewer",
                RepairHold::RepairInProgress => "repairInProgress",
                RepairHold::Settled => "settled",
                RepairHold::UnsupportedShape => "unsupportedShape",
            }),
        ),
    };
    json!({"outcome":name,"hold":hold,"terminal":v.is_terminal()})
}

fn view(v: &StudioFaultView) -> Value {
    json!({"v":1,"kind":"fault","scope":scope(v.scope),"channel":u128::from_be_bytes(v.target.channel()).to_string(),
    "object":match v.target {StudioTarget::Index{..}=>None,StudioTarget::Flipnote{object,..}=>Some(hex::encode(object))},
    "source":json!({"epochId":format!("{:032x}",v.source.epoch_id),"epoch":v.source.epoch.to_string(),
        "phase":match v.source.phase {EpochPhase::Open=>"open",EpochPhase::Closing=>"closing",EpochPhase::Settled=>"settled",EpochPhase::Fault=>"fault"}}),
    "candidates":v.candidates.as_ref().map(|c|c.iter().map(candidate).collect::<Vec<_>>()),
    "repair":v.repair.as_ref().map(|r|json!({"repair":hex::encode(r.repair_hash),
        "selected":hex::encode(r.selected),"sequence":r.sequence.to_string(),"held":r.held,
        "disposition":r.disposition.map(disposition),"installPending":r.install_pending,"installed":r.installed})),
    "mayDecide":v.may_decide,
    "blockedBy":v.blocked_by.map(|b|match b {
        StudioRepairBlocker::NotOwner=>"notOwner",StudioRepairBlocker::TenureUnverified=>"tenureUnverified",
        StudioRepairBlocker::TenureUnobserved=>"tenureUnobserved",StudioRepairBlocker::HeldRepair=>"heldRepair",
        StudioRepairBlocker::NoFault=>"noFault",
    }),
    "waiting":v.waiting,"preservedOperations":v.preserved_operations})
}

pub(super) fn response_value(response: Response) -> Result<Value, String> {
    let value = match response {
        Response::Fault(v) => view(&v),
        Response::Repaired {
            target,
            scope: fault_scope,
            outcome: result,
        } => {
            let mut value = outcome(result);
            value["v"] = 1.into();
            value["kind"] = "faultRepair".into();
            value["scope"] = scope(fault_scope);
            value["channel"] = u128::from_be_bytes(target.channel()).to_string().into();
            // The renderer re-reads; a repair outcome is never inferred into a phase label.
            value["refreshRequired"] = true.into();
            value
        }
        _ => return Err("mismatched fault response".into()),
    };
    bounded_view(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_repair_outcome_has_one_stable_name_and_terminality() {
        let cases = [
            (Outcome::Repaired, "repaired", true),
            (Outcome::Screened, "screened", true),
            (Outcome::Installed, "installed", true),
            (Outcome::AlreadyRepaired, "alreadyRepaired", true),
            (Outcome::AwaitingSeed, "awaitingSeed", false),
            (Outcome::RecoveryPending, "recoveryPending", false),
            (Outcome::StorageRefused, "storageRefused", false),
            (Outcome::Held(RepairHold::RepairInProgress), "held", false),
        ];
        for (value, name, terminal) in cases {
            let encoded = response_value(Response::Repaired {
                target: StudioTarget::Index { channel: [1; 16] },
                scope: StudioFaultScope::Source,
                outcome: value,
            })
            .unwrap();
            assert_eq!(encoded["kind"], "faultRepair");
            assert_eq!(encoded["scope"]["kind"], "source");
            assert_eq!(encoded["outcome"], name);
            assert_eq!(encoded["terminal"], terminal);
            assert_eq!(encoded["refreshRequired"], true);
        }
        let held = response_value(Response::Repaired {
            target: StudioTarget::Index { channel: [1; 16] },
            scope: StudioFaultScope::RegistryBucket(9),
            outcome: Outcome::Held(RepairHold::SequenceNotNewer),
        })
        .unwrap();
        assert_eq!(held["hold"], "sequenceNotNewer");
        // A bucket decision is never presented as the source's own.
        assert_eq!(held["scope"]["kind"], "registryBucket");
        assert_eq!(held["scope"]["bucket"], 9);
    }
}
