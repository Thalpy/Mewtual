//! Continuing source coverage per helper and exact encrypted variant. This native search uses
//! the MLS leaf catalog, never a truncated connected-peer or evictable descriptor snapshot.
//! Transport preparation owns admission: merely proposing a batch does not consume coverage.
use catcoms_crypto::DeviceId;
use catcoms_mls::group::{
    MemberCatalogCursor, MemberCatalogEntry, MemberCatalogError, MemberCatalogView,
};
use catcoms_mls::ServerGroup;
use catcoms_rt::PeerId;
use std::sync::Arc;

const HELPERS: usize = 4;
const VARIANTS: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Helper {
    pub device: DeviceId,
    pub peer: PeerId,
}
#[derive(Clone, Debug)]
pub(crate) struct Choice {
    pub variant: [u8; 32],
    pub helper: Helper,
    pub sources: Vec<MemberCatalogEntry>,
    ticket: u64,
    owner: Arc<()>,
}
impl Choice {
    /// A cloned observation may propose the same batch, but cannot become a later generation.
    pub(super) fn same_as(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.owner, &other.owner)
            && self.ticket == other.ticket
            && self.variant == other.variant
            && self.helper == other.helper
            && self.sources.len() == other.sources.len()
            && self.sources.iter().zip(&other.sources).all(|(a, b)| {
                a.device_id() == b.device_id() && a.signature_key() == b.signature_key()
            })
    }
}
struct Progress {
    variant: [u8; 32],
    helper: DeviceId,
    cursor: MemberCatalogCursor,
    complete: bool,
    retry: bool,
    passes: u8,
}
struct Proposal {
    choice: Choice,
    position: usize,
    next: MemberCatalogCursor,
    complete: bool,
}

/// These observations describe only this read's admitted helper/variant combinations. Exhausted
/// is not a signed statement of global absence. Yield advances at most one 64-position local
/// catalog turn; the owning job's original deadline and durable allowance still bound execution.
#[derive(Debug)]
pub(crate) enum Step {
    Ready(Choice),
    Yield,
    Backoff,
    Exhausted,
}
pub(crate) struct Search {
    owner: Arc<()>,
    requester: DeviceId,
    view: MemberCatalogView,
    helpers: Vec<DeviceId>,
    variants: Vec<[u8; 32]>,
    progress: Vec<Progress>,
    unsupported: Vec<DeviceId>,
    pending: Option<Proposal>,
    turn: usize,
    ticket: u64,
}
impl Search {
    /// Only the native owner supplies the actual local C and immutable current MLS observation.
    /// Root must additionally bind this state to its original native/read/epoch lifetime; a public
    /// roster view itself grants no local membership, file or route authority.
    pub(crate) fn new(requester: DeviceId, view: MemberCatalogView) -> Self {
        Self {
            owner: Arc::new(()),
            requester,
            view,
            helpers: Vec::new(),
            variants: Vec::new(),
            progress: Vec::new(),
            unsupported: Vec::new(),
            pending: None,
            turn: 0,
            ticket: 0,
        }
    }

    /// Propose one page without committing its source cursor. Repeating after a proven LOCAL
    /// pre-handoff Busy returns the same sources and ticket. If that helper/variant is currently
    /// unavailable, drop only the unstarted proposal and allow another pair to progress; its
    /// original cursor remains intact. No attempt is refunded after an ambiguous handoff.
    pub(crate) fn propose(
        &mut self,
        group: &ServerGroup,
        variants: &[[u8; 32]],
        helpers: &[Helper],
    ) -> Result<Step, MemberCatalogError> {
        self.view.validate(group)?;
        if let Some(proposal) = &self.pending {
            for entry in &proposal.choice.sources {
                entry.validate(group)?;
            }
            if variants.contains(&proposal.choice.variant)
                && !self.unsupported.contains(&proposal.choice.helper.device)
            {
                if let Some(helper) = helpers
                    .iter()
                    .find(|h| h.device == proposal.choice.helper.device)
                {
                    let mut choice = proposal.choice.clone();
                    if choice.helper != *helper {
                        // PeerId can change away and back. A distinct preparation generation
                        // prevents an old clone becoming current again, without spending the
                        // proposed source cursor. Same-PeerId connection incarnations still
                        // require the original Root/transport lease at actual admission.
                        self.ticket = self
                            .ticket
                            .checked_add(1)
                            .ok_or(MemberCatalogError::Inconsistent)?;
                        choice.ticket = self.ticket;
                    }
                    choice.helper = *helper;
                    self.pending.as_mut().unwrap().choice = choice.clone();
                    return Ok(Step::Ready(choice));
                }
            }
            self.pending = None;
        }
        for helper in helpers {
            if helper.device != self.requester
                && self.helpers.len() < HELPERS
                && !self.helpers.contains(&helper.device)
            {
                self.helpers.push(helper.device);
            }
        }
        for variant in variants {
            if self.variants.len() < VARIANTS && !self.variants.contains(variant) {
                self.variants.push(*variant);
            }
        }
        let combinations = self.helpers.len() * self.variants.len();
        if combinations == 0 {
            return Ok(Step::Exhausted);
        }
        // At most16 fixed metadata rows, even under route churn. Only ONE actual catalog page
        // is read per call, so a run of holes cannot turn an actor turn into a roster scan.
        for _ in 0..combinations {
            let index = self.turn % combinations;
            self.turn = (index + 1) % combinations;
            let variant = self.variants[index / self.helpers.len()];
            let device = self.helpers[index % self.helpers.len()];
            if !variants.contains(&variant) || self.unsupported.contains(&device) {
                continue;
            }
            let Some(helper) = helpers.iter().find(|h| h.device == device).copied() else {
                continue;
            };
            let position = if let Some(position) = self
                .progress
                .iter()
                .position(|p| p.variant == variant && p.helper == device)
            {
                position
            } else {
                self.progress.push(Progress {
                    variant,
                    helper: device,
                    cursor: self.view.start(),
                    complete: false,
                    retry: false,
                    passes: 0,
                });
                self.progress.len() - 1
            };
            let progress = &mut self.progress[position];
            if progress.complete {
                continue;
            }
            let page = self
                .view
                .page(group, &progress.cursor, self.requester, device)?;
            if page.entries.is_empty() {
                progress.cursor = page.next;
                progress.complete = page.complete;
                return Ok(Step::Yield);
            }
            self.ticket = self
                .ticket
                .checked_add(1)
                .ok_or(MemberCatalogError::Inconsistent)?;
            let choice = Choice {
                variant,
                helper,
                sources: page.entries,
                ticket: self.ticket,
                owner: self.owner.clone(),
            };
            self.pending = Some(Proposal {
                choice: choice.clone(),
                position,
                next: page.next,
                complete: page.complete,
            });
            return Ok(Step::Ready(choice));
        }
        // A signed Unavailable can mean temporary pressure rather than missing data. Revisit
        // affected helper/variant pairs only AFTER fresh coverage, never once per offline batch.
        // Sixteen flags and two additional passes are bounded by the same read/Keep allowance.
        let mut retry = false;
        for progress in &mut self.progress {
            if progress.complete
                && progress.retry
                && progress.passes < 2
                && variants.contains(&progress.variant)
                && !self.unsupported.contains(&progress.helper)
                && helpers.iter().any(|h| h.device == progress.helper)
            {
                progress.cursor = self.view.start();
                progress.complete = false;
                progress.retry = false;
                progress.passes += 1;
                retry = true;
            }
        }
        Ok(if retry {
            Step::Backoff
        } else {
            Step::Exhausted
        })
    }

