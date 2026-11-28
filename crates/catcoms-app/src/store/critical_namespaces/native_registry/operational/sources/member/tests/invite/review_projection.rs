//! Exercise review display on a later member that never received the ordinary original.
use super::super::super::super::tests::selected_joined;
use super::{command, policy, ActionId, Owner, RequestCancellation};
use crate::store::critical_namespaces::native_registry::preparation::tests::{
    current_policy, SECRET,
};
use crate::store::{
    NativeTextFounderDependencyProvider, NativeTextInvitePacketData,
    NativeTextJoinedMemberProjection, NativeTextJoinedReviewResponse, NativeTextJoinedRowSelection,
    NativeTextJoinedSelectionPreparation, NativeTextJoinedStorage, NativeTextMemberPreparation,
    NativeTextReviewApprovalStatus, NativeTextReviewHeaderStatus, NativeTextSourceRetirementStatus,
    NativeTextVault, NativeUiInvocationBudget,
};
use catcoms_discovery::EndpointDialScheduler;
use catcoms_net::MeshService;
use catcoms_replication::chat::{
    owner_bodies::{ReviewStreamEntry, ReviewStreamQuery},
    GuardedProjection, ProjectionCertainty,
};
use catcoms_rt::{OsCryptoRng, SystemClock, TransportEvent};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use std::{
    net::Ipv4Addr,
    sync::{
        atomic::{AtomicBool, AtomicU64},
        Arc,
    },
    time::Duration,
};

type FounderPreparation = NativeTextMemberPreparation<MeshService, ChaCha20Rng>;
const TOKEN: [u8; 32] = [0xb3; 32];
const POST: &str = "A separate post from the same removed member";

pub(super) struct LaterReceiver {
    directory: tempfile::TempDir,
    budget: NativeUiInvocationBudget,
    rng: ChaCha20Rng,
}

struct MountedReceiver {
    vault: NativeTextVault,
    sources: Owner,
    selection: NativeTextJoinedRowSelection,
}

impl LaterReceiver {
    pub(super) async fn join(
        founder: impl Fn() -> FounderPreparation,
        invite: &NativeTextInvitePacketData<'_>,
        now: u64,
        caller: &RequestCancellation,
        seed: u64,
    ) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let (vault, budget) = selected_joined(directory.path());
        let sources: Owner = vault
            .capture_native_sources(policy().prepare().unwrap())
            .finish()
            .unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(seed);
        let capture = || sources.capture_join_request(command(&vault, &budget));
        capture()
            .cover(
                invite,
                &invite.binding().spec_data().anchor,
                TOKEN,
                now,
                caller,
                &mut rng,
            )
            .unwrap();
        let exchange = capture().prepare_exchange(invite, caller.clone()).unwrap();
        let mut incoming = founder()
            .open_connected_join_requests(caller.clone())
            .unwrap();
        let scheduler = EndpointDialScheduler::new(Default::default());
        exchange
            .connect_approved_host(invite, Ipv4Addr::LOCALHOST.into(), &scheduler)
            .await
            .unwrap();
        let serve = async {
            let Some(TransportEvent::Request {
                proto,
                data,
                responder,
                ..
            }) = incoming.next().await
            else {
                panic!("the registered fourth vault sends its own JoinRequest");
            };
            founder()
                .serve_connected_join_request(proto, &data, responder, caller.clone())
                .unwrap();
        };
        let (delivery, ()) = tokio::time::timeout(Duration::from_secs(15), async {
            tokio::join!(exchange.receive(), serve)
        })
        .await
        .expect("the founder covers a later Add and returns the fourth vault's Welcome");
        let mut delivery = Some(delivery.unwrap());
        let mut inputs = Some((
            rng.clone(),
            Box::new(SystemClock) as Box<dyn catcoms_rt::Clock + Send>,
        ));
        let mut install = sources
            .capture_join_installation(&mut delivery, &mut inputs)
            .unwrap();
        let registered = install.install().unwrap();
        drop(install);
        sources
            .capture_joined_selection(command(&vault, &budget))
            .register(
                invite,
                TOKEN,
                "Review receiver".into(),
                "Fourth member".into(),
                caller,
                &mut rng,
            )
            .unwrap();
        let selection = sources
            .capture_joined_selection(command(&vault, &budget))
            .select_registered(caller)
            .unwrap();
        assert_eq!(selection.id_data(), 1);
        assert_eq!(
            sources
                .capture_joined_selection(command(&vault, &budget))
                .inspect(invite, TOKEN, caller)
                .unwrap(),
            Some(registered)
        );
        MountedReceiver {
            vault,
            sources,
            selection,
        }
        .retire()
        .await;

