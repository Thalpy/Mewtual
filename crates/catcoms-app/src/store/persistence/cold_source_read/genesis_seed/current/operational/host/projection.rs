//! One independently paid negative publication domain for the actual original Host.
//!
//! Kept private until the same paid verifier and close consumers are paired. Neither
//! group labels nor this domain supplies semantic authority. Core verifies the actual
//! pinned genesis separately; no owning Origin/View/revoker getter is exposed here.
use super::*;
use crate::store::critical_namespaces::read_pool::{
    CatalogReadReservation, CriticalCatalogReadPool,
};
use crate::store::persistence::v2_text::{
    V2TextFounderAnchorData, V2_TEXT_FOUNDER_ANCHOR_LAYOUT_BYTES,
};
use crate::store::persistence::V2TextOwnerRef;
use crate::store::ServerStore;
use catcoms_replication::chat::owner_header::SignedOwnerHeader;
use catcoms_replication::chat::owner_profile::{AdmissionBody, SignedOwnerProfile};
use catcoms_replication::chat::{
    FounderGenesisAnchorData, ProjectionGroup, ProjectionGroupCloseCompletion, ProjectionGroupView,
    V2FounderExpectation, V2FounderPrefixVerifier, V2OriginNegative, V2ProjectionOrigin,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, TryLockError};

const DOMAIN_METADATA_BYTES: usize = 16 * 1024;
#[cfg(all(test, windows))]
thread_local! {
    static NEXT_DOMAIN_FEE_NOTICE: std::cell::RefCell<Option<Arc<AtomicBool>>> = const { std::cell::RefCell::new(None) };
}
const DOMAIN_LAYOUT_BYTES: usize = 2 * std::mem::size_of::<V2TextProjectionDomain>()
    + 2 * std::mem::size_of::<BoundV2TextFounderAnchor>()
    + 4 * std::mem::size_of::<DomainNegative>()
    + 2 * std::mem::size_of::<DomainFunding>()
    + ProjectionGroup::initial_metadata_layout_bytes()
    + V2ProjectionOrigin::retained_layout_bytes()
    + 2 * V2FounderPrefixVerifier::metadata_layout_bytes()
    + 2 * std::mem::size_of::<FounderGenesisAnchorData>()
    + V2_TEXT_FOUNDER_ANCHOR_LAYOUT_BYTES
    + 4 * catcoms_replication::chat::MAX_GROUP_ID_BYTES
    + 2 * 1024
    + super::create::V2_TEXT_RESIDENT_DOMAIN_LAYOUT_BYTES;
const _: () = assert!(DOMAIN_LAYOUT_BYTES <= DOMAIN_METADATA_BYTES);

struct DomainFunding {
    pool: CriticalCatalogReadPool,
    // LAST: concrete negative/GroupLife/Origin tails retain this independent debit.
    fee: CatalogReadReservation,
}
struct DomainNegative {
    source: V2TextSourceNegativeObserver,
    stopped: AtomicBool,
    // LAST: funding follows the concrete negative into Core's original Origin.
    funding: Arc<DomainFunding>,
}
impl V2OriginNegative for DomainNegative {
    fn is_live(&self) -> bool {
        !self.stopped.load(Ordering::Acquire) && self.source.is_live()
    }
}
impl std::fmt::Debug for DomainNegative {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TextProjectionNegative { .. }")
    }
}

/// Resource-backed cached negative only. No Core Origin, close or positive permission
/// is lent; a caller cannot independently cancel Core while this object appears live.
#[derive(Clone)]
pub(super) struct V2TextProjectionNegative {
    body: Arc<DomainNegative>,
}
impl V2TextProjectionNegative {
    pub(super) fn validate_negative(&self) -> Result<(), AppError> {
        if !self.body.is_live() {
            return Err(refused());
        }
        Ok(())
    }
    pub(super) fn require_same_original(&self, other: &Self) -> Result<(), AppError> {
        if !Arc::ptr_eq(&self.body, &other.body) {
            return Err(refused());
        }
        Ok(())
    }
}

