//! Explicit owner repair controls. The renderer echoes candidate hashes only; the actor/store
//! rederive the fault, current tenure and signed decision under the existing custody path.
use super::*;
use catcoms_app::store::StudioRepairOutcome as Outcome;
use catcoms_app::studio::{
    RepairDisposition, RepairHold, StudioControlResponse as Response, StudioFaultCandidate,
    StudioFaultScope, StudioFaultView, StudioRepairBlocker, StudioRepairReport, StudioRepairStart,
};

fn read_action(scope: &str) -> Result<catcoms_app::studio::StudioControlAction, String> {
    use catcoms_app::studio::StudioControlAction as A;
    match scope {
        "source" => Ok(A::ReadFault),
        "registry" => Ok(A::ReadRegistryFault),
        _ => Err("unknown repair scope".into()),
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RepairInput {
    receipt_a: String,
    receipt_b: String,
    selected: String,
}
impl RepairInput {
    fn checked(self) -> Result<catcoms_app::store::StudioRepairRequest, String> {
        let request = catcoms_app::store::StudioRepairRequest {
            receipt_a: recovery::hash(&self.receipt_a)?,
            receipt_b: recovery::hash(&self.receipt_b)?,
            selected: recovery::hash(&self.selected)?,
        };
        if request.receipt_a == request.receipt_b
            || ![request.receipt_a, request.receipt_b].contains(&request.selected)
        {
            return Err("repair must select one of two distinct receipt hashes".into());
        }
        Ok(request)
    }
}

#[tauri::command]
pub(crate) async fn studio_fault_read(
    state: State<'_, AppState>,
    server: u64,
    channel: String,
    object: Option<String>,
    object_kind: Option<String>,
    scope: String,
) -> Result<Value, String> {
    recovery::invoke_control(
        &state,
        server,
        recovery::target_with_kind(&channel, object.as_deref(), object_kind.as_deref())?,
        read_action(&scope)?,
    )
    .await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub(crate) async fn studio_fault_repair(
    state: State<'_, AppState>,
    server: u64,
    channel: String,
    object: Option<String>,
    object_kind: Option<String>,
    scope: String,
    decision: RepairInput,
) -> Result<Value, String> {
    use catcoms_app::studio::StudioControlAction as A;
    let action = match scope.as_str() {
        "source" => A::RepairFault(Box::new(decision.checked()?)),
        "registry" => A::RepairRegistryFault(Box::new(decision.checked()?)),
        _ => return Err("unknown repair scope".into()),
    };
    repair_classified(
        recovery::invoke_control(
            &state,
            server,
            recovery::target_with_kind(&channel, object.as_deref(), object_kind.as_deref())?,
            action,
        )
        .await,
    )
}

fn repair_classified(result: Result<Value, String>) -> Result<Value, String> {
    result.map_err(|error| if error == catcoms_app::studio::CONTROL_REPLY_DROPPED || error.contains(catcoms_app::UNCERTAIN_OUTCOME) {
        format!("outcome=uncertain; repair may have been scheduled; refresh the actual fault status before choosing again ({error})")
    } else { error })
}

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
    "object":match v.target {StudioTarget::Index{..}=>None,StudioTarget::Flipnote{object,..}|StudioTarget::Score{object,..}=>Some(hex::encode(object))},
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
        StudioRepairBlocker::NoFault=>"noFault",StudioRepairBlocker::Scheduled=>"scheduled",
    }),
    // Volatile and local: what this device's last repair job for this fault did, if remembered.
    "lastAttempt":v.last_attempt.as_ref().map(|a|match a {
        StudioRepairReport::Completed(result)=>outcome(*result),
        StudioRepairReport::Failed(error)=>json!({"outcome":"failed","error":error,"terminal":false}),
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
        // A scheduled decision has decided nothing yet; its outcome arrives as the fault view's
        // `lastAttempt`, so the renderer re-reads rather than waiting on this reply.
        Response::RepairStarted {
            target,
            scope: fault_scope,
            start,
        } => json!({"v":1,"kind":"faultRepairStarted","scope":scope(fault_scope),
            "channel":u128::from_be_bytes(target.channel()).to_string(),
            "start":match start {StudioRepairStart::Scheduled=>"scheduled",StudioRepairStart::Busy=>"busy"},
            "refreshRequired":true}),
        _ => return Err("mismatched fault response".into()),
    };
    bounded_view(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repair_input_refuses_extra_authority_and_bad_candidate_pairs() {
        let a = "a".repeat(64);
        let b = "b".repeat(64);
        for value in [
            json!({"receiptA":a,"receiptB":a,"selected":a}),
            json!({"receiptA":a,"receiptB":b,"selected":"c".repeat(64)}),
            json!({"receiptA":a,"receiptB":b,"selected":a,"tenure":"0"}),
        ] {
            assert!(serde_json::from_value::<RepairInput>(value)
                .map_err(|e| e.to_string())
                .and_then(RepairInput::checked)
                .is_err());
        }
        assert!(serde_json::from_value::<RepairInput>(
            json!({"receiptA":a,"receiptB":b,"selected":b})
        )
        .unwrap()
        .checked()
        .is_ok());
        assert!(read_action("other").is_err());
        let error =
            repair_classified(Err(catcoms_app::studio::CONTROL_REPLY_DROPPED.into())).unwrap_err();
        assert!(error.starts_with("outcome=uncertain;"));
        assert_eq!(
            repair_classified(Err("stale pair".into())).unwrap_err(),
            "stale pair"
        );
    }

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

    #[test]
    fn a_scheduled_decision_names_only_whether_it_started() {
        for (start, name) in [
            (StudioRepairStart::Scheduled, "scheduled"),
            (StudioRepairStart::Busy, "busy"),
        ] {
            let encoded = response_value(Response::RepairStarted {
                target: StudioTarget::Index { channel: [1; 16] },
                scope: StudioFaultScope::Source,
                start,
            })
            .unwrap();
            assert_eq!(encoded["kind"], "faultRepairStarted");
            assert_eq!(encoded["start"], name);
            assert_eq!(encoded["refreshRequired"], true);
            // Nothing decided is claimed: no outcome or terminality on this reply.
            assert!(encoded.get("outcome").is_none() && encoded.get("terminal").is_none());
        }
    }
}
