//! Shared outgoing progress slots for one ORIGINAL native process/group. Confirmation and
//! SDP phases share these two positions; neither phase can close another's capacity. This
//! owns capacity only. Every packet/epoch/join/role check remains at its original effect gate.
use super::*;

// Includes the independent owner, two WorkLanes, their idle slot credits/Arc counters and
// the weak group-index entry. This graph can outlive the phase which first allocated it,
// so it has its own payment rather than borrowing a phase's nearly-full fixed metadata.
pub(crate) const GROUP_FORWARD_METADATA_BYTES: usize = 1024;

pub(crate) struct GroupForwardLanes {
    lanes: [Arc<WorkLane>; 2],
    // Last: actual lane owners/credits and index metadata remain paid through destruction.
    metadata: Credit,
}
redacted_debug!(GroupForwardLanes);

impl GroupForwardLanes {
    /// Ordinary helper/public bytes can use an idle native lane without a phase reserve.
    /// Their original actual credit owns this group lane; phase retirement cannot refund it.
    pub(super) fn charge_ordinary(
        self: &Arc<Self>,
        resources: &Arc<Resources>,
        group: [u8; 32],
        bytes: usize,
    ) -> Result<Credit, VoiceError> {
        if !Arc::ptr_eq(resources, self.metadata.resources()) || group != self.metadata.group {
            return Err(VoiceError::Scope);
        }
        for lane in &self.lanes {
            let mut borrowed = match lane.borrow(resources, group) {
                Ok(borrowed) => borrowed,
                Err(VoiceError::Capacity) => continue,
                Err(error) => return Err(error),
            };
            borrowed.group = Some(self.clone());
            let mut credit = resources.charge_inner(group, bytes, ResourceKind::Retained, false)?;
            credit.lane = Some(borrowed);
            return Ok(credit);
        }
        Err(VoiceError::Capacity)
    }
    /// Borrow an actual shared position with bytes from the caller's original reservation.
    /// No ordinary slot/attempt is spent on local refusal. Every returned credit retains
    /// this SAME owner in LaneBorrow, independently of any Workflow/Exchange custody.
    pub(crate) fn charge(
        self: &Arc<Self>,
        resources: &Arc<Resources>,
        group: [u8; 32],
        reservation: &Arc<Reservation>,
        bytes: usize,
    ) -> Result<Credit, VoiceError> {
        if !Arc::ptr_eq(resources, self.metadata.resources()) || group != self.metadata.group {
            return Err(VoiceError::Scope);
        }
        for lane in &self.lanes {
            let budget = ByteBudget::ReservedLane {
                reservation: reservation.clone(),
                lane: lane.clone(),
            };
            match budget.charge(resources, group, bytes, ResourceKind::Forward) {
                Ok(mut credit) => {
                    credit
                        .lane
                        .as_mut()
                        .expect("actual shared lane borrower")
                        .group = Some(self.clone());
                    return Ok(credit);
                }
                Err(VoiceError::Capacity) => {}
                Err(error) => return Err(error),
            }
        }
        Err(VoiceError::Capacity)
    }
}
impl Drop for GroupForwardLanes {
    fn drop(&mut self) {
        // Remove our OWN weak index while metadata is still paid. Otherwise a live group
        // record could retain this dead owner allocation after its Credit has returned.
        // A concurrently installed replacement must keep its unrelated entry untouched.
        let indexed = {
            let mut state = self
                .metadata
                .pool
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            state
                .groups
                .get_mut(&self.metadata.group)
                .and_then(|counts| {
                    std::ptr::eq(counts.outgoing_lanes.as_ptr(), self)
                        .then(|| std::mem::take(&mut counts.outgoing_lanes))
                })
        };
        // Even fixed weak/control-block destruction stays outside Pool.
        drop(indexed);
        // Every actual borrower owns this group Arc, so this runs only after all such
        // requests/results have returned. Closing a Workflow/Exchange never calls this.
        for lane in &self.lanes {
            lane.close();
        }
    }
}

impl Resources {
    pub(super) fn existing_group_forward_lanes(
        &self,
        group: [u8; 32],
    ) -> Option<Arc<GroupForwardLanes>> {
        // Upgrade while Pool still fences the index, but return/drop the resulting STRONG
        // owner only after releasing it. A cloned Weak escaping this guard could pin a dead
        // allocation after concurrent last-owner Drop has already returned its payment.
        let existing = {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state
                .groups
                .get(&group)
                .and_then(|counts| counts.outgoing_lanes.upgrade())
        };
        existing
    }

    /// Reuse the group's exact capacity owner across phases, End and actor replacement.
    /// Prepare one candidate outside Pool. A racing loser releases its actual owners outside
    /// that lock. Transient preparation pressure refuses before any protocol identity is spent.
    pub(crate) fn group_forward_lanes(
        self: &Arc<Self>,
        group: [u8; 32],
    ) -> Result<Arc<GroupForwardLanes>, VoiceError> {
        if let Some(existing) = self.existing_group_forward_lanes(group) {
            return Ok(existing);
        }
        let prepared = (|| {
            let metadata =
                self.charge(group, GROUP_FORWARD_METADATA_BYTES, ResourceKind::Retained)?;
            let lanes = [
                self.reserve_forward_lane(group)?,
                self.reserve_forward_lane(group)?,
            ];
            Ok::<_, VoiceError>(Arc::new(GroupForwardLanes { lanes, metadata }))
        })();
        let prepared = match prepared {
            Ok(prepared) => prepared,
            Err(error) => return self.existing_group_forward_lanes(group).ok_or(error),
        };
        let (competing, retired) = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let counts = state
                .groups
                .get_mut(&group)
                .expect("prepaid shared lane metadata");
            if let Some(existing) = counts.outgoing_lanes.upgrade() {
                (Some(existing), None)
            } else {
                (
                    None,
                    Some(std::mem::replace(
                        &mut counts.outgoing_lanes,
                        Arc::downgrade(&prepared),
                    )),
                )
            }
        };
        drop(retired); // old index control blocks never destruct under Pool
        if let Some(competing) = competing {
            drop(prepared);
            Ok(competing)
        } else {
            Ok(prepared)
        }
    }
}

#[cfg(test)]
mod tests;
