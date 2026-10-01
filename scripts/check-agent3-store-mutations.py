"""Run the inert fault-store guards through the same isolated mutation/restoration harness.

Run serially, only in Agent 3's isolated worktree or a disposable Actions checkout. The debug
override reduces this large app test binary's build memory without changing assertions/features.
"""

import importlib.util
from pathlib import Path


spec = importlib.util.spec_from_file_location(
    "agent3_mutations", Path(__file__).with_name("check-agent3-core-mutations.py")
)
core = importlib.util.module_from_spec(spec)
spec.loader.exec_module(core)
core.PREFIX = "store::epoch_owner::fault_record::tests::"
core.COMMAND = [
    "cargo", "test", "--locked", "--config", "profile.test.package.catcoms-app.debug=0",
    "-j", "1", "-p", "catcoms-app", "--lib",
]
OWNER = "crates/catcoms-app/src/store/epoch_owner.rs"
FAULT = "crates/catcoms-app/src/store/epoch_owner/fault_record.rs"
STUDIO_REPAIR = "crates/catcoms-app/src/store/epoch_studio/repair.rs"
STUDIO_ADOPTION = "crates/catcoms-app/src/store/epoch_studio/adoption.rs"
STUDIO_HEAD = "crates/catcoms-app/src/store/epoch_studio/discovery.rs"
APP_FAULT = "crates/catcoms-app/src/studio/fault.rs"
REPAIR_TESTS = "store::epoch_studio::tests::repair::"
core.MUTATIONS = [
    (
        "STORE-empty-fault-section", FAULT,
        "if pairs.is_empty() && reserved.is_none() && !has_overflow && bound.is_none() {",
        "if false {",
        "fault_record_empty_section_is_noncanonical",
        "empty fault section must be absent",
    ),
    (
        "STORE-live-fault-guard", OWNER,
        "if self.fault_record.is_some()\n", "if false\n",
        "fault_record_reopen_inventory_succeeds_but_legacy_reads_and_writes_refuse",
        "legacy live read must refuse repair-bearing state",
    ),
    (
        "STORE-live-journal-guard", OWNER,
        "            || self.journal.reconciled().is_some()\n"
        "            || self.journal.retained_repair().is_some()\n", "",
        "fault_record_journal_only_repair_cannot_bypass_legacy_fences_or_scope",
        "legacy live read must refuse repair-bearing state",
    ),
    (
        "STORE-repair-signature", FAULT,
        "            repair.verify_signature_only().map_err(invalid)?;", "",
        "fault_record_corruption_and_oversized_files_fail_structural_inventory",
        "inventory must reject corrupted repair signature",
    ),
    (
        "STORE-reconciled-scope", OWNER,
        "            .chain(self.journal.reconciled())\n", "",
        "fault_record_journal_only_repair_cannot_bypass_legacy_fences_or_scope",
        "reconciled-only journal must bind the enclosing document",
    ),
    (
        "STORE-exact-attestation", FAULT,
        "if hashes != self.hashes", "if false",
        "fault_record_attestations_bind_full_tuple_pair_and_epoch_shape",
        "attestation tuple and exact pair must bind",
    ),
    # Runtime repair guards. Each named regression runs alone and must fail at its own message.
    (
        "REPAIR-terminal-recycle", STUDIO_REPAIR,
        "        if owner.is_some() {\n"
        "            let terminal = TerminalRepairSource::after_flushed_source(repair.hash());",
        "        if false {\n"
        "            let terminal = TerminalRepairSource::after_flushed_source(repair.hash());",
        REPAIR_TESTS + "terminal_repair_recycles_to_ordinary_owner_state_across_save_and_reopen",
        "assertion failed: owner_is_ordinary",
    ),
    (
        "REPAIR-held-decision-fence", STUDIO_REPAIR,
        "if !request.names(held_pair.hashes()) || held.selected_receipt_hash != request.selected",
        "if false",
        REPAIR_TESTS
        + "a_b1_failure_retries_exactly_and_a_held_decision_owns_the_target_until_resumed",
        "a second decision wrote",
    ),
    (
        "REPAIR-adoption-claim", STUDIO_ADOPTION,
        "if self.epoch_owner_repair_claimed(server, &document)? {", "if false {",
        REPAIR_TESTS + "head_service_serves_an_applied_repair_but_never_proves_while_it_is_held",
        "ordinary discovery must not install into a held target",
    ),
    (
        "REPAIR-proof-gate", STUDIO_HEAD,
        "current.is_none_or(|c| journal.fault_suppresses_proof(r.hash(), c))",
        # Keeps `r` and `c` used so the mutant builds under CI's -D warnings.
        "current.is_none_or(|c| c == r.hash() && journal.fault_retains_member(c))",
        REPAIR_TESTS
        + "a_current_tenure_report_stages_suppresses_proof_and_is_decided_from_the_reserved_slot",
        "a staged live pair suppresses proof in the same answer",
    ),
    (
        "REPAIR-hint-filter", STUDIO_HEAD,
        ".filter(|r| !journal.fault_retains_member(r.hash()))", ".filter(|_| true)",
        REPAIR_TESTS
        + "a_current_tenure_report_stages_suppresses_proof_and_is_decided_from_the_reserved_slot",
        "a disputed receipt is not offered as a hint",
    ),
    (
        "REPAIR-fingerprint-release", FAULT,
        "            record.reserved = Some(admission.pair);\n"
        "            stored(&mut record);\n",
        "            record.reserved = Some(admission.pair);\n",
        REPAIR_TESTS
        + "a_current_tenure_report_stages_suppresses_proof_and_is_decided_from_the_reserved_slot",
        "nothing retained: tag 3 omitted",
    ),
    (
        "REPAIR-v5-issuance", APP_FAULT,
        "        // other check can answer with a less specific reason.\n"
        "        let observed = self.require_observed_owner_tenure()?;",
        "        // other check can answer with a less specific reason.\n"
        "        let observed = 0u64;",
        "studio::fault::tests::issuance_and_application_refuse_an_unobserved_tenure_with_that_message",
        "issuance must refuse Unknown as Unknown",
    ),
    (
        "REPAIR-current-admission", FAULT,
        "        for receipt in [a, b] {\n"
        "            receipt\n"
        "                .verify_current_owner(group, authoring_start)\n"
        "                .map_err(invalid)?;\n"
        "        }\n",
        "",
        REPAIR_TESTS
        + "a_current_tenure_report_stages_suppresses_proof_and_is_decided_from_the_reserved_slot",
        "only a current-tenure pair can be staged as live",
    ),
]


if __name__ == "__main__":
    core.main("agent3-store-mutations")
