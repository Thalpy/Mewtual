//! Authenticated first PC intent. This control is still not an SDP data/action endpoint:
//! the browser must use the same bounded native generation in every eventual RTC effect.
use super::bootstrap::Route;
use super::channel::{self, BodyKind, CompletedReply, Incoming, Operation, PhysicalHold};
use super::context::Context;
use super::factory::{self, Refused};
use super::ledger::{PcReceipt, PcState, RowId};
use super::requests::pc::{Kind, Request};
use catcoms_rt::Clock;
use catcoms_sync::voice_v2::VoiceError;
// This pays the actual queued Actor/future/completion, independently of the fixed intent.
// No task or provider has entered when the measured graph is refused.
const WORK_BYTES: usize = 4096;
const SHORT_REPLY: usize = 3;
const IDENTITY_REPLY: usize = 43;
const ADMITTED_REPLY: usize = 85;
fn reply_bytes(receipt: &PcReceipt) -> usize {
    if matches!(receipt.state, PcState::Admitted { .. }) {
        ADMITTED_REPLY
    } else if receipt.proof.is_some() {
        IDENTITY_REPLY
    } else {
        SHORT_REPLY
    }
}
pub(super) async fn apply(
    context: &Context,
    route: &Route,
    incoming: Incoming,
) -> Result<CompletedReply, channel::Refused> {
    let physical = incoming.hold_position()?;
    let result = prepare(context, route, &incoming, &physical).await;
    let mut reply = incoming.reply(
        BodyKind::Control,
        result.as_ref().map_or(3, reply_bytes),
        None,
    )?;
    match result {
        Ok(receipt) => encode_into(receipt, reply.plain_mut()),
        Err(error) => reply
            .plain_mut()
            .copy_from_slice(&super::replies::refused(error)),
    }
    reply.seal()
}
async fn prepare(
    context: &Context,
    route: &Route,
    incoming: &Incoming,
    physical: &PhysicalHold,
) -> Result<PcReceipt, Refused> {
    if !incoming.belongs_to(route.session())
        || incoming.operation() != Operation::PcIntent
        || incoming.kind() != BodyKind::Control
    {
        return Err(VoiceError::Unauthorized.into());
    }
    let request = Request::decode(
        incoming.logical(),
        incoming.plain().map_err(|_| VoiceError::Malformed)?,
    )?;
    let row = {
        let live = context.upgrade().ok_or(Refused::Closed)?;
        live.channels
            .require_cleanup_route(route)
            .map_err(|_| Refused::Closed)?;
        live.ledger
            .resolve_sdp_parent(route.selected(), RowId::from_logical(incoming.logical())?)?
    };
    if matches!(request.kind, Kind::Close) {
        return row
            .close_pc(route.session().identity(), &request)
            .map_err(Into::into);
    }
    if let Some(receipt) = row.pc_early_receipt(route.session().identity(), &request)? {
        return Ok(receipt);
    }
    factory::preflight_binding(context, route).await?;
    let preparation = super::sdp::current(context, route, |admission| {
        row.begin_pc(admission, physical, route.session(), &request)
    })?;
    let clock = {
        let live = context.upgrade().ok_or(Refused::Closed)?;
        let clock = live
            .authority
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .runtime()
            .clock();
        clock
    };
    if let Some(mut preparation) = preparation {
        let work = preparation.phase.copy(WORK_BYTES)?;
        let (actor, grant) = factory::native::observation_actor(context, route)?;
        let future = actor.prepare_voice_initial_sdp_intent(
            grant,
            request.base.handshake,
            preparation.custody.clone(),
            preparation.phase.effect(),
            1,
            work,
        );
        let bytes = std::mem::size_of_val(&future) + catcoms_app::INITIAL_SDP_RPC_BYTES;
        if bytes > WORK_BYTES {
            return Err(VoiceError::Capacity.into());
        }
        let completion = future.await?;
        preparation.observe(completion.result()?);
        let wall = clock.now_ms();
        let mono = clock.monotonic_ms();
        let result = super::sdp::current(context, route, |admission| {
            row.pc_receipt(admission, &request, Some(&mut preparation), wall, mono)
        });
        // Actual Actor result outlives final admission, then native worker finally. A held
        // completion or queued command still blocks Phase drainage after Close/SDK Release.
        drop(completion);
        drop(preparation);
        result
    } else {
        let wall = clock.now_ms();
        let mono = clock.monotonic_ms();
        super::sdp::current(context, route, |admission| {
            row.pc_receipt(admission, &request, None, wall, mono)
        })
    }
}
fn encode_into(receipt: PcReceipt, bytes: &mut [u8]) {
    assert_eq!(bytes.len(), reply_bytes(&receipt));
    let state = match receipt.state {
        PcState::Preparing => 0,
        PcState::Admitted { .. } => 1,
        PcState::Closed => 2,
    };
    bytes[..3].copy_from_slice(&[7, state, u8::from(receipt.proof.is_some())]);
    if let Some(proof) = receipt.proof {
        bytes[3..11].copy_from_slice(&1u64.to_be_bytes());
        bytes[11..43].copy_from_slice(&proof);
    }
    if let PcState::Admitted {
        round,
        low,
        remaining,
    } = receipt.state
    {
        assert!(receipt.proof.is_some() && remaining > 0 && remaining <= 30_000);
        bytes[43] = match round.profile {
            catcoms_sync::voice_v2::SdpProfile::CompleteSdpV1 => 1,
        }; // A new profile must explicitly define its compatible reply grammar.
        bytes[44] = u8::from(low);
        bytes[45..77].copy_from_slice(&round.ready);
        bytes[77..85].copy_from_slice(&remaining.to_be_bytes());
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_pc_reply_vectors_match_browser_without_any_missing_ready_sentinel() {
        let vectors: serde_json::Value =
            serde_json::from_str(include_str!("requests/pc-vectors.json")).unwrap();
        for vector in vectors["replies"].as_array().unwrap() {
            let proof = if vector["name"] == "closed-before-identity" {
                None
            } else {
                Some([22; 32])
            };
            let state = match vector["name"].as_str().unwrap() {
                "preparing" => PcState::Preparing,
                "admitted" => PcState::Admitted {
                    round: catcoms_sync::voice_v2::SdpRound {
                        ready: [0; 32],
                        profile: catcoms_sync::voice_v2::SdpProfile::CompleteSdpV1,
                    },
                    low: true,
                    remaining: 123,
                },
                _ => PcState::Closed,
            };
            let receipt = PcReceipt { proof, state };
            let mut bytes = vec![0; reply_bytes(&receipt)];
            encode_into(receipt, &mut bytes);
            assert_eq!(hex::encode(bytes), vector["body"]);
        }
    }
    #[test]
    fn pc_reply_union_never_turns_missing_ready_or_closed_history_into_admission() {
        for (proof, state, expected) in [
            (None, PcState::Closed, vec![7, 2, 0]),
            (Some([8; 32]), PcState::Preparing, vec![7, 0, 1]),
            (Some([8; 32]), PcState::Closed, vec![7, 2, 1]),
        ] {
            let receipt = PcReceipt { proof, state };
            let mut bytes = vec![0; reply_bytes(&receipt)];
            encode_into(receipt, &mut bytes);
            assert_eq!(&bytes[..3], expected.as_slice());
        }
        let receipt = PcReceipt {
            proof: Some([8; 32]),
            state: PcState::Admitted {
                round: catcoms_sync::voice_v2::SdpRound {
                    ready: [0; 32],
                    profile: catcoms_sync::voice_v2::SdpProfile::CompleteSdpV1,
                },
                low: true,
                remaining: 123,
            },
        };
        let mut bytes = [0; ADMITTED_REPLY];
        encode_into(receipt, &mut bytes);
        assert_eq!(&bytes[..3], &[7, 1, 1]);
        assert_eq!(&bytes[43..45], &[1, 1]);
        assert_eq!(u64::from_be_bytes(bytes[77..85].try_into().unwrap()), 123);
        // All-zero actual ReadyIDs are data, never a substitute for the absent union branch.
        assert_eq!(&bytes[45..77], &[0; 32]);
    }
}
