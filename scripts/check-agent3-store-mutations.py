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
core.MUTATIONS = [
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
]


if __name__ == "__main__":
    core.main("agent3-store-mutations")
