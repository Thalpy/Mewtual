"""Exact native acceptance regressions; no full-suite or physical audio claim.

The seven CASES scenarios exercise the separate native workspace with actual
Node/WebCrypto, TCP and native row/custody paths. The ledger, ManualClock deadline
and command-ACL cohorts are separate unit/command-contract evidence. --self-test
invokes no Cargo. The complete native suite remains required.
"""
from __future__ import annotations

import argparse
from pathlib import Path
import runpy

REPO = Path(__file__).resolve().parents[2]
CASES = (
    "native_v2_pc_intent_close_and_sdk_release_order_actual_queued_and_returned_actor_work",
    "native_browser_local_discovery_refusal_selects_other_peer_and_preserves_indexed_sibling",
    "native_v2_pc_intent_original_sdp_worker_retries_local_refusal_and_collects_after_waiter_loss",
    "native_v2_original_sdp_result_uses_actual_actor_and_survives_eager_cancellation_lock_and_history_retries",
    "native_browser_original_round_retries_lost_http_results_applied_and_return",
    "native_browser_failed_media_peer_preserves_original_session_and_indexed_sibling",
    "native_browser_media_retires_original_negotiation_and_keeps_indexed_peer_usable",
)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    guard = runpy.run_path(str(REPO / ".github/scripts/check-chat-v2-member-save.py"))
    guard["self_test"]()
    assert len(set(CASES)) == 7
    if args.self_test:
        return
    directory = REPO / "logs/chat-v2-native-gates"
    directory.mkdir(parents=True, exist_ok=True)
    # These two genuine ledger regressions pin missing-key selector retention,
    # original-child filtering and actual child-mutex contention. Compile-only
    # coverage would not prove that the snapshot remains nonblocking.
    code, summaries, timed_out, _ = guard["run_logged"](
        "original-scope-diagnostics",
        ["cargo", "test", "--locked", "--manifest-path", "apps/desktop/src-tauri/Cargo.toml",
         "--lib", "sdp_failure_diagnostics_", "--", "--nocapture"],
        directory, limit=45 * 60,
    )
    if timed_out:
        raise RuntimeError("original-scope-diagnostics: native cohort exceeded its process bound")
    guard["validate"]("".join(summaries), code, 2)
    for name in CASES:
        selector = "six_client_recovery::voice_live::" + name
        code, summaries, timed_out, _ = guard["run_logged"](
            name,
            ["cargo", "test", "--locked", "--manifest-path", "apps/desktop/src-tauri/Cargo.toml",
             "--lib", selector, "--", "--exact", "--nocapture"],
            directory, limit=45 * 60,
        )
        if timed_out:
            raise RuntimeError(f"{name}: native cohort exceeded its process bound")
        guard["validate"]("".join(summaries), code, 1)

    # A real refused native completion sleeps on ManualClock, then its original
    # issuer closes at the fixed bound. Re-dispatch would produce Closed; the
    # correct helper returns the original Capacity without starting new work.
    code, summaries, timed_out, _ = guard["run_logged"](
        "original-discovery-deadline",
        ["cargo", "test", "--locked", "--manifest-path", "apps/desktop/src-tauri/Cargo.toml",
         "--lib", "voice_handoff::service::maintenance::tests::original_discovery_retry_does_not_dispatch_after_sleep_reaches_its_bound",
         "--", "--exact", "--nocapture"],
        directory, limit=45 * 60,
    )
    if timed_out:
        raise RuntimeError("original-discovery-deadline: native cohort exceeded its process bound")
    guard["validate"]("".join(summaries), code, 1)

    # This is a separate integration-test binary: --lib would silently select
    # the wrong target. Require all six real Tauri command-policy controls.
    code, summaries, timed_out, _ = guard["run_logged"](
        "command-acl",
        ["cargo", "test", "--locked", "--manifest-path", "apps/desktop/src-tauri/Cargo.toml",
         "--test", "command_acl", "--", "--nocapture"],
        directory, limit=45 * 60,
    )
    if timed_out:
        raise RuntimeError("command-acl: native cohort exceeded its process bound")
    guard["validate"]("".join(summaries), code, 6)


if __name__ == "__main__":
    main()
