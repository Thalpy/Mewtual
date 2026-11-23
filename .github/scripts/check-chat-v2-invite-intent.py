"""Hosted invitation cohorts with exact nonignored counts and retained logs."""
from pathlib import Path
import json
import os
import runpy
import sys
import tempfile

REPO = Path(__file__).resolve().parents[2]
FENCE_PREDECESSOR = "3d3bce638fe6736f9073bd07be1cf89465766b83"
FENCE_CASE = "governed_add_fence_preserves_old_wal_authentication_and_refusal"
FENCE_CONTROL = "crates/catcoms-app/src/store/persistence/v2_text/control/tests.rs"
FENCE_MEMBER = (
    "crates/catcoms-app/src/store/persistence/cold_source_read/genesis_seed/"
    "current/tests/text_group_guard/host/member.rs"
)

COHORTS = {
    "bindings": ("catcoms-replication", "chat::join_bindings::tests::", 15),
    "journal": ("catcoms-app", "unsigned_invite_journal_", 4),
    "add-fence": ("catcoms-app", "governed_add_fence_", 5),
    "add-request-record": ("catcoms-app", "governed_add_request_record_", 1),
    "add-sdk-entry": ("catcoms-app", "governed_add_sdk_entry_", 1),
    "add-sdk-budget": ("catcoms-app", "governed_add_sdk_budget_", 1),
    "add-package-preflight": ("catcoms-sync", "expired_actual_package_refuses_with_a_still_valid_signed_invite", 1),
    "add-outcome": ("catcoms-app", "governed_add_outcome_", 4),
    "add-header-signing": ("catcoms-app", "governed_add_header_signing_", 1),
    "add-welcome-intent": ("catcoms-app", "governed_add_welcome_intent_", 1),
    "add-welcome-signing": ("catcoms-app", "governed_add_welcome_signing_", 1),
    "add-response": ("catcoms-app", "governed_add_response_", 1),
    "add-archive": ("catcoms-app", "governed_add_archive_", 1),
    "add-head2": ("catcoms-app", "governed_add_head2_", 1),
    "joined-pin": ("catcoms-app", "governed_joined_pin_", 9),
    "joined-native-exchange": (
        "catcoms-app",
        "native_runtime_invite_returns_only_the_original_installed_join_response",
        3,
    ),
    "joined-wire": ("catcoms-net", "new_service_rejects_oversized_headers_before_body_read_or_allocation", 1),
    "joined-wire-ingress": ("catcoms-net", "native_join_transport_dispatch_preserves_original_epoch_and_ordinary_stream", 1),
    "joined-contact": ("catcoms-app", "joined_contact_refuses_a_queued_dial_after_its_invocation_retires", 1),
    "mounted-join-request": ("catcoms-app", "mounted_join_request_", 2),
    "joined-registry-codec": ("catcoms-app", "native_birth_discriminant_preserves_all_existing_encodings_and_uses_four", 1),
    "member-outbox-admission": ("catcoms-app", "admission_must_name_the_exact_prepared_facts_and_returns_everything_when_refused", 1),
    "founder-member-save": ("catcoms-app", "native_runtime_member_prepare_uses_actual_native_signer_and_exact_retry_never_resigns", 1),
    "member-envelope": ("catcoms-sync", "member_envelope_uses_the_real_mls_group_and_exact_signed_original", 1),
    "member-publication-driver": ("catcoms-net", "captured_publication_checks_queued_authority_and_never_adopts_a_replacement_driver", 1),
    "member-receive-dependencies": ("catcoms-replication", "ordinary_receiver_waits_for_exact_valid_dependencies_then_projects", 1),
    "member-receive-native": ("catcoms-net", "native_member_gossip_uses_one_original_topic_and_revokes_handed_off_frames", 1),
    "member-transport-selection": ("catcoms-sync", "stale_join_and_same_epoch_mismatch_cannot_replace_a_consumed_newer_selection", 1),
    "member-shared-history-record": ("catcoms-app", "shared_history_reopens_exact_signed_chunks_and_refuses_foreign_manifests", 1),
    "member-shared-history-disk": ("catcoms-app", "shared_history_disk_", 2),
    "member-shared-history-declaration": ("catcoms-app", "shared_history_declaration_only_adds_eight_and_predecessor_refuses", 1),
    "member-shared-history-seed": ("catcoms-app", "shared_history_seed_retains_exact_catalogs_marker_and_pin_index", 1),
    "member-shared-history-stage": ("catcoms-app", "shared_history_physical_", 4),
    "member-shared-history-replay": ("catcoms-replication", "ordinary_history_replay_restores_dependencies_without_publishing_removed_originals", 1),
    "member-action-history": ("catcoms-app", "member_history_keeps_settled_originals_and_a_fixed_prefix_across_appends_and_reopen", 1),
    "member-dependency-transport": ("catcoms-net", "native_member_dependency_exchange_keeps_the_original_connected_lane", 1),
    "selected-history": ("catcoms-replication", "selected_history_archive_page_", 2),
    "add-source-close": ("catcoms-sync", "governed_add_whole_source_retirement_", 1),
    "add-ui-lock": ("catcoms-sync", "governed_add_ui_retirement_", 1),
    "source-retirement": ("catcoms-app", "operational::sources::retirement::tests::", 11),
    "control-scratch": ("catcoms-app", "v2_text::control::tests::scratch::", 3),
    "warm-member-fence": ("catcoms-app", "a_crash_left_founder_wal_keeps_the_member_lane_closed_until_it_is_gone", 1),
    "native": ("catcoms-app", "native_runtime_invite_", 19),
    "packet": ("catcoms-app", "native_invite_packet_", 3),
    "join-request": ("catcoms-sync", "prepared_direct_join::v2::tests::", 3),
    "joined-pending": ("catcoms-sync", "v2_pending_join_keeps_the_original_", 1),
    "join-response": ("catcoms-sync", "prepared_direct_join::v2::welcome::tests::", 1),
    "join-welcome": ("catcoms-sync", "governance::tests::welcome::", 7),
    "native-joiner-key": ("catcoms-sync", "native_joiner_welcome_requires_the_original_device_source_and_sdk_cut", 1),
    "governed-header": ("catcoms-sync", "raw_header_survives_late_lock_and_exact_retry_never_reenters_key", 1),
    "interrupted-header": ("catcoms-sync", "panic_after_header_entry_does_not_repeat_key_or_lose_completed_sdk_result", 1),
    "predecessor": (
        "catcoms-app",
        "invite_journal_survives_predecessor_save_and_physical_retirement",
        1,
    ),
}


