"""Run the detached repair job's runtime guards through the shared Agent 3 mutation harness.

Design 10.3's evidence plan names the mutants this must keep detecting: the live claim consult
(M18), restoring the `replay_ready` gate on S2 or S3, admission before the first body read, and
the stale-rebuild check. The Registry job and its review add the custody boundary at S3, the
owner's crash-between-install-and-recycle resume, the per-offer hold, the claimed bucket's skipped
turn and the provider a queued preparation still needs.

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
STORE_REPAIR = "crates/catcoms-app/src/store/epoch_registry/repair.rs"
SOURCE = "crates/catcoms-app/src/store/epoch_registry/repair_source.rs"
REGISTRY = "studio::receiver::catchup::tests::repair::registry::"
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
        "RUNTIME-queued-preparation-provider", REPAIR,
        "        if holds_graph || none_pending {\n",
        "        if true || holds_graph || none_pending {\n",
        REGISTRY + "a_bucket_commit_never_strands_a_queued_registry_preparation",
        "a queued preparation keeps its attachment target",
    ),
]


if __name__ == "__main__":
    core.main("agent3-runtime-mutations")