        let mut receiver = Self {
            directory,
            budget,
            rng,
        };
        let storage = receiver.open();
        assert!(storage
            .select_review_storage_before_joined_startup(&mut receiver.rng)
            .unwrap());
        receiver
    }

    fn open(&mut self) -> NativeTextJoinedStorage {
        NativeTextJoinedStorage::open_for_native_startup(
            self.directory.path(),
            SECRET,
            &mut self.rng,
            current_policy(),
        )
        .unwrap()
    }

    fn mount(&mut self, caller: &RequestCancellation) -> MountedReceiver {
        let storage = self.open();
        assert!(!storage
            .select_review_storage_before_joined_startup(&mut self.rng)
            .unwrap());
        let (vault, sources) = storage
            .bind_recovered_native_runtime(self.rng.clone(), Box::new(SystemClock), caller)
            .map_err(|(_, error)| error)
            .unwrap()
            .into_recovered_native_sources(
                (
                    Arc::new(AtomicU64::new(7)),
                    Arc::new(AtomicU64::new(7)),
                    Arc::new(AtomicBool::new(false)),
                ),
                self.budget.clone(),
                &policy(),
                caller,
            )
            .unwrap();
        let selection = sources
            .capture_joined_selection(command(&vault, &self.budget))
            .select_registered(caller)
            .unwrap();
        MountedReceiver {
            vault,
            sources,
            selection,
        }
    }

    pub(super) async fn check_projection(
        mut self,
        founder: impl Fn() -> FounderPreparation,
        provider: &mut NativeTextFounderDependencyProvider<ChaCha20Rng>,
        invite: &NativeTextInvitePacketData<'_>,
        (entry, action, channel): (&ReviewStreamEntry, ActionId, u128),
        caller: &RequestCancellation,
    ) {
        let mounted = self.mount(caller);
        let row = || mounted.prepare(&self.budget);
        let selection = &mounted.selection;
        let scheduler = EndpointDialScheduler::new(Default::default());
        row()
            .prepare_reconnect(
                selection,
                invite,
                TOKEN,
                Ipv4Addr::LOCALHOST.into(),
                caller.clone(),
            )
            .unwrap()
            .connect(&scheduler)
            .await
            .unwrap();
        assert_eq!(
            row().selected_archive_target_data(selection, channel, action, caller),
            Ok(None)
        );
        assert!(matches!(
            row().received_archive_lookup(
                selection,
                channel,
                action,
                caller.clone(),
                &mut self.rng
            ),
            Ok((true, None))
        ));

        let contact = row().sign_member_contact(selection, caller).unwrap();
        let mut wire_rng = OsCryptoRng;
        let read = async |query,
                          provider: &mut NativeTextFounderDependencyProvider<ChaCha20Rng>,
                          rng: &mut OsCryptoRng| {
            let exchange = row()
                .prepare_review_delivery(selection, invite, (&contact, query), caller.clone(), rng)
                .unwrap();
            let serve = async {
                let incoming = provider
                    .next()
                    .await
                    .expect("a real request from the fourth member");
                founder()
                    .serve_founder_dependency_request(provider, incoming)
                    .unwrap();
            };
            let (response, ()) = tokio::time::timeout(Duration::from_secs(12), async {
                tokio::join!(exchange.receive(), serve)
            })
            .await
            .unwrap();
            response.unwrap()
        };
        let sequence = entry.position_data().sequence;
        assert_eq!(sequence, 4);
        for ordinal in 1..=sequence {
            let response = read(ReviewStreamQuery::Header(ordinal), provider, &mut wire_rng).await;
            assert_eq!(
                row()
                    .select_review_delivery_header(selection, response, caller, &mut self.rng)
                    .unwrap(),
                Some(NativeTextReviewHeaderStatus::Covered)
            );
        }
        let approval = |channel| ReviewStreamQuery::Approval {
            sequence,
            entry: entry.id(),
            channel,
        };
        let settle = |response: NativeTextJoinedReviewResponse, rng: &mut ChaCha20Rng| {
            row()
                .select_review_delivery_approval(selection, response, caller, rng)
                .unwrap()
        };
        let wrong = read(approval(channel ^ 1), provider, &mut wire_rng).await;
        assert_eq!(settle(wrong, &mut self.rng), None);
        let exact = read(approval(channel), provider, &mut wire_rng).await;
        assert_eq!(
            settle(exact, &mut self.rng),
            Some(NativeTextReviewApprovalStatus::Covered(action))
        );
        assert_eq!(
            row().selected_archive_target_data(selection, channel, action, caller),
            Ok(None)
        );

        let (cancel, negative) = tokio::sync::watch::channel(false);
        let original = RequestCancellation::new(negative, None);
        let projection = row()
            .archived_message_projection(selection, channel, action, original, &mut self.rng)
            .unwrap()
            .expect("an exact ID12 receipt supplies a missing ordinary Create");
        assert_post(&projection, action);
        cancel.send(true).unwrap();
        assert!(projection
            .with_publication(|_| panic!("a cancelled original must not disclose the post"))
            .is_err());
        drop(projection);
        mounted.retire().await;

        // No provider or reconnect is needed to prove the saved original again.
        let mounted = self.mount(caller);
        let row = || mounted.prepare(&self.budget);
        let selection = &mounted.selection;
        assert_eq!(
            row().selected_archive_target_data(selection, channel, action, caller),
            Ok(None)
        );
        let projection = row()
            .archived_message_projection(selection, channel, action, caller.clone(), &mut self.rng)
            .unwrap()
            .expect("the exact approved post survives a second cold reopen");
        assert_post(&projection, action);
        drop(projection);

        let journal = self.directory.path().join("chat-v2-review-received");
        let lane = std::fs::read_dir(&journal)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| path.is_dir())
            .unwrap();
        let receipts = std::fs::read_dir(lane)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "receipt")
            })
            .collect::<Vec<_>>();
        let [receipt] = receipts.as_slice() else {
            panic!("one exact approval receipt was selected");
        };
        let parked = receipt.with_extension("parked");
        std::fs::rename(receipt, &parked).unwrap();
        assert!(row()
            .archived_message_projection(selection, channel, action, caller.clone(), &mut self.rng)
            .is_err());
        assert!(!receipt.exists());
        assert_eq!(
            row().selected_archive_target_data(selection, channel, action, caller),
            Ok(None)
        );
        std::fs::rename(parked, receipt).unwrap();
        mounted.retire().await;
    }
}

impl MountedReceiver {
    fn prepare(
        &self,
        budget: &NativeUiInvocationBudget,
    ) -> NativeTextJoinedSelectionPreparation<ChaCha20Rng> {
        self.sources
            .capture_joined_selection(command(&self.vault, budget))
    }

    async fn retire(self) {
        let Self {
            vault,
            sources,
            selection,
        } = self;
        drop(selection);
        let mut retirement = sources.prepare_retirement().unwrap();
        tokio::time::timeout(Duration::from_secs(8), async {
            while retirement.try_retire().unwrap() == NativeTextSourceRetirementStatus::Pending {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the original joined worker releases its vault before reopen");
        drop((retirement, sources, vault));
    }
}

fn assert_post(projection: &NativeTextJoinedMemberProjection, action: ActionId) {
    assert!(projection.with_publication(|view| matches!(view,
        GuardedProjection::Visible { create, selected, text, reply_to: None, certainty: ProjectionCertainty::Proven, .. }
            if *create == action && *selected == action && text == POST
    )).unwrap());
}
