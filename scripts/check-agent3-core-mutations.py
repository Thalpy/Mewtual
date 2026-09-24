"""Run isolated Agent 3 core mutations and require exact restored regression passes.

Run only in an isolated worktree or a disposable GitHub Actions checkout. This deliberately
does not share the other agents' mutable sources. Set CARGO_TARGET_DIR externally to an
existing target directory when disk is constrained; wait for other local builds first.
"""

from pathlib import Path
import os
import subprocess


ROOT = Path(__file__).resolve().parents[1]
PREFIX = "epoch::repair_state::tests::"
COMMAND = ["cargo", "test", "--locked", "-j", "1", "-p", "catcoms-replication", "--lib"]
ANCHOR_TEST = "repair_losing_adoption_anchors_do_not_recreate_a_resolved_fault"
MUTATIONS = [
    (
        "CORE006-nonterminal-nochange", "crates/catcoms-replication/src/epoch/repair_transition/joint.rs",
        "        journal.check_repair_progress(repair)?;", "",
        "registry_epoch::repair::tests::joint_repair_nochange_cannot_bypass_unfinished_journal_provenance",
        "unfinished journal must fence even NoChange",
    ),
    (
        "CORE006-effective-choice", "crates/catcoms-replication/src/epoch/repair_transition/joint.rs",
        "if journal\n        .effective_choice()\n        .is_some_and(|r| resolved.covers(r))\n    {", "if false {",
        "epoch::repair_transition::joint::tests::joint_compatibility_rejects_covered_roles_and_inconsistent_gate_binding",
        "covered journal choice must refuse",
    ),
    (
        "CORE006-covered-source", "crates/catcoms-replication/src/epoch/repair_transition/joint.rs",
        "if book.latest().is_some_and(|r| resolved.covers(r)) {", "if false {",
        "epoch::repair_transition::joint::tests::joint_compatibility_rejects_covered_roles_and_inconsistent_gate_binding",
        "unsafe source role 0 must refuse",
    ),
    (
        "CORE006-source-version", "crates/catcoms-replication/src/epoch/repair_transition/joint.rs",
        "            || source_version != plan.source_version\n", "",
        "epoch::repair_transition::joint::tests::joint_commit_fences_gate_races_even_after_fingerprint_comparison_and_on_retry",
        "source fingerprint mismatch must refuse",
    ),
    (
        "CORE006-journal-version", "crates/catcoms-replication/src/epoch/repair_transition/joint.rs",
        "            || current_journal.encode() != plan.journal.encode()\n", "",
        "registry_epoch::repair::tests::joint_repair_delayed_plan_rejects_source_journal_and_authority_changes",
        "stale joint plan must refuse",
    ),
    (
        "C1-complete-gate-stamp", "crates/catcoms-replication/src/epoch/repair_transition.rs",
        "*inner != *expected || inner.phase == EpochPhase::Settled", "inner.phase == EpochPhase::Settled",
        "epoch::repair_transition::tests::repair_commit_checks_every_gate_field_and_never_runs_callback_on_failure",
        "assertion failed:",
    ),
    (
        "C5-covered-live-head", "crates/catcoms-replication/src/epoch/repair_transition.rs",
        "if book.latest().is_some_and(|r| resolved.covers(r)) {", "if false {",
        "registry_epoch::repair::tests::repair_restore_rejects_covered_live_heads_and_screened_openings",
        "accepted covered live role",
    ),
    (
        "C5-retargeted-mode", "crates/catcoms-replication/src/epoch/repair_transition.rs",
        "if self.disposition == RepairDisposition::Retargeted && !adopting && opening.is_none() {", "if false {",
        "registry_epoch::repair::tests::repair_restore_rejects_retargeted_without_adoption_or_installed_opening",
        "assertion failed:",
    ),
    (
        "C6-pending-continuation", "crates/catcoms-replication/src/epoch/repair_transition.rs",
        "if !self.state(book, phase, opening, adopting)?.install_pending {", "if true {",
        "registry_epoch::repair::tests::repair_pending_continuation_fences_competing_public_adoption_and_seal",
        "assertion failed:",
    ),
    (
        "C5-registry-protocol-growth", "crates/catcoms-replication/src/registry_epoch.rs",
        "let repair_binding = if self.repair_binding.is_some() { 39 } else { 0 };", "let repair_binding = 0;",
        "registry_epoch::repair::tests::repair_protocol_allowance_matches_actual_snapshot_growth",
        "assertion `left == right` failed",
    ),
    (
        "C6-screened-pending", "crates/catcoms-replication/src/epoch/repair_transition.rs",
        "install_pending: self.disposition != RepairDisposition::Screened\n                && adopting",
        "install_pending: adopting",
        "registry_epoch::repair::tests::repair_screening_does_not_claim_ordinary_adoption_or_replace_unrelated_fault",
        "assertion failed:",
    ),
    (
        "C7-active-historical-publication", "crates/catcoms-replication/src/epoch/owner_journal.rs",
        "            && self\n"
        "                .in_flight\n"
        "                .as_ref()\n"
        "                .is_none_or(|r| r.hash() != receipt_hash)\n", "",
        "epoch::owner_journal::tests::reselected_historical_publication_completes_its_new_pending_obligation",
        "active reselected publication must complete even when its hash equals historical high-water",
    ),
    (
        "C7-authority-before-retry", "crates/catcoms-replication/src/epoch/owner_journal.rs",
        "        repair.verify_current_owner(group, issuer_tenure_start_group_epoch)?;\n", "",
        "epoch::owner_journal::tests::live_authority_and_complete_evidence_precede_noop_and_exact_retry",
        "operation must refuse",
    ),
    (
        "C7-retired-close-signature", "crates/catcoms-replication/src/epoch/owner_journal.rs",
        "    if !verify_with_public_bytes(\n"
        "        &close.author_public_key,\n"
        "        &close.signature_hash(),\n"
        "        &close.signature,\n"
        "    ) {\n"
        "        return Err(ReplError::EpochAuthority);\n"
        "    }\n", "",
        "epoch::owner_journal::tests::pending_retirement_requires_exact_canonical_signed_close_before_mutation",
        "operation must refuse",
    ),
    (
        "C7-pending-publication-precedence", "crates/catcoms-replication/src/epoch/owner_journal.rs",
        "            .in_flight\n            .as_ref()\n            .or(self.reconciled.as_ref())",
        "            .reconciled\n            .as_ref()\n            .or(self.in_flight.as_ref())",
        "epoch::owner_journal::tests::pending_successor_outranks_reconciliation_and_leaves_only_one_step_evidence",
        "operation must refuse",
    ),
    (
        "C7-ordinary-inheritance", "crates/catcoms-replication/src/epoch/owner_journal.rs",
        "        && TenureSelection::from(base) == TenureSelection::from(next)\n", "",
        "epoch::owner_journal::tests::ordinary_prepare_cannot_change_inheritance_or_skip_adjacency",
        "operation must refuse",
    ),
    (
        "C8-headless-predecessor", "crates/catcoms-replication/src/epoch.rs",
        "        if latest.is_none() && previous_until_installed.is_some() {\n"
        "            return Err(ReplError::Malformed);\n"
        "        }\n",
        "",
        "repair_headless_book_rejects_a_same_document_predecessor",
        "a headless repaired book must reject a same-document predecessor",
    ),
    (
        "M5-exact-pair", "crates/catcoms-replication/src/epoch/repair_state.rs",
        "|| self.receipt_hashes != hashes", "|| false",
        "repair_evidence_binds_the_exact_pair_even_when_it_shares_the_winner",
        "repair evidence accepted a different pair sharing the winner",
    ),
    (
        "C3-admission-anchor", "crates/catcoms-replication/src/epoch/adoption.rs",
        "receipts_conflict(prior, &receipt) && !self.is_repaired_loser(prior)",
        "receipts_conflict(prior, &receipt)", ANCHOR_TEST,
        "the retained loser must not re-fault the selected checkpoint",
    ),
    (
        "C3-previous-restart-anchor", "crates/catcoms-replication/src/epoch/adoption.rs",
        "self.previous_until_installed.as_ref().is_none_or(|prior| {\n"
        "                        prior.tenure_id != latest.tenure_id\n"
        "                            || self.is_repaired_loser(prior)",
        "self.previous_until_installed.as_ref().is_none_or(|prior| {\n"
        "                        prior.tenure_id != latest.tenure_id",
        ANCHOR_TEST, "a repaired losing anchor must remain restorable",
    ),
    (
        "C3-opening-restart-anchor", "crates/catcoms-replication/src/epoch/adoption.rs",
        "opening.is_none_or(|prior| {\n"
        "                        prior.tenure_id != latest.tenure_id\n"
        "                            || self.is_repaired_loser(prior)",
        "opening.is_none_or(|prior| {\n"
        "                        prior.tenure_id != latest.tenure_id",
        ANCHOR_TEST, "a repaired losing anchor must remain restorable",
    ),
    (
        "C3-admission-descendant", "crates/catcoms-replication/src/epoch/adoption.rs",
        "receipts_conflict(prior, &receipt) && !self.is_repaired_loser(prior)",
        "receipts_conflict(prior, &receipt) && !self.resolved_repair.as_ref()"
        ".is_some_and(|r| r.losing.hash() == prior.hash())", ANCHOR_TEST,
        "the retained loser must not re-fault the selected checkpoint",
    ),
    (
        "C3-previous-restart-descendant", "crates/catcoms-replication/src/epoch/adoption.rs",
        "self.previous_until_installed.as_ref().is_none_or(|prior| {\n"
        "                        prior.tenure_id != latest.tenure_id\n"
        "                            || self.is_repaired_loser(prior)",
        "self.previous_until_installed.as_ref().is_none_or(|prior| {\n"
        "                        prior.tenure_id != latest.tenure_id\n"
        "                            || self.resolved_repair.as_ref()"
        ".is_some_and(|r| r.losing.hash() == prior.hash())",
        ANCHOR_TEST, "a repaired losing anchor must remain restorable",
    ),
    (
        "C3-opening-restart-descendant", "crates/catcoms-replication/src/epoch/adoption.rs",
        "opening.is_none_or(|prior| {\n"
        "                        prior.tenure_id != latest.tenure_id\n"
        "                            || self.is_repaired_loser(prior)",
        "opening.is_none_or(|prior| {\n"
        "                        prior.tenure_id != latest.tenure_id\n"
        "                            || self.resolved_repair.as_ref()"
        ".is_some_and(|r| r.losing.hash() == prior.hash())",
        ANCHOR_TEST, "a repaired losing anchor must remain restorable",
    ),
    (
        "C8-headless-identity", "crates/catcoms-replication/src/epoch.rs",
        ".map(|receipt| receipt.document.clone())\n"
        "            .or_else(|| resolved_repair.as_ref().map(|r| r.repair.document.clone()))",
        ".map(|receipt| receipt.document.clone())",
        "repair_headless_book_roundtrips_and_retains_its_evidence_identity",
        "headless repair book must restore",
    ),
]


