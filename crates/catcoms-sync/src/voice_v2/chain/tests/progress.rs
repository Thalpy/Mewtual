//! Native Source budget composition. These exercise the real phase/Actor admission boundaries;
//! signed packet dispatch/ACK verification and WebView execution remain separate integration gates.
use super::*;
use crate::voice_v2::negotiation::Stage;
use crate::voice_v2::runtime::{Runtime, View};
use crate::voice_v2::source::SourceProgress;

#[test]
fn original_source_progress_and_ack_admission_survive_full_ordinary_pool_and_busy_crypto() {
    let runtime = Runtime::new(Arc::new(ManualClock::new(1000)));
    let bound = binding(8);
    let ticket = runtime.session_ticket();
    runtime.activate_session(&ticket).unwrap();
    let (mut actor, registration) = runtime
        .prepare_actor(bound.scope.group, bound.local, &ticket)
        .unwrap();
    registration.register().unwrap();
    actor
        .sync_view(View {
            epoch: bound.epoch,
            members: [bound.local, bound.peer].into(),
            policy: bound.scope.policy,
            eligible: true,
            indirect: true,
        })
        .unwrap();
    let pool = runtime.resources();
    // Real native pair admission prepays both rows before admitting its phase. This raw
    // Workflow fixture reaches the same precondition without fabricating network authority.
    actor
        .with_limits(|limits| {
            limits.prepare_exchange_members(&[bound.local, bound.peer], &pool, 1000)
        })
        .unwrap();
    let infrastructure = pool.counts();
    let chain = prepare_chain(
        pool.clone(),
        bound.clone(),
        Arc::new(slots::Slots::default()),
    )
    .unwrap();
    let workflow = chain
        .control
        .lock()
        .unwrap()
        .workflow
        .as_ref()
        .unwrap()
        .clone();
    let progress = SourceProgress::Workflow(&workflow);
    progress
        .require_current(&pool, bound.scope.group, None)
        .unwrap();
    let fill = pool
        .charge(
            bound.scope.group,
            crate::voice_v2::resources::GROUP_BYTES - pool.counts().0,
            ResourceKind::Retained,
        )
        .unwrap();
    let before = pool.counts();
    assert!(pool
        .charge(bound.scope.group, 1, ResourceKind::Retained)
        .is_err());
    let source = progress
        .charge(&pool, bound.scope.group, 4096, Stage::Source)
        .unwrap();
    let lower = Arc::new(
        progress
            .charge(&pool, bound.scope.group, 8192, Stage::Lower)
            .unwrap(),
    );
    let second = progress
        .charge(&pool, bound.scope.group, 8192, Stage::Lower)
        .unwrap();
    let sdk = progress
        .charge(&pool, bound.scope.group, 8192, Stage::Crypto)
        .unwrap();
    // Repeated local lane refusal must not debit requester/process verification limits. The
    // SAME retained completion is eligible once actual SDK custody returns; no attempt reset.
    for _ in 0..256 {
        assert_eq!(
            actor
                .admit_transport_verification_in_progress(
                    bound.peer,
                    &[1; 128],
                    Some(progress),
                    true
                )
                .err(),
            Some(VoiceError::Capacity)
        );
    }
    assert_eq!(pool.counts(), before);
    drop(sdk);
    let ack = actor
        .admit_transport_verification_in_progress(bound.peer, &[1; 128], Some(progress), true)
        .unwrap();
    drop(ack);
    let control = workflow.confirmation_control();
    chain.close();
    let retirement = control.retire();
    drop((second, fill, workflow, chain));
    assert!(
        !retirement.is_drained(),
        "Source record and real Lower still own this Workflow"
    );
    let last = lower.clone();
    drop((source, lower));
    assert!(
        !retirement.is_drained(),
        "last transport clone is not a logical cancellation"
    );
    drop(last);
    assert!(retirement.is_drained());
    drop((retirement, actor, runtime));
    // Native actor/origin infrastructure has independent owners (including registration),
    // and must remain paid after this test's variable Workflow/transport owners drain.
    assert_eq!(pool.counts(), infrastructure);
}

#[test]
fn progress_never_adopts_an_equal_foreign_pool_or_retired_workflow_and_never_falls_back() {
    let bound = binding(8);
    let pool = Arc::new(Resources::default());
    let foreign = Arc::new(Resources::default());
    let chain = prepare_chain(
        pool.clone(),
        bound.clone(),
        Arc::new(slots::Slots::default()),
    )
    .unwrap();
    let other = prepare_chain(
        foreign.clone(),
        bound.clone(),
        Arc::new(slots::Slots::default()),
    )
    .unwrap();
    let workflow = chain
        .control
        .lock()
        .unwrap()
        .workflow
        .as_ref()
        .unwrap()
        .clone();
    let wrong = other
        .control
        .lock()
        .unwrap()
        .workflow
        .as_ref()
        .unwrap()
        .clone();
    let before = (pool.counts(), foreign.counts());
    assert_eq!(
        SourceProgress::Workflow(&wrong).require_current(&pool, bound.scope.group, None),
        Err(VoiceError::Unauthorized)
    );
    assert_eq!(
        SourceProgress::Workflow(&workflow).require_current(&pool, [99; 32], None),
        Err(VoiceError::Unauthorized)
    );
    assert_eq!(
        SourceProgress::Workflow(&wrong)
            .charge(&pool, bound.scope.group, 1, Stage::Source)
            .err(),
        Some(VoiceError::Scope)
    );
    assert_eq!((pool.counts(), foreign.counts()), before);
    chain.close();
    let before = pool.counts();
    assert_eq!(
        SourceProgress::Workflow(&workflow).require_current(&pool, bound.scope.group, None),
        Err(VoiceError::Unauthorized)
    );
    assert_eq!(
        SourceProgress::Workflow(&workflow)
            .charge(&pool, bound.scope.group, 1, Stage::Source)
            .err(),
        Some(VoiceError::Unauthorized)
    );
    assert_eq!(
        pool.counts(),
        before,
        "ordinary free capacity cannot replace the retired source's owner"
    );
    drop((workflow, chain));
    let replacement =
        prepare_chain(pool.clone(), bound, Arc::new(slots::Slots::default())).unwrap();
    replacement.close();
    other.close();
    drop((replacement, other, wrong));
    assert_eq!(pool.counts(), (0, 0));
    assert_eq!(foreign.counts(), (0, 0));
}