def fenced_predecessor(guard: dict, directory: Path) -> None:
    git = guard["git"]
    results = []

    def execute(name: str, selector: str, count: int, cwd: Path, env: dict) -> None:
        code, summaries, timed_out, _ = guard["run_logged"](
            name, ["cargo", "test", "--locked", "-p", "catcoms-app", "--lib", selector,
                   "--", "--test-threads=1"], directory, cwd=cwd, env=env,
            limit=guard["PREDECESSOR_TIMEOUT_SECONDS"],
        )
        results.append({"name": name, "source": git("rev-parse", "HEAD", cwd=cwd),
                        "exit": code, "timed_out": timed_out})
        if timed_out:
            raise RuntimeError(f"{name} exceeded its hosted compile/execution bound")
        guard["validate"]("".join(summaries), code, count)

    try:
        with tempfile.TemporaryDirectory(prefix="a2-", dir=os.environ.get("RUNNER_TEMP")) as temp:
            original = Path(temp).resolve()
            fixture = original / "fence.bin"
            env = dict(os.environ, MEWTUAL_GOVERNED_ADD_FENCE_FIXTURE=str(fixture),
                       CARGO_TARGET_DIR=os.environ.get("CARGO_TARGET_DIR", str(REPO / "target")))
            execute("add-fence-fixture", FENCE_CASE, 1, REPO, env)
            if not fixture.is_file() or fixture.is_symlink() or not 40 < fixture.stat().st_size <= 512:
                raise RuntimeError("missing or invalid head-generated Add fence")
            exact = fixture.read_bytes()
            old = original / "p"
            if not old.resolve().is_relative_to(original):
                raise RuntimeError("predecessor escaped the owned temporary directory")
            fetch = ["fetch", "--no-tags"]
            if git("rev-parse", "--is-shallow-repository") == "true":
                fetch.append("--depth=1")
            git(*fetch, "origin", FENCE_PREDECESSOR)
            git("worktree", "add", "--detach", str(old), FENCE_PREDECESSOR)

            def remove_old() -> None:
                if old.resolve().parent != original:
                    raise RuntimeError("predecessor escaped its owned directory before cleanup")
                git("worktree", "remove", "--force", str(old))

            try:
                if (git("rev-parse", "HEAD", cwd=old) != FENCE_PREDECESSOR
                        or git("status", "--porcelain", "--untracked-files=all", cwd=old)):
                    raise RuntimeError("expected the pristine pinned predecessor")
                if (old / "Cargo.lock").read_bytes() != (REPO / "Cargo.lock").read_bytes():
                    raise RuntimeError("predecessor and head dependency locks differ")
                harnesses = {FENCE_CONTROL: "governed_add_fence_control.rs"}
                if sys.platform == "win32":
                    harnesses[FENCE_MEMBER] = "governed_add_fence_owner.rs"
                expected = {}
                for target, harness in harnesses.items():
                    path = old / target
                    expected[path] = path.read_bytes() + b"\n" + (
                        REPO / ".github/predecessor" / harness
                    ).read_bytes()
                    path.write_bytes(expected[path])

                def require_harness_only() -> None:
                    changed = git("status", "--porcelain", "--untracked-files=all", cwd=old)
                    if ({line.strip() for line in changed.splitlines()}
                            != {f"M {target}" for target in harnesses}
                            or any(path.read_bytes() != data for path, data in expected.items())):
                        raise RuntimeError("predecessor changed beyond its appended test harnesses")

                require_harness_only()
                guard["clean_source"]("add-fence-clean-old", old, env, directory, results)
                execute("add-fence-predecessor", "governed_add_old_reader_", len(harnesses), old, env)
                require_harness_only()
                if fixture.read_bytes() != exact:
                    raise RuntimeError("predecessor changed the head-generated fixture")
            finally:
                guard["retire_old_source"](
                    remove_old,
                    lambda: guard["clean_source"]("add-fence-clean-head", REPO, env, directory, results),
                    results, sys.exception(),
                )
    finally:
        (directory / "add-fence-predecessor.json").write_text(
            json.dumps(results, indent=2) + "\n", encoding="utf-8",
        )


def main() -> None:
    options = (*COHORTS, "add-fence-predecessor")
    if len(sys.argv) != 2 or sys.argv[1] not in options:
        raise SystemExit("choose " + ", ".join(options))
    name = sys.argv[1]
    guard = runpy.run_path(str(Path(__file__).with_name("check-chat-v2-member-save.py")))
    directory = REPO / "logs/chat-v2-invite-intent"
    directory.mkdir(parents=True, exist_ok=True)
    if name == "add-fence-predecessor":
        fenced_predecessor(guard, directory)
        return
    package, selector, expected = COHORTS[name]
    code, summaries, timed_out, _ = guard["run_logged"](
        name,
        ["cargo", "test", "--locked", "-p", package, "--lib", selector, "--", "--test-threads=1"],
        directory,
        limit=60 * 60,
    )
    if timed_out:
        raise RuntimeError(f"{name} exceeded its hosted compile/execution bound")
    guard["validate"]("".join(summaries), code, expected)


if __name__ == "__main__":
    main()