def run(test):
    return subprocess.run(
        COMMAND + [test if "::" in test else PREFIX + test, "--", "--exact", "--nocapture"],
        cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
        text=True, encoding="utf-8", errors="replace", timeout=900, check=False,
    )


def main():
    if not (ROOT / ".git").is_file() and os.environ.get("GITHUB_ACTIONS") != "true":
        raise RuntimeError("mutations require an isolated worktree or disposable Actions checkout")
    log_dir = ROOT / "logs" / "agent3-core-mutations"
    log_dir.mkdir(parents=True, exist_ok=True)
    for name, path, before, after, test, assertion in MUTATIONS:
        source = ROOT / path
        original = source.read_bytes()
        # Preserve the file's exact newline convention as well as its complete bytes.
        newline = b"\r\n" if b"\r\n" in original else b"\n"
        before = before.encode().replace(b"\n", newline)
        after = after.encode().replace(b"\n", newline)
        if original.count(before) != 1:
            raise AssertionError(f"mutation anchor is not unique: {name}")
        changed = original.replace(before, after, 1)
        try:
            source.write_bytes(changed)
            result = run(test)
            (log_dir / f"{name}-mutant.log").write_text(result.stdout, encoding="utf-8")
            if not (result.returncode != 0 and assertion in result.stdout
                    and "test result: FAILED. 0 passed; 1 failed;" in result.stdout):
                print(result.stdout, flush=True)
                raise AssertionError(f"mutation did not fail at its intended assertion: {name}")
            print(f"DETECTED {name}: {assertion}", flush=True)
        finally:
            if source.read_bytes() != changed:
                raise AssertionError(f"source changed concurrently; refusing overwrite: {path}")
            source.write_bytes(original)
            assert source.read_bytes() == original
            print(f"RESTORED {path} byte-for-byte", flush=True)
        # Recheck after each restoration, not just at the end: each detected failure must have
        # its own passing control before another mutation can touch the sources.
        restored = run(test)
        (log_dir / f"{name}-restored.log").write_text(restored.stdout, encoding="utf-8")
        if restored.returncode != 0 or "test result: ok. 1 passed; 0 failed;" not in restored.stdout:
            print(restored.stdout, flush=True)
            raise AssertionError(f"restored regression failed: {name}")
        print(f"PASS restored {name}", flush=True)


if __name__ == "__main__":
    main()
