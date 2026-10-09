"""Run the detached repair job's runtime guards through the shared Agent 3 mutation harness.

Design 10.3's evidence plan names the mutants this must keep detecting: the live claim consult
(M18), restoring the `replay_ready` gate on S2 or S3, admission before the first body read, and
the stale-rebuild check. The Registry job and its review add the custody boundary at S3, the
owner's crash-between-install-and-recycle resume, the per-offer hold, the claimed bucket's skipped
turn and the provider a queued preparation still needs. The fairness round's review adds the
per-target visit deferral that doubles and resets, the router's own resume of a landed install,
and the stale page S3 drops; its re-review adds the acknowledgement that ends a backoff, the cold
resume after a deferral, the router's snapshot gate, the held bucket's unprepared turn and the
Registry terminal reset.

Every replacement must compile under CI's `RUSTFLAGS=-D warnings`: a mutant that fails to build
would fail "at its intended assertion" only by accident, so the harness would reject it anyway.
Run serially, only in Agent 3's isolated worktree or a disposable Actions checkout.
"""

import importlib.util
from pathlib import Path


spec = importlib.util.spec_from_file_location(
    "agent3_mutations", Path(__file__).with_name("check-agent3-core-mutations.py")
)
core = importlib.util.module_from_spec(spec)
spec.loader.exec_module(core)
core.PREFIX = "studio::receiver::catchup::tests::repair::"
core.COMMAND = [
    "cargo", "test", "--locked", "--config", "profile.test.package.catcoms-app.debug=0",
    "-j", "1", "-p", "catcoms-app", "--lib",
]
CATCHUP = "crates/catcoms-app/src/studio/receiver/catchup.rs"
JOB = "crates/catcoms-app/src/studio/receiver/catchup/repair_job.rs"
REPAIR = "crates/catcoms-app/src/studio/receiver/catchup/repair.rs"
REGISTRY_RUNTIME = "crates/catcoms-app/src/studio/receiver/catchup/registry_runtime.rs"
RECEIVER = "crates/catcoms-app/src/studio/receiver.rs"
STORE_REPAIR = "crates/catcoms-app/src/store/epoch_registry/repair.rs"
SOURCE = "crates/catcoms-app/src/store/epoch_registry/repair_source.rs"
REGISTRY = "studio::receiver::catchup::tests::repair::registry::"
# A name containing "::" skips the core harness's prefix, and `--exact` then matches nothing, so a
# submodule's tests need their whole path.
TWO_PEER = "studio::receiver::catchup::tests::repair::two_peer::"
core.MUTATIONS = [
    (
        "RUNTIME-live-claim-consult", REPAIR,
        "        if self.repair_claimed(target) {\n",
        "        if false {\n",
        "a_live_job_claim_defers_installs_where_no_durable_claim_exists",
        "the live claim defers the install",
    ),
    (
        "RUNTIME-s2-replay-gate", CATCHUP,
        "        } else if let Some(job) = self.catchup.repair_detach() {\n",
        "        } else if let Some(job) = self\n"
        "            .catchup\n"
        "            .replay_ready()\n"
        "            .then(|| self.catchup.repair_detach())\n"
        "            .flatten()\n"
        "        {\n",
        "a_repair_job_is_never_parked_behind_the_catch_up_it_blocks",
        "the repair job detaches past pending catch-up",
    ),
    (
        "RUNTIME-s3-replay-gate", CATCHUP,
        "        if let Some(updated) = self.repair_commit(server, store, id)? {\n",
        "        if !self.replay_ready() {\n"
        "        } else if let Some(updated) = self.repair_commit(server, store, id)? {\n",
        "a_repair_job_is_never_parked_behind_the_catch_up_it_blocks",
        "S3 committed in the first turn",
    ),
    (
        "RUNTIME-admission-before-read", JOB,
        "        let Ok(permit) = self.preparation_pool().try_acquire_owned() else {\n",
        "        let _early = match target {\n"
        "            CheckpointTarget::Studio(studio) => server\n"
        "                .sync\n"
        "                .with_registry_context(|g, d, _, _| {\n"
        "                    store.capture_studio_source(id, g, studio, d)\n"
        "                })\n"
        "                .ok(),\n"
        "            CheckpointTarget::Registry(_) => None,\n"
        "        };\n"
        "        let Ok(permit) = self.preparation_pool().try_acquire_owned() else {\n",
        "a_repair_job_reserves_a_slot_before_reading_and_waits_flat_when_the_pool_is_full",
        "a refused job read the source",
    ),
    (
        "RUNTIME-studio-stale-rebuild", JOB,
        "                if !installed {\n",
        "                if false && !installed {\n",
        "a_rebuild_that_went_stale_during_s2_writes_nothing_and_backs_off",
        "a stale rebuild commits nothing",
    ),
    (
        "RUNTIME-registry-stale-rebuild", JOB,
        "                if !current {\n",
        "                if false && !current {\n",
        REGISTRY + "a_stale_bucket_rebuild_writes_nothing_and_backs_off",
        "a stale explicit decision is reported for its person to repeat",
    ),
    (
        "RUNTIME-registry-record-digest", SOURCE,
        "        blake3::hash(&record.plain) == self.digest && record.physical_bytes == self.physical\n",
        "        true || (blake3::hash(&record.plain) == self.digest\n"
        "            && record.physical_bytes == self.physical)\n",
        REGISTRY + "a_stale_bucket_rebuild_writes_nothing_and_backs_off",
        "a stale explicit decision is reported for its person to repeat",
    ),
    (
        "RUNTIME-registry-custody-restore", STORE_REPAIR,
        "            request,\n            None,\n            Some(prepared),\n",
        "            request,\n            None,\n            {\n                drop(prepared);\n"
        "                None\n            },\n",
        REGISTRY + "a_bucket_decision_runs_as_a_detached_job_without_a_custody_restore",
        "S1 and S3 never restored the bucket under custody",
    ),
    (
        "RUNTIME-owner-crash-resume", REPAIR,
        "            Some(owed) => Ok(owed.is_some_and(|(owed, _)| owed.hash() == repair.hash())),\n"
        "            None => server",
        "            Some(owed) if false => {\n"
        "                Ok(owed.is_some_and(|(owed, _)| owed.hash() == repair.hash()))\n"
        "            }\n"
        "            _ => server",
        REGISTRY + "an_owner_that_crashed_between_install_and_recycle_resumes_and_recycles",
        "no seed is fetched for a bucket that owes nothing",
    ),
    (
        "RUNTIME-offer-holds-repair-only", JOB,
        "        match offered {\n",
        "        match offered.filter(|_| false) {\n",
        REGISTRY + "a_failed_offer_holds_only_that_repair_and_the_owed_seed_is_still_fetched",
        "a failed offer holds that repair",
    ),
    (
        "RUNTIME-claimed-bucket-turn", REGISTRY_RUNTIME,
        "        if self.repair_claimed(CheckpointTarget::Registry(bucket)) {\n"
        "            // A repair job owns this bucket: no pointer refresh",
        "        if false {\n"
        "            // A repair job owns this bucket: no pointer refresh",
        REGISTRY + "a_claimed_bucket_gets_no_registry_turn",
        "the claimed bucket's turn is skipped, not spent",
    ),
    (
        # Plan D fairness: a held target's deferral must never delay another target's resume.
        "RUNTIME-per-target-visit", REPAIR,
        "        self.repair_visits\n"
        "            .get(&target)\n"
        "            .is_some_and(|(until, _)| now < *until)\n",
        "        let _ = target;\n"
        "        self.repair_visits.values().any(|(until, _)| now < *until)\n",
        "a_held_target_never_delays_another_targets_resume",
        "the next 5 s turn fetches the other target's owed seed",
    ),
    (
        # Review MEDIUM-1 on the fairness round: a repeating deferral doubles to its cap.
        "RUNTIME-visit-deferral-doubles", REPAIR,
        "                .saturating_mul(1u64 << streak.min(4))\n",
        "                .saturating_mul(1u64 << streak.min(0))\n",
        "a_seed_that_never_arrives_is_refetched_ever_more_rarely_until_the_repair_ends",
        "fetch starts, in seconds",
    ),
    (
        # ... and the streak ends with the repair.
        "RUNTIME-terminal-ends-backoff", REPAIR,
        "                self.repair_visits.remove(&scope);\n"
        "                self.remember_repair(scope, repair);\n",
        "                self.remember_repair(scope, repair);\n",
        "a_seed_that_never_arrives_is_refetched_ever_more_rarely_until_the_repair_ends",
        "a terminal outcome ends the backoff",
    ),
    (
        # ... or with a new explicit decision.
        "RUNTIME-decision-starts-afresh", JOB,
        "            self.repair_reports.remove(&target);\n"
        "            self.repair_visits.remove(&target);\n",
        "            self.repair_reports.remove(&target);\n",
        "repeated_holds_back_a_target_off_further_and_a_new_decision_starts_it_afresh",
        "a new decision does not inherit the old backoff",
    ),
    (
        # A deferred bucket's held decision still owns its Registry turn.
        "RUNTIME-deferred-bucket-turn-owned", REPAIR,
        "            return Ok(matches!(held, Ok(Some(_))));\n",
        "            return Ok(false);\n",
        REGISTRY + "a_held_bucket_never_delays_another_buckets_resume",
        "the held bucket's decision owns its turn",
    ),
    (
        # Review MEDIUM-2 on the fairness round: the router resumes a landed install itself.
        "RUNTIME-router-resumes-landed-install", REPAIR,
        "        if matches!(landed, Ok(true)) {\n",
        "        if false && matches!(landed, Ok(true)) {\n",
        "an_owner_that_crashed_between_install_and_recycle_resumes_a_studio_source",
        "the exact classification resumes the decision (cold, seed served)",
    ),
    (
        # Re-review LOW-1: a cold B3 visit after a deferral resumes instead of refetching.
        "RUNTIME-cold-b3-resumes-after-a-deferral", REPAIR,
        "                Ok(true) if !self.repair_visits.contains_key(&scope) => {\n",
        "                Ok(true) if true || !self.repair_visits.contains_key(&scope) => {\n",
        "an_owner_that_crashed_between_install_and_recycle_resumes_a_studio_source",
        "no second fetch",
    ),
    (
        # Second re-review MEDIUM: a scheduled resume job defers the target's next visit.
        "RUNTIME-scheduled-resume-defers", REPAIR,
        "            RepairSchedule::Scheduled | RepairSchedule::Held => self.defer_visit(scope, now),\n",
        "            RepairSchedule::Held => self.defer_visit(scope, now),\n"
        "            RepairSchedule::Scheduled => {}\n",
        "a_cold_owed_source_whose_fetch_cannot_start_reruns_its_job_ever_more_rarely",
        "a job on every visit",
    ),
    (
        # PR #36 review HIGH-1: a cold B3 guess whose fetch cannot start resumes instead.
        "RUNTIME-cold-guess-without-fetch-resumes", REPAIR,
        "            if warm || !alone {\n",
        "            if true || warm || !alone {\n",
        "an_owner_alone_after_a_crash_between_install_and_recycle_still_recycles",
        "a cold visit whose fetch cannot start resumes",
    ),
    (
        # ... but only with no peer at all: a busy slot waits a visit instead of paying a job.
        "RUNTIME-busy-slot-waits", REPAIR,
        "            if warm || !alone {\n",
        "            if warm || (false && !alone) {\n",
        "a_cold_guess_behind_a_busy_slot_waits_rather_than_resumes",
        "a busy slot waits; it does not resume",
    ),
    (
        # ... and for a bucket whose provider is unknown.
        "RUNTIME-bucket-guess-without-fetch-resumes", REPAIR,
        "                } else if guessed && server.sync.studio_page_peers().is_empty() {\n",
        "                } else if false && guessed && server.sync.studio_page_peers().is_empty() {\n",
        REGISTRY + "an_owner_alone_whose_bucket_seed_cannot_be_fetched_resumes_instead",
        "a guessed seed that cannot be fetched resumes",
    ),
    (
        # PR #36 review MEDIUM-1: a repeated offer's receipt reaches the running job's S3.
        "RUNTIME-repeat-offer-keeps-evidence", JOB,
        "            if !offered.as_deref().is_some_and(named) && named(&new) {\n",
        "            if false && !offered.as_deref().is_some_and(named) && named(&new) {\n",
        TWO_PEER + "a_repeated_offer_brings_the_receipt_its_running_job_lacked",
        "S3 used the receipt the second answer brought",
    ),
    (
        # PR #36 review MEDIUM-2: an offer for an absent copy holds that offer, not the document.
        "RUNTIME-absent-copy-holds-offer", JOB,
        "                    Some(repair) => self.hold_offer(target, repair, now),\n"
        "                    None => self.hold_repair(target, now),\n",
        "                    Some(_) | None => self.hold_repair(target, now),\n",
        TWO_PEER + "an_offer_for_a_document_this_peer_lacks_holds_only_that_offer",
        "that offer is held",
    ),
    (
        # PR #36 residual LOW-1: a stale rebuild of an offer holds the offer, not the document.
        "RUNTIME-stale-offer-holds-offer", JOB,
        "            Some(repair) => self.hold_offer_for(target, repair, now, REPAIR_STALE_RETRY_MS),\n",
        "            Some(_) => {\n"
        "                self.repair_backoff\n"
        "                    .insert(target, now.saturating_add(REPAIR_STALE_RETRY_MS));\n"
        "            }\n",
        TWO_PEER + "a_stale_rebuild_of_an_offer_holds_only_that_offer",
        "the stale offer waits the short stale retry",
    ),
    (
        # PR #27 review MEDIUM-1: S3 builds its budget only after installing the rebuild.
        "RUNTIME-install-before-budget", JOB,
        "                let installed = server.sync.with_registry_context(|g, d, _, _| {\n"
        "                    store.install_prepared_studio_source(g, d, *prepared)\n",
        "                let _ = Self::commit_budget(server, store, id, &input)?;\n"
        "                let installed = server.sync.with_registry_context(|g, d, _, _| {\n"
        "                    store.install_prepared_studio_source(g, d, *prepared)\n",
        "a_cold_source_over_the_receive_limit_commits_without_an_inline_validation",
        "the decision commits on the source S2 rebuilt",
    ),
    (
        # Agent 1's note on PR #36's merge: a Prepared document's skip resumes its bucket first.
        "RUNTIME-prepared-skip-resumes-bucket", REGISTRY_RUNTIME,
        "                if !self.repair_claimed(CheckpointTarget::Registry(bucket)) {\n",
        "                if false && !self.repair_claimed(CheckpointTarget::Registry(bucket)) {\n",
        REGISTRY + "a_prepared_document_never_strands_its_buckets_held_decision",
        "the held decision resumed despite the Prepared document",
    ),
    (
        # Re-review LOW-2: the router's resume needs a current durable snapshot.
        "RUNTIME-landed-install-needs-snapshot", REPAIR,
        "        if self.current_owner_snapshot(server, store, id).is_err() {\n",
        "        if false && self.current_owner_snapshot(server, store, id).is_err() {\n",
        "the_router_resumes_a_landed_install_only_under_a_current_snapshot",
        "no job starts without a current snapshot",
    ),
    (
        # Re-review MEDIUM-1: acknowledging the warning ends the document's backoff.
        "RUNTIME-acknowledge-ends-backoff", RECEIVER,
        "                self.catchup.repair_user_resolved(server, target);\n",
        "                if false {\n"
        "                    self.catchup.repair_user_resolved(server, target);\n"
        "                }\n",
        "acknowledging_a_documents_warning_ends_its_repair_backoff",
        "the acknowledgement ends the backoff",
    ),
    (
        # Re-review LOW-3: a backing-off held bucket's turn prepares nothing.
        "RUNTIME-held-bucket-turn-prepares-nothing", REGISTRY_RUNTIME,
        "        if self.registry_decision_waiting(server, store, id, bucket, now) {\n",
        "        if false && self.registry_decision_waiting(server, store, id, bucket, now) {\n",
        REGISTRY + "a_backing_off_held_bucket_spends_its_turn_without_preparing_anything",
        "nothing was prepared for it",
    ),
    (
        # Re-review LOW-4: a terminal bucket outcome ends that bucket's backoff.
        "RUNTIME-registry-terminal-ends-backoff", REPAIR,
        "            self.repair_visits.remove(&scope);\n"
        "            self.remember_repair(scope, repair);\n"
        "            return;\n",
        "            self.remember_repair(scope, repair);\n"
        "            return;\n",
        REGISTRY + "an_owed_bucket_replacement_is_fetched_then_installed_by_a_job",
        "a terminal bucket outcome ends the backoff",
    ),
    (
        # Review LOW-4 on the fairness round: S3 drops a page fetched for the source it rewrote.
        "RUNTIME-s3-drops-stale-page", REPAIR,
        "        if self.target == Some(target) && self.pass.is_some() {\n",
        "        if false && self.target == Some(target) && self.pass.is_some() {\n",
        TWO_PEER + "a_repair_that_retargets_a_source_with_a_pending_page_never_pauses_catch_up",
        "a repair must never pause catch-up",
    ),
    (
        # Review MEDIUM-1, held half: a held offer holds that repair, never its document.
        "RUNTIME-held-offer-holds-repair-only", REPAIR,
        "            StudioRepairOutcome::Held(_) if offered => self.hold_offer(scope, repair.hash(), now),\n",
        "            StudioRepairOutcome::Held(_) if false && offered => {\n"
        "                self.hold_offer(scope, repair.hash(), now)\n"
        "            }\n",
        TWO_PEER + "a_held_replay_on_a_real_peer_holds_only_itself_and_the_owed_seed_is_still_fetched",
        "the replay is held",
    ),
    (
        "RUNTIME-queued-preparation-provider", REPAIR,
        "        if holds_graph || none_pending {\n",
        "        if true || holds_graph || none_pending {\n",
        REGISTRY + "a_bucket_commit_never_strands_a_queued_registry_preparation",
        "a queued preparation keeps its attachment target",
    ),
]


if __name__ == "__main__":
    core.main("agent3-runtime-mutations")