/// Max ONE per whole original Host. Future replacement on the same projection target
/// must preserve this actual flat domain/close debt, rather than making an equal group.
pub(super) struct V2TextProjectionDomain {
    _group: ProjectionGroup,
    view: ProjectionGroupView,
    origin: V2ProjectionOrigin,
    // Whole unsigned custody shares the SAME domain so lower-level genuine owners
    // cannot drop its Group/View prematurely. Only explicit outside-writer close
    // touches this small completion cell; Core's negative callback never locks it.
    closing: Mutex<Option<ProjectionGroupCloseCompletion>>,
    // LAST: no group/view/actual close-completion tail outlives its original fee.
    negative: V2TextProjectionNegative,
}
/// Produced only by SAME-domain/SAME-source checks in actual Current. Neither an
/// equal group label nor an independently genuine anchor can swap its Origin.
pub(super) struct BoundV2TextFounderAnchor {
    anchor: V2TextFounderAnchorData,
    negative: V2TextProjectionNegative,
}
impl V2TextProjectionDomain {
    #[cfg(all(test, windows))]
    pub(super) fn with_fee_drop_notice_for_test<T>(
        notice: Arc<AtomicBool>,
        prepare: impl FnOnce() -> T,
    ) -> T {
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                NEXT_DOMAIN_FEE_NOTICE.with(|slot| {
                    slot.borrow_mut().take();
                });
            }
        }
        NEXT_DOMAIN_FEE_NOTICE.with(|slot| {
            assert!(
                slot.borrow().is_none(),
                "nested exact domain-fee observation"
            );
            *slot.borrow_mut() = Some(notice);
        });
        let _reset = Reset;
        let output = prepare();
        NEXT_DOMAIN_FEE_NOTICE.with(|slot| {
            assert!(
                slot.borrow().is_none(),
                "fixture missed actual domain reservation"
            )
        });
        output
    }
    /// Outside all writers: this prepares only a negative domain and its actual fees.
    /// The concrete Current callback separately binds SAME source before verifier use.
    pub(super) fn prepare(
        store: &ServerStore,
        owner: &V2TextOwnerRef<'_>,
        source: &V2TextSourceNegativeObserver,
    ) -> Result<Self, AppError> {
        owner.require_original_negative(store, &owner.observe_writer())?;
        if !source.is_live() {
            return Err(refused());
        }
        let pool = store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(refused)?
            .pending
            .pool
            .clone();
        let fee = pool.try_reserve(DOMAIN_METADATA_BYTES)?;
        #[cfg(all(test, windows))]
        let fee = {
            let mut fee = fee;
            NEXT_DOMAIN_FEE_NOTICE.with(|slot| {
                if let Some(notice) = slot.borrow_mut().take() {
                    fee.observe_drop_for_test(notice);
                }
            });
            fee
        };
        fee.validate(&pool, DOMAIN_METADATA_BYTES)?;
        // Funding is declared before every paid group/negative/control allocation.
        let funding = Arc::new(DomainFunding { pool, fee });
        store.with_v2_text_current(owner, |admission| source.require_same_original(admission))?;
        owner.require_original_negative(store, &owner.observe_writer())?;
        if !source.is_live() {
            return Err(refused());
        }
        let negative = V2TextProjectionNegative {
            body: Arc::new(DomainNegative {
                source: source.clone(),
                stopped: AtomicBool::new(false),
                funding,
            }),
        };
        let mut group = ProjectionGroup::new(owner.source().data().context.spec_data().group)
            .map_err(|_| refused())?;
        let mut view = group.begin_view();
        if !view.try_ready() {
            return Err(refused());
        }
        let origin = V2ProjectionOrigin::new(negative.body.clone());
        negative.validate_negative()?;
        Ok(Self {
            _group: group,
            view,
            origin,
            closing: Mutex::new(None),
            negative,
        })
    }

    pub(super) fn validate_accounts(&self, store: &ServerStore) -> Result<(), AppError> {
        let pool = &store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(refused)?
            .pending
            .pool;
        self.negative
            .body
            .funding
            .fee
            .validate(&self.negative.body.funding.pool, DOMAIN_METADATA_BYTES)?;
        self.negative
            .body
            .funding
            .fee
            .validate(pool, DOMAIN_METADATA_BYTES)
    }
    pub(super) fn validate_accounts_only(&self) -> Result<(), AppError> {
        let funding = &self.negative.body.funding;
        funding.fee.validate(&funding.pool, DOMAIN_METADATA_BYTES)
    }

    /// Paid cached association only; the caller still proves actual selected physical
    /// history before trusting an anchor. This cannot make historical O_G current again.
    pub(super) fn require_current_source(
        &self,
        admission: &crate::store::chat_admission::v2_current::V2TextCurrentAdmission<'_>,
    ) -> Result<(), AppError> {
        self.negative.body.source.require_same_original(admission)?;
        self.negative.validate_negative()
    }

    pub(super) fn negative(&self) -> V2TextProjectionNegative {
        self.negative.clone()
    }
    pub(super) fn validate_negative(&self) -> Result<(), AppError> {
        self.negative.validate_negative()
    }

    /// All fallible checks precede the owning move, preserving the outside anchor
    /// slot on refusal. The binding retains exact domain identity outside Current.
    pub(super) fn bind_anchor_into(
        &self,
        guard: &crate::store::critical_namespaces::current_layout::V2TextCurrentGroupGuard<'_, '_>,
        anchor: &mut Option<V2TextFounderAnchorData>,
        bound: &mut Option<BoundV2TextFounderAnchor>,
    ) -> Result<(), AppError> {
        if let Some(existing) = bound.as_ref() {
            if anchor.is_some() {
                return Err(refused());
            }
            self.negative.require_same_original(&existing.negative)?;
            existing.anchor.require_original_source_data(guard)?;
            return self.require_current_source(guard.scope_admission());
        }
        anchor
            .as_ref()
            .ok_or_else(refused)?
            .require_original_source_data(guard)?;
        self.require_current_source(guard.scope_admission())?;
        self.negative.validate_negative()?;
        *bound = Some(BoundV2TextFounderAnchor {
            anchor: anchor.take().expect("same checked original anchor"),
            negative: self.negative.clone(),
        });
        Ok(())
    }

    /// Only the private actual Host anchor producer may choose this expectation. SDK
    /// records/unsigned bytes never choose their own trust root. All math runs outside
    /// global writers under the independent resident-model debit held by its caller.
    pub(super) fn verifier_from_genesis(
        &self,
        anchor: &BoundV2TextFounderAnchor,
        profile: &SignedOwnerProfile,
        admission: &AdmissionBody,
        header: &SignedOwnerHeader,
        image: &[u8],
    ) -> Result<V2FounderPrefixVerifier, AppError> {
        self.negative.validate_negative()?;
        self.negative.require_same_original(&anchor.negative)?;
        let data = anchor.anchor.core_data();
        let expectation = V2FounderExpectation::from_trusted_host(FounderGenesisAnchorData {
            group: data.group.clone(),
            founder: data.founder,
            profile: data.profile,
            admission: data.admission,
            tenure: data.tenure,
            header: data.header,
            state: data.state,
        })
        .map_err(|_| refused())?;
        V2FounderPrefixVerifier::from_genesis(
            expectation,
            profile,
            admission,
            header,
            image,
            &self.view,
            self.origin.clone(),
        )
        .map_err(|_| refused())
    }

    /// Cancellation is immediate but is never an acknowledgment of callback drain.
    pub(super) fn cancel(&self) {
        self.negative.body.stopped.store(true, Ordering::Release);
        self.origin.cancel();
        let mut closing = self
            .closing
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if closing.is_none() {
            *closing = Some(self.view.revoker().close());
        }
    }
    /// Poll the ACTUAL group barrier outside all native/Current writers. The domain
    /// keeps that completion and its fee even when callbacks have not yet returned.
    pub(super) fn try_closed(&self) -> bool {
        let mut closing = match self.closing.try_lock() {
            Ok(closing) => closing,
            Err(TryLockError::WouldBlock) => return false,
            Err(TryLockError::Poisoned(poison)) => poison.into_inner(),
        };
        closing
            .as_mut()
            .is_some_and(ProjectionGroupCloseCompletion::try_complete)
    }
}
impl Drop for V2TextProjectionDomain {
    fn drop(&mut self) {
        // Drop cancels only. Explicit Host close owns/polls the real completion.
        self.cancel();
    }
}
