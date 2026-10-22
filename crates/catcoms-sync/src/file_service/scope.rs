//! Existing direct file work also needs the sole Root's current MLS/policy lifetime. This is
//! deliberately independent of optional voice activation and of the stronger bridge route gates.
//! It grants no peer standing, new protocol permission, native unlock or execution allowance.
use std::fmt;
use std::sync::Arc;

use catcoms_rt::{CryptoRngCore, MeshTransport, RequestAuthority};
use tokio::sync::watch;

use crate::{ChannelSync, SyncError};

#[derive(Clone, Copy, PartialEq, Eq)]
struct View {
    epoch: u64,
    active_member: bool,
    policy: Option<[u8; 32]>,
    policy_active: bool,
    policy_revision: u64,
}

/// Sole publisher. This is the FIRST ChannelSync field, so ordinary field destruction closes
/// every observer before releasing transport/MLS/store ownership. No worker/handle clones it.
/// Reconciliation permanently closes a generation; later readmission creates a fresh channel.
pub(crate) struct FileEffectOwners {
    view: Option<View>,
    live: watch::Sender<bool>,
}
impl Default for FileEffectOwners {
    fn default() -> Self {
        Self {
            view: None,
            live: watch::channel(false).0,
        }
    }
}
impl FileEffectOwners {
    fn observe(&mut self, view: View) {
        if self.view != Some(view) {
            self.live.send_replace(false);
            self.live = watch::channel(view.active_member).0;
            self.view = Some(view);
        }
    }
}
struct Observer(watch::Receiver<bool>);
impl fmt::Debug for Observer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("FileRootScope { .. }")
    }
}
#[async_trait::async_trait]
impl RequestAuthority for Observer {
    fn authorized(&self) -> bool {
        *self.0.borrow() && self.0.has_changed().is_ok()
    }
    async fn revoked(&self) {
        let mut live = self.0.clone();
        while self.authorized() {
            if live.changed().await.is_err() {
                return;
            }
        }
    }
}
impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    /// Reconcile BEFORE any optional voice early return at the ordinary observed transition
    /// hooks. Existing direct-read compatibility permits legacy groups, but epoch/member/policy
    /// changes still retire their captured work. New bridge service requires additional explicit
    /// authenticated P2P policy, current descriptors and captured Noise connection incarnations.
    pub(crate) fn reconcile_file_effects(&mut self) {
        // This is ordinary native construction/committed-membership reconciliation. Cache one
        // epoch observation here, before a later bounded requester page can use it. Page APIs
        // never repair a missing/stale view by walking the roster themselves.
        self.group.observe_member_catalog();
        self.file_effects.observe(View {
            epoch: self.group.epoch(),
            active_member: self.group.is_active()
                && self.group.contains_device(&self.device.device_id()),
            policy: self.group_policy_digest(),
            policy_active: self.group_policy_active,
            policy_revision: self.group_policy_revision,
        });
    }

    /// Capture the exact current Root/member/policy lifetime for a native file job. This observer
    /// owns no Root, key, native entry or revocation publisher. Keep it with the whole job and
    /// compose it at each actual lower/effect edge; checking only returned bytes is too late.
    /// Root replacement/drop, observed membership/epoch or policy transitions retire old captures.
    /// The caller must additionally enforce native account/entry/actor scope and its original
    /// deadline/run. This does not authorize a destination, manifest, new request kind or bridge.
    pub fn file_effect_authority(&mut self) -> Result<Arc<dyn RequestAuthority>, SyncError> {
        self.reconcile_file_effects();
        let observer = Arc::new(Observer(self.file_effects.live.subscribe()));
        if !observer.authorized() {
            return Err(SyncError::Unauthorized);
        }
        Ok(observer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> View {
        View {
            epoch: 1,
            active_member: true,
            policy: None,
            policy_active: false,
            policy_revision: 0,
        }
    }
    fn capture(owner: &FileEffectOwners) -> Arc<Observer> {
        Arc::new(Observer(owner.live.subscribe()))
    }

    #[tokio::test]
    async fn scope_replacement_drop_and_repeated_view_never_revive_old_observers() {
        let mut owner = FileEffectOwners::default();
        assert!(!capture(&owner).authorized());
        owner.observe(view());
        let old = capture(&owner);
        assert!(old.authorized());
        owner.observe(view());
        assert!(
            old.authorized(),
            "unchanged observation does not cancel healthy work"
        );
        owner.observe(View { epoch: 2, ..view() });
        assert!(!old.authorized());
        old.revoked().await;
        owner.observe(view());
        assert!(
            !old.authorized(),
            "returning to an old view is a NEW generation"
        );
        let current = capture(&owner);
        assert!(current.authorized());
        drop(owner);
        assert!(!current.authorized());
        current.revoked().await;
    }

    #[tokio::test]
    async fn removal_policy_activation_revision_and_body_each_retire_scope() {
        for changed in [
            View {
                active_member: false,
                ..view()
            },
            View {
                policy: Some([7; 32]),
                ..view()
            },
            View {
                policy_active: true,
                ..view()
            },
            View {
                policy_revision: 1,
                ..view()
            },
        ] {
            let mut owner = FileEffectOwners::default();
            owner.observe(view());
            let old = capture(&owner);
            let mut waiting = std::pin::pin!(old.revoked());
            std::future::poll_fn(|cx| {
                use std::future::Future;
                assert!(waiting.as_mut().poll(cx).is_pending());
                std::task::Poll::Ready(())
            })
            .await;
            owner.observe(changed);
            assert!(!old.authorized());
            waiting.await;
        }
    }

    #[tokio::test]
    async fn actual_policy_migration_retires_scope_before_optional_voice_return() {
        let (_, mut members, _) = crate::tests::build_members(2).await;
        let mut root = members.remove(0);
        assert!(root.voice_runtime.is_none());
        let legacy = root.file_effect_authority().unwrap();
        root.initialize_group_policy(crate::GroupMode::PeerToPeer)
            .unwrap();
        assert!(
            !legacy.authorized(),
            "real policy digest/revision transition retires captured work"
        );
        // Existing direct compatibility remains eligible while a migration awaits persistence.
        let migrating = root.file_effect_authority().unwrap();
        assert!(migrating.authorized());
        root.publish_group_policy().unwrap();
        assert!(
            !migrating.authorized(),
            "actual activation is an independent lifetime transition"
        );
        assert!(!legacy.authorized());
        assert!(root.file_effect_authority().unwrap().authorized());
    }

    #[tokio::test]
    async fn actual_root_without_voice_attachment_drop_retires_its_file_scope() {
        let (_, mut members, _) = crate::tests::build_members(2).await;
        let mut root = members.pop().unwrap();
        assert!(root.voice_runtime.is_none());
        let scope = root.file_effect_authority().unwrap();
        assert!(scope.authorized());
        root.reconcile_voice_actor();
        assert!(scope.authorized());
        drop(root);
        assert!(!scope.authorized());
        scope.revoked().await;
    }
}