    /// Commit only the exact admitted proposal. The caller must retain any known-unstarted lower
    /// preparation under its ORIGINAL authority/deadline, rather than refunding a submitted batch.
    /// A stale cloned Choice cannot accidentally commit a later pending page.
    pub(crate) fn admitted(&mut self, choice: &Choice) -> bool {
        if !self.pending.as_ref().is_some_and(|p| {
            Arc::ptr_eq(&p.choice.owner, &choice.owner)
                && p.choice.ticket == choice.ticket
                && p.choice.helper == choice.helper
                && p.choice.variant == choice.variant
                && p.choice.sources.len() == choice.sources.len()
                && p.choice.sources.iter().zip(&choice.sources).all(|(a, b)| {
                    a.device_id() == b.device_id() && a.signature_key() == b.signature_key()
                })
        }) {
            return false;
        }
        let proposal = self.pending.take().unwrap();
        let progress = &mut self.progress[proposal.position];
        progress.cursor = proposal.next;
        progress.complete = proposal.complete;
        true
    }

    /// Root has checked its exact currently selected choice, including admitted retries. Mint
    /// a new generation only when the helper route changes, preserving sources and progress.
    pub(super) fn refresh(
        &mut self,
        choice: &Choice,
        helper: Helper,
    ) -> Result<Choice, MemberCatalogError> {
        if !Arc::ptr_eq(&self.owner, &choice.owner)
            || choice.helper.device != helper.device
            || self.unsupported.contains(&helper.device)
            || self
                .pending
                .as_ref()
                .is_some_and(|p| !p.choice.same_as(choice))
        {
            return Err(MemberCatalogError::Inconsistent);
        }
        let mut refreshed = choice.clone();
        if choice.helper != helper {
            self.ticket = self
                .ticket
                .checked_add(1)
                .ok_or(MemberCatalogError::Inconsistent)?;
            refreshed.ticket = self.ticket;
            refreshed.helper = helper;
            if let Some(pending) = self.pending.as_mut() {
                pending.choice = refreshed.clone();
            }
        }
        Ok(refreshed)
    }

    pub(super) fn retry_batch(&mut self, choice: &Choice) {
        if Arc::ptr_eq(&self.owner, &choice.owner) {
            if let Some(progress) = self
                .progress
                .iter_mut()
                .find(|p| p.variant == choice.variant && p.helper == choice.helper.device)
            {
                progress.retry = true;
            }
        }
    }

    /// Relinquish an unstarted selection without advancing its source cursor. Reintroducing
    /// that variant must mint a fresh ticket, even if bootstrap yields before the next search.
    pub(super) fn retire_pending(&mut self, choice: &Choice) {
        if self
            .pending
            .as_ref()
            .is_some_and(|p| p.choice.same_as(choice))
        {
            self.pending = None;
        }
    }

    /// An old peer's empty new-kind response disables this helper for this read. It never enables
    /// legacy whole-file forwarding, evicts a row or resets completed coverage after reconnect.
    pub(crate) fn unsupported(&mut self, device: DeviceId) {
        if self.helpers.contains(&device) && !self.unsupported.contains(&device) {
            self.unsupported.push(device);
        }
    }
}

#[cfg(test)]
mod tests;
