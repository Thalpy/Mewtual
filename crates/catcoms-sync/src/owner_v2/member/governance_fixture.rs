//! Test-only HOST trust selection over REAL signed genesis records and the REAL production
//! `V2GeneralPrefixVerifier`. The expectation is an explicit test trust decision (the App
//! host derives it from the authenticated original pin); no witness/roster/fixture-authority
//! constructor issues anything. Every Plan below comes from the production issuer.
use catcoms_replication::chat::acceptance_tree::TreeContext;
use catcoms_replication::chat::owner_header::{HeaderKind, HeaderSpec, SignedOwnerHeader};
use catcoms_replication::chat::owner_profile::{
    AdmissionBody, AdmissionOrigin, AdmissionSpec, ProfileSpec, SignedOwnerProfile,
};
use catcoms_replication::chat::owner_state::{
    encode_state, virtual_genesis_state, CurrentAdmissionSlot, StateFrame, StateMeta,
};
use catcoms_replication::chat::{
    ActionDraft, ActionSpec, Body, ChatLedger, FounderGenesisAnchorData, LedgerLimits,
    ProjectionGroup, ProjectionGroupView, ProjectionView, Realm, V2AuthenticatedAdmission,
    V2GeneralExpectation, V2GeneralPrefixVerifier, V2OriginNegative, V2ProjectionOrigin,
    MAX_ACTION_BYTES,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub(crate) const CHANNEL: u128 = 42;

#[derive(Debug)]
pub(crate) struct Live(pub(crate) AtomicBool);
impl V2OriginNegative for Live {
    fn is_live(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

/// Field order matters for Drop: the channel view and ledger close before the verifier,
/// whose Drop closes the group view; the group owner drops last.
pub(crate) struct Governance {
    pub(crate) view: ProjectionView,
    pub(crate) _ledger: ChatLedger,
    pub(crate) verifier: V2GeneralPrefixVerifier,
    pub(crate) founder: V2AuthenticatedAdmission,
    pub(crate) live: Arc<Live>,
    pub(crate) _group_view: ProjectionGroupView,
    pub(crate) _group: ProjectionGroup,
    pub(crate) group_id: Vec<u8>,
}

/// Genuine founder genesis signed by `sign` for `founder` (the device's full key).
pub(crate) fn signed_genesis(
    group: &[u8],
    founder: [u8; 32],
    sign: impl Fn(&[u8]) -> [u8; 64],
) -> (
    SignedOwnerProfile,
    AdmissionBody,
    SignedOwnerHeader,
    Vec<u8>,
) {
    let creation = *blake3::hash(group).as_bytes();
    let profile = SignedOwnerProfile::sign_using(
        ProfileSpec {
            group: group.to_vec(),
            founder,
            creation,
        },
        |bytes| Ok(sign(bytes)),
    )
    .unwrap();
    let admission = AdmissionBody::new(AdmissionSpec {
        group: group.to_vec(),
        profile: profile.id(),
        subject: founder,
        origin: AdmissionOrigin::Founder { creation },
    })
    .unwrap();
    let context = TreeContext {
        profile: profile.id(),
        admission: admission.admission_id(),
    };
    let image = encode_state(
        StateMeta {
            group,
            profile: profile.id(),
            issuer: founder,
            tenure: profile.tenure(),
            parent: profile.genesis_parent(),
            previous: virtual_genesis_state(&profile).unwrap(),
            sequence: 1,
            kind: HeaderKind::Admission,
            body: admission.id(),
            mls_epoch: 0,
        },
        &[CurrentAdmissionSlot {
            admission: admission.id(),
            subject: founder,
            tip_body: admission.id(),
            accepted_count: 0,
            accepted_root: context.empty_hash(),
        }],
    )
    .unwrap();
    let state = StateFrame::decode(&image).unwrap().commitment();
    let header = SignedOwnerHeader::sign_using(
        HeaderSpec {
            group: group.to_vec(),
            profile: profile.id(),
            issuer: founder,
            tenure: profile.tenure(),
            parent: profile.genesis_parent(),
            sequence: 1,
            kind: HeaderKind::Admission,
            body: admission.id(),
            state,
        },
        |bytes| Ok(sign(bytes)),
    )
    .unwrap();
    (profile, admission, header, image)
}

/// The production general verifier over exact genesis records, plus one live channel view.
pub(crate) fn governance(
    profile: &SignedOwnerProfile,
    admission: &AdmissionBody,
    header: &SignedOwnerHeader,
    image: &[u8],
) -> Governance {
    let group_id = profile.spec().group.to_vec();
    let expectation = V2GeneralExpectation::from_trusted_host(FounderGenesisAnchorData {
        group: group_id.clone(),
        founder: profile.spec().founder,
        profile: profile.id(),
        admission: admission.admission_id(),
        tenure: profile.tenure(),
        header: header.id(),
        state: header.spec().state,
    })
    .unwrap();
    let mut group = ProjectionGroup::new(&group_id).unwrap();
    let mut group_view = group.begin_view();
    assert!(group_view.try_ready());
    let live = Arc::new(Live(AtomicBool::new(true)));
    let verifier = V2GeneralPrefixVerifier::from_genesis(
        expectation,
        profile,
        admission,
        header,
        image,
        &group_view,
        V2ProjectionOrigin::new(live.clone()),
    )
    .unwrap();
    let founder = verifier.admission(admission.admission_id()).unwrap();
    let mut ledger = ChatLedger::new(
        Realm::new(&group_id, CHANNEL).unwrap(),
        LedgerLimits {
            max_actions: 16,
            max_action_bytes: 16 * MAX_ACTION_BYTES,
        },
    )
    .unwrap();
    let mut view = ledger.begin_projection_view_in(&group_view).unwrap();
    assert!(view.try_ready());
    Governance {
        view,
        _ledger: ledger,
        verifier,
        founder,
        live,
        _group_view: group_view,
        _group: group,
        group_id,
    }
}

/// An immutable unsigned Create Draft for `signer` under `admission`, using the drawn
/// presented time and nonce exactly once.
pub(crate) fn create_draft(
    g: &Governance,
    admission: &V2AuthenticatedAdmission,
    signer: [u8; 32],
    (presented_ms, nonce): (u64, [u8; 16]),
    text: &str,
) -> ActionDraft {
    let realm = Realm::new(&g.group_id, CHANNEL).unwrap();
    let labels = admission.admission_data().ordinary_labels(&realm).unwrap();
    ActionDraft::new(
        ActionSpec {
            realm,
            basis: labels.basis,
            grant: labels.grant,
            nonce,
            presented_ms,
            parents: Vec::new(),
            body: Body::Create {
                text: text.to_owned(),
                reply_to: None,
            },
        },
        signer,
    )
    .unwrap()
}
