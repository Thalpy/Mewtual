#!/usr/bin/env bash
# Forbid raw filesystem mutation in the app store outside its `persistence` module.
#
# I-4 (docs/GATE4-AGENT-1-DESIGN.md 9.2) requires every write, rename, unlink or sync of an
# inventoried epoch file to rotate `inventory_generation` first. The guarded primitives make that a
# compile error for `atomic_write`, staging files and directory syncs, but nothing stops a plain
# `std::fs` call from being written beside them, and since C-3 keeps inventory cursors open across
# visits such a call would let a budget be minted from an inventory that missed the write (I-4
# audit M-1). This gate closes that gap mechanically: in non-test code under
# `crates/catcoms-app/src/store*`, raw mutation is allowed only inside `mod persistence` and at the
# reviewed sites listed below.
set -euo pipefail

cd "$(dirname "$0")/.."

# A sync counts as a mutation under I-4 (an unchanged-file flush still invalidates a captured
# inventory), so explicit syncs are matched too, wherever the handle came from.
#
# The open flags that can change a file are matched on their own lines too. An allowance keys only
# the line that matched, and an `OpenOptions` chain spans several, so without these a reviewed
# sync-only open could gain `.truncate(true)` or `.create(true)` unseen (implementation review of
# the 18.3 fixes, LOW-5). `.write(true)` alone is not matched: a sync needs a writable handle.
pattern='fs::(write|rename|remove_file|remove_dir|remove_dir_all|copy|hard_link|create_dir|create_dir_all|set_permissions)\(|File::(create|options)\(|OpenOptions::new\(\)|\.set_len\(|\.sync_(all|data)\(|\.(truncate|create|create_new|append)\(true\)'

# Reviewed sites, each `file|enclosing fn|the matching line's own text, trimmed`, and each allowed
# exactly once. Extend deliberately, with review, and say why next to the entry.
#
# Anchored the way the mutation harnesses anchor their bytes (design 18.3 review, F6). A per-file
# count, which this used to be, passes when a reviewed site is replaced by a different raw
# mutation in the same file, since the count is unchanged. With function and text in the key, a
# reviewed line moved within its function still passes; a changed line, a copy of it, or the same
# line in another function does not. "Enclosing fn" is the nearest `fn` declared above the hit, so
# a hit in an initializer after a function's body is charged to that function: never to an
# allowance, unless its text is also exactly a reviewed line of that function.
declare -A allowed=(
  # Private namespace7 declaration, not epoch-family mutation. Only the original startup
  # mount and cached authenticated exact catalog/index/marker/seed can reach these fixed
  # destinations. Claim -> durable old-reader fence -> complete seed -> covering outputs ->
  # catalog selection -> late tuple proofs -> cleanup. No SDK/native session is granted.
  ['crates/catcoms-app/src/store/persistence/native_registry.rs|capture_writable|let file = OpenOptions::new()']=1
  ['crates/catcoms-app/src/store/persistence/native_registry.rs|claim|let file = OpenOptions::new()']=1
  ['crates/catcoms-app/src/store/persistence/native_registry.rs|claim|.create_new(true)']=1
  ['crates/catcoms-app/src/store/persistence/native_registry.rs|claim|value.file.sync_all().map_err(io)?;']=1
  ['crates/catcoms-app/src/store/persistence/native_registry.rs|complete|value.file.sync_all().map_err(io)?;']=1
  ['crates/catcoms-app/src/store/persistence/native_registry.rs|write|inputs.vault.file.sync_all().map_err(io)?;']=7
  ['crates/catcoms-app/src/store/persistence/native_registry.rs|write|inputs.root.file.sync_all().map_err(io)?;']=3
  ['crates/catcoms-app/src/store/persistence/native_registry.rs|write|fs::rename(&stage, &current).map_err(io)?;']=1
  ['crates/catcoms-app/src/store/persistence/native_registry.rs|write|selected.sync_all().map_err(io)?;']=1
  ['crates/catcoms-app/src/store/persistence/native_registry.rs|write|value.file.sync_all().map_err(io)?;']=1
  ['crates/catcoms-app/src/store/persistence/native_registry.rs|write|fs::remove_file(&stage).map_err(io)?;']=1
  ['crates/catcoms-app/src/store/persistence/native_registry.rs|write|fs::remove_file(&seed).map_err(io)?;']=1
  # Only an original held unauthenticated first seed and its classified EMPTY fence, with
  # authenticated pre-ID7 current and no index/marker. Never an authenticated future seed.
  ['crates/catcoms-app/src/store/persistence/native_registry.rs|dispose_native_first_torn|fs::remove_file(&stage_path).map_err(io)?;']=1
  ['crates/catcoms-app/src/store/persistence/native_registry.rs|dispose_native_first_torn|fs::remove_file(&seed_path).map_err(io)?;']=1
  ['crates/catcoms-app/src/store/persistence/native_registry.rs|dispose_native_first_torn|inputs.vault.file.sync_all().map_err(io)?;']=2
  # Explicit namespace7/v2 fixed metadata, outside inventoried epoch files. The
  # original paid owner/cold role authenticates exact catalog/index/marker/seed
  # relations and retains actual File identities. No row supplies a path. Each
  # destination is re-proved after large reads before rename; selection/unlink
  # phases are cached before covering barriers. These are not epoch-file permits.
  ['crates/catcoms-app/src/store/persistence/native_registry/operational.rs|abandon_first_torn|fs::remove_file(&seed_path).map_err(io)?;']=1
  ['crates/catcoms-app/src/store/persistence/native_registry/operational.rs|abandon_first_torn|fs::remove_file(&stage_path).map_err(io)?;']=1
  ['crates/catcoms-app/src/store/persistence/native_registry/operational.rs|abandon_first_torn|self.base.vault.file.sync_all().map_err(io)?;']=2
  ['crates/catcoms-app/src/store/persistence/native_registry/operational.rs|advance_upgrade|fs::remove_file(&seed).map_err(io)?;']=1
  ['crates/catcoms-app/src/store/persistence/native_registry/operational.rs|advance_upgrade|fs::rename(&index_temp, &index).map_err(io)?;']=1
  ['crates/catcoms-app/src/store/persistence/native_registry/operational.rs|advance_upgrade|fs::rename(&marker_temp, &marker).map_err(io)?;']=1
  ['crates/catcoms-app/src/store/persistence/native_registry/operational.rs|advance_upgrade|fs::rename(&stage, &current).map_err(io)?;']=1
  ['crates/catcoms-app/src/store/persistence/native_registry/operational.rs|advance_upgrade|selected.file.sync_all().map_err(io)?;']=1
  # Repeat exact output/root coverage before catalog content on warm AND cold
  # retries. These fixed non-epoch roles retain the original authenticated tuple.
  ['crates/catcoms-app/src/store/persistence/native_registry/operational.rs|cover_selected_outputs_before_catalog|output.file.sync_all().map_err(io)?;']=1
  ['crates/catcoms-app/src/store/persistence/native_registry/operational.rs|cover_selected_outputs_before_catalog|self.original.root.file.sync_all().map_err(io)?;']=1
  ['crates/catcoms-app/src/store/persistence/native_registry/operational.rs|advance_upgrade|self.original.root.file.sync_all().map_err(io)?;']=5
  ['crates/catcoms-app/src/store/persistence/native_registry/operational.rs|advance_upgrade|self.original.vault.file.sync_all().map_err(io)?;']=6
  ['crates/catcoms-app/src/store/persistence/native_registry/operational.rs|commit_index|fs::rename(&stage_path, &index_path).map_err(io)?;']=1
  ['crates/catcoms-app/src/store/persistence/native_registry/operational.rs|commit_index|selected.file.sync_all().map_err(io)?;']=1
  ['crates/catcoms-app/src/store/persistence/native_registry/operational.rs|commit_index|self.original.root.file.sync_all().map_err(io)?;']=2
  ['crates/catcoms-app/src/store/persistence/native_registry/operational.rs|run|fs::remove_file(&stage_path).map_err(io)?;']=1
  ['crates/catcoms-app/src/store/persistence/native_registry/operational.rs|run|self.original.index.file.sync_all().map_err(io)?;']=1
  ['crates/catcoms-app/src/store/persistence/native_registry/operational.rs|run|self.original.root.file.sync_all().map_err(io)?;']=2
  # Namespace4 declaration first creates/syncs the SAME empty legacy stage so
  # actual older readers refuse before any governance-only recovery debt exists.
  ['crates/catcoms-app/src/store/persistence/governance_layout.rs|ensure_legacy_stage_fence|let file = OpenOptions::new()']=1
  ['crates/catcoms-app/src/store/persistence/governance_layout.rs|ensure_legacy_stage_fence|.create_new(true)']=1
  ['crates/catcoms-app/src/store/persistence/governance_layout.rs|ensure_legacy_stage_fence|original.file.sync_all().map_err(io)?;']=1
  # Private offline Current4 metadata declaration only. Complete authenticated TWO-frame
  # seed and exact original native objects precede promotion; no arbitrary path, Source,
  # SDK/key entry or epoch-family effect. Preserve partial/foreign evidence on refusal.
  ['crates/catcoms-app/src/store/persistence/governance_layout.rs|ensure|let newly_opened = OpenOptions::new()']=1
  ['crates/catcoms-app/src/store/persistence/governance_layout.rs|ensure|.create_new(true)']=1
  ['crates/catcoms-app/src/store/persistence/governance_layout.rs|ensure|let existing = OpenOptions::new()']=1
  ['crates/catcoms-app/src/store/persistence/governance_layout.rs|ensure|slot.file.sync_all().map_err(io)?;']=1
  ['crates/catcoms-app/src/store/persistence/governance_layout.rs|sync_marker_parent|root.directory.sync_all().map_err(io)']=1
  ['crates/catcoms-app/src/store/persistence/governance_layout.rs|verify_seed|file.sync_all().map_err(io)?;']=1
  ['crates/catcoms-app/src/store/persistence/governance_layout.rs|write_governance_layout_declaration|actual.sync_all().map_err(io)?;']=1
  # Separate original selection-stage sync and exact captured cold-duplicate sync.
  ['crates/catcoms-app/src/store/persistence/governance_layout.rs|write_governance_layout_declaration|stage.file.sync_all().map_err(io)?;']=2
  ['crates/catcoms-app/src/store/persistence/governance_layout.rs|write_governance_layout_declaration|fs::rename(&stage_path, &current_path).map_err(io)?;']=1
  ['crates/catcoms-app/src/store/persistence/governance_layout.rs|write_governance_layout_declaration|fs::remove_file(&stage_path).map_err(io)?;']=1
  ['crates/catcoms-app/src/store/persistence/governance_layout.rs|write_governance_layout_declaration|current.sync_all().map_err(io)?;']=1
  ['crates/catcoms-app/src/store/persistence/governance_layout.rs|write_governance_layout_declaration|selected.sync_all().map_err(io)?;']=1
  ['crates/catcoms-app/src/store/persistence/governance_layout.rs|write_governance_layout_declaration|fs::remove_file(&seed_path).map_err(io)?;']=1
  ['crates/catcoms-app/src/store/persistence/governance_layout.rs|install_governance_layout_marker|original.marker.sync_all().map_err(io)?;']=1
  # Namespace5 (member outbox layout) declaration: the same reviewed protocol as namespace4,
  # with its own fixed seed/marker names. A zero-byte claim of the fixed seed name comes
  # first (no content, so older readers may ignore it), then the empty legacy stage fence
  # precedes every content-bearing member-only output so actual older readers refuse first.
  ['crates/catcoms-app/src/store/persistence/member_outbox_layout.rs|ensure_seed_claim|let file = OpenOptions::new()']=1
  ['crates/catcoms-app/src/store/persistence/member_outbox_layout.rs|ensure_seed_claim|.create_new(true)']=1
  ['crates/catcoms-app/src/store/persistence/member_outbox_layout.rs|ensure_seed_claim|claim.file.sync_all().map_err(io)?;']=1
  # Cold first-torn disposal: only the classified zero-byte fence, then the classified
  # UNAUTHENTICATED seed at their fixed names, each followed by a barrier and a NotFound proof,
  # and only with no marker, parent or ID5 (docs/communication-recovery/CHAT-V2-NAMESPACE5-RECOVERY.md).
  ['crates/catcoms-app/src/store/persistence/member_outbox_layout.rs|dispose_first_torn_member_outbox_seed|fs::remove_file(&stage_path).map_err(io)?;']=1
  ['crates/catcoms-app/src/store/persistence/member_outbox_layout.rs|dispose_first_torn_member_outbox_seed|fs::remove_file(&seed_path).map_err(io)?;']=1
  ['crates/catcoms-app/src/store/persistence/member_outbox_layout.rs|ensure_legacy_stage_fence|let file = OpenOptions::new()']=1
  ['crates/catcoms-app/src/store/persistence/member_outbox_layout.rs|ensure_legacy_stage_fence|.create_new(true)']=1
  ['crates/catcoms-app/src/store/persistence/member_outbox_layout.rs|ensure_legacy_stage_fence|original.file.sync_all().map_err(io)?;']=1
  # The one fixed `chat-owner-v2/member-outbox-v1` parent, created only by an owned declaration
  # after its marker and before promotion; never re-created after the commit point.
  ['crates/catcoms-app/src/store/persistence/member_outbox_layout.rs|ensure_parent_directory|fs::create_dir(&path).map_err(io)?;']=1
  ['crates/catcoms-app/src/store/persistence/member_outbox_layout.rs|sync_parent_of_marker|root.directory.sync_all().map_err(io)']=1
  ['crates/catcoms-app/src/store/persistence/member_outbox_layout.rs|ensure|let newly_opened = OpenOptions::new()']=1
  ['crates/catcoms-app/src/store/persistence/member_outbox_layout.rs|ensure|.create_new(true)']=1
  ['crates/catcoms-app/src/store/persistence/member_outbox_layout.rs|ensure|let existing = OpenOptions::new()']=1
  ['crates/catcoms-app/src/store/persistence/member_outbox_layout.rs|ensure|slot.file.sync_all().map_err(io)?;']=1
  ['crates/catcoms-app/src/store/persistence/member_outbox_layout.rs|verify_seed|file.sync_all().map_err(io)?;']=1
  ['crates/catcoms-app/src/store/persistence/member_outbox_layout.rs|write_member_outbox_layout_declaration|actual.sync_all().map_err(io)?;']=1
  # Separate original selection-stage sync and exact captured cold-duplicate sync.
  ['crates/catcoms-app/src/store/persistence/member_outbox_layout.rs|write_member_outbox_layout_declaration|stage.file.sync_all().map_err(io)?;']=2
  ['crates/catcoms-app/src/store/persistence/member_outbox_layout.rs|write_member_outbox_layout_declaration|fs::rename(&stage_path, &current_path).map_err(io)?;']=1
  ['crates/catcoms-app/src/store/persistence/member_outbox_layout.rs|write_member_outbox_layout_declaration|fs::remove_file(&stage_path).map_err(io)?;']=1
  ['crates/catcoms-app/src/store/persistence/member_outbox_layout.rs|write_member_outbox_layout_declaration|current.sync_all().map_err(io)?;']=1
  ['crates/catcoms-app/src/store/persistence/member_outbox_layout.rs|write_member_outbox_layout_declaration|selected.sync_all().map_err(io)?;']=1
  ['crates/catcoms-app/src/store/persistence/member_outbox_layout.rs|write_member_outbox_layout_declaration|fs::remove_file(&seed_path).map_err(io)?;']=1
  ['crates/catcoms-app/src/store/persistence/member_outbox_layout.rs|install_member_outbox_layout_marker|original.marker.sync_all().map_err(io)?;']=1
  # SAME-original warm retained-A recovery only. A distinct fresh Current repair marker
  # verifies complete authenticated MAIN before extending ONLY matching listed prefixes;
  # a distinct scratch marker verifies the original strict pre-MAIN prefix, parent and
  # absence of dependents before unlinking that one fixed stage. Actual outcomes cache
  # before barriers/late errors. Every helper File drains before its actual lease/End.
  ['crates/catcoms-app/src/store/persistence/v2_text/transaction/recovery/effect.rs|scratch_inner|fs::remove_file(&p.wal_stage_path).map_err(io::io)?;']=1
  ['crates/catcoms-app/src/store/persistence/v2_text/transaction/recovery/effect.rs|complete_authenticated_output|OpenOptions::new()']=1
  ['crates/catcoms-app/src/store/persistence/v2_text/transaction/recovery/effect.rs|complete_authenticated_output|Err(error) if error.kind() == std::io::ErrorKind::NotFound => OpenOptions::new()']=1
  ['crates/catcoms-app/src/store/persistence/v2_text/transaction/recovery/effect.rs|complete_authenticated_output|.create_new(true)']=1
  ['crates/catcoms-app/src/store/persistence/v2_text/transaction/recovery/effect.rs|complete_authenticated_output|file.sync_all().map_err(io::io)?;']=1
  # Read-only held-directory capture, with explicit reparse/name/native-ID checks.
  ['crates/catcoms-app/src/store/persistence/v2_text/transaction/recovery/io.rs|capture|fs::OpenOptions::new()']=1
  # First retained-A only: the SAME consumed Current Stage/Commit marker retains its
  # actual vault lease and writer gate through all helper Files/cleanup. WAL is complete
  # and parent-synced before immutable outputs; all exact outputs precede selector CAS.
  # Existing objects are compared/resynced only; fresh writes use create_new, never
  # truncate/append/unlink. Closed names stay inside the original non-epoch text family.
  ['crates/catcoms-app/src/store/persistence/v2_text/transaction/work.rs|ensure_wal|fs::rename(&parent.wal_stage_path, &parent.wal_path).map_err(native_error)?;']=1
  ['crates/catcoms-app/src/store/persistence/v2_text/transaction/work.rs|commit_inner|fs::rename(&head_stage, &parent.head_path).map_err(native_error)?;']=1
  # Unsigned-I only: its distinct consumed Current Stage/Commit markers hold the real
  # native lease and writer end. Full original WA/WI comparisons and file/parent sync
  # barriers precede journal replacement or exact O_I selection; no signing is implied.
  ['crates/catcoms-app/src/store/persistence/v2_text/transaction/intent/work.rs|ensure_wal|fs::rename(&p.wal_stage_path, &p.wal_path).map_err(failed)?;']=1
  ['crates/catcoms-app/src/store/persistence/v2_text/transaction/intent/work.rs|commit_inner|fs::rename(&head_stage, &p.head_path).map_err(failed)?;']=1
  # Raw S consumes the real Current Stage/Commit admission and SAME whole I/SDK;
  # full original WI/WS, I/S dependencies and barriers precede these two renames.
  ['crates/catcoms-app/src/store/persistence/v2_text/transaction/intent/raw/work.rs|ensure_wal|fs::rename(&p.base.wal_stage_path, &p.base.wal_path).map_err(failed)?;']=1
  ['crates/catcoms-app/src/store/persistence/v2_text/transaction/intent/raw/work.rs|commit_inner|fs::rename(&head_stage, &p.base.head_path).map_err(failed)?;']=1
  # First accepted B only: SAME original S/Core candidate and distinct consumed
  # actual Current Stage/Commit own the native lease/End; full WS/WB and ALL exact
  # new artifacts plus file/parent barriers precede these two replacement edges.
  ['crates/catcoms-app/src/store/persistence/v2_text/transaction/intent/raw/accepted/work.rs|ensure_wal|fs::rename(&base.wal_stage_path, &base.wal_path).map_err(failed)?;']=1
  ['crates/catcoms-app/src/store/persistence/v2_text/transaction/intent/raw/accepted/work.rs|commit_inner|fs::rename(&stage_path, &base.head_path).map_err(failed)?;']=1
  # DISTINCT completed-WC readonly source holds directory handles in the real
  # same-vault Source/lease/End bundle. It performs no writes or removal.
  ['crates/catcoms-app/src/store/persistence/v2_text/completed_workspace/io.rs|capture|fs::OpenOptions::new()']=1
  # Only original NEXT-A Stage after full original WC; fixed recorded stage,
  # bounded whole prefix before suffix, no missing/adopt/truncate/new nonce.
  ['crates/catcoms-app/src/store/persistence/v2_text/transaction/work/completed_continuation.rs|write_original|let mut file = OpenOptions::new()']=1
  ['crates/catcoms-app/src/store/persistence/v2_text/transaction/work/completed_continuation.rs|write_original|file.sync_all().map_err(failed)?;']=1
  ['crates/catcoms-app/src/store/persistence/v2_text/transaction/work/completed.rs|ensure_completed_wal|std::fs::rename(&self.parent.wal_stage_path, &self.parent.wal_path)']=1
  # DISTINCT actual WD deletion / fresh WC completion, SAME exact B+Drained.
  # Full WB/WD precedes stage replacement, durable WD precedes only its exact
  # authenticated private list unlink, actual Files drain before ReadyWC/End.
  ['crates/catcoms-app/src/store/persistence/v2_text/transaction/intent/raw/accepted/retirement/work.rs|ensure_deletion_journal|fs::rename(&facts.stage_path, &facts.wal_path).map_err(failed)?;']=1
  ['crates/catcoms-app/src/store/persistence/v2_text/transaction/intent/raw/accepted/retirement/work.rs|deletion_inner|fs::remove_file(&object.path).map_err(failed)?;']=1
  ['crates/catcoms-app/src/store/persistence/v2_text/transaction/intent/raw/accepted/retirement/work.rs|completion_inner|fs::rename(&facts.stage_path, &facts.wal_path).map_err(failed)?;']=1
  # SAME-original retirement-stage prefix continuation only, never missing
  # adoption/general cold repair. Distinct actual role+full parent WAL first.
  ['crates/catcoms-app/src/store/persistence/v2_text/transaction/intent/raw/accepted/retirement/continuation.rs|write_original|let mut file = OpenOptions::new()']=1
  ['crates/catcoms-app/src/store/persistence/v2_text/transaction/intent/raw/accepted/retirement/continuation.rs|write_original|file.sync_all().map_err(failed)?;']=1
  # SAME-B recorded Stage continuation only: full originalWS beforeWB stage or
  # exactownWB beforeimmutable/head outputs; fixed original bytes, native refusal
  # filter, bounded wholeprefix +suffix/sync/name; no cold adoption/key/reseal.
  ['crates/catcoms-app/src/store/persistence/v2_text/transaction/intent/raw/accepted/warm.rs|write_original|let mut file = OpenOptions::new()']=1
  ['crates/catcoms-app/src/store/persistence/v2_text/transaction/intent/raw/accepted/warm.rs|write_original|file.sync_all().map_err(failed)?;']=1
  # SAME-I recorded Stage continuation only: actual opaque original + exact WA/own WI,
  # regular bounded exact prefix and recorded native refusal filter precede suffix-only
  # completion. Full sync/bytes/names/barriers follow; missing/foreign work is preserved.
  ['crates/catcoms-app/src/store/persistence/v2_text/transaction/intent/warm.rs|write_original|let mut file = OpenOptions::new()']=1
  ['crates/catcoms-app/src/store/persistence/v2_text/transaction/intent/warm.rs|write_original|file.sync_all().map_err(failed)?;']=1
  # SAME-S recorded Stage continuation has the same restrictions, with full original
  # WI or own WS and the original raw signature; it never reopens MAIN for appending.
  ['crates/catcoms-app/src/store/persistence/v2_text/transaction/intent/raw/warm.rs|write_original|let mut file = OpenOptions::new()']=1
  ['crates/catcoms-app/src/store/persistence/v2_text/transaction/intent/raw/warm.rs|write_original|file.sync_all().map_err(failed)?;']=1
  ['crates/catcoms-app/src/store/persistence/v2_text/transaction/work/io.rs|sync_exact|let mut writable = OpenOptions::new()']=1
  ['crates/catcoms-app/src/store/persistence/v2_text/transaction/work/io.rs|sync_exact|writable.sync_all().map_err(io)?;']=1
  ['crates/catcoms-app/src/store/persistence/v2_text/transaction/work/io.rs|write_complete_only|let mut file = OpenOptions::new()']=1
  ['crates/catcoms-app/src/store/persistence/v2_text/transaction/work/io.rs|write_complete_only|.create_new(true)']=1
  ['crates/catcoms-app/src/store/persistence/v2_text/transaction/work/io.rs|write_complete_only|file.sync_all().map_err(io)?;']=1
  # Both public open APIs use this one private constructor before any token/cursor exists.
  ['crates/catcoms-app/src/store.rs|open_with_catalog_policies|fs::create_dir_all(dir.join("servers")).map_err(|e| AppError::Io(e.to_string()))?;']=1
  # remove_server unlinks only the non-family {id}.bin, .net and .cache files (I-4 audit:
  # correctly not rotating).
  ['crates/catcoms-app/src/store.rs|remove_server|fs::remove_file(p).map_err(|e| AppError::Io(e.to_string()))?;']=1
  # The per-family sync helpers. Each opens an existing epoch file only to `sync_all` it, and
  # takes `&EpochMutation`, so the rotation is a type-level prerequisite of calling it.
  ['crates/catcoms-app/src/store/epoch_intents.rs|sync_intent|let file = OpenOptions::new()']=1
  ['crates/catcoms-app/src/store/epoch_intents.rs|sync_intent|file.sync_all().map_err(|e| AppError::Io(e.to_string()))?;']=1
  ['crates/catcoms-app/src/store/epoch_registry.rs|sync_registry|let file = OpenOptions::new()']=1
  ['crates/catcoms-app/src/store/epoch_registry.rs|sync_registry|file.sync_all().map_err(|e| AppError::Io(e.to_string()))?;']=1
  ['crates/catcoms-app/src/store/epoch_studio.rs|sync_studio|let file = OpenOptions::new()']=1
  ['crates/catcoms-app/src/store/epoch_studio.rs|sync_studio|file.sync_all().map_err(|e| AppError::Io(e.to_string()))?;']=1
  # Original unexposed bootstrap source worker, a child of persistence. Only these fixed
  # source.seed/source.bin/records.bin/manifest.bin destinations are reachable after its
  # actual saved per-pass SP debit. No epoch inventory family or caller path is accepted.
  # Keep exact site keys rather than exempting this child (or all persistence descendants).
  ['crates/catcoms-app/src/store/persistence/bootstrap_source_work.rs|ensure_large_source_output|Err(error) if error.kind() == std::io::ErrorKind::NotFound => OpenOptions::new()']=1
  ['crates/catcoms-app/src/store/persistence/bootstrap_source_work.rs|ensure_large_source_output|.create_new(true)']=1
  ['crates/catcoms-app/src/store/persistence/bootstrap_source_work.rs|ensure_large_source_output|OpenOptions::new()']=1
  ['crates/catcoms-app/src/store/persistence/bootstrap_source_work.rs|ensure_large_source_output|held.file.sync_all().map_err(io)?;']=1
  # First empty operational text promotion only: fixed text controls and complete-hash
  # children under the actual original group, independently paid Current/lane admissions
  # and same-mount physical lease. These are NOT inventoried epoch files. Reopened output
  # must already match the complete original frame; only create_new permits content writes.
  # Keep each exact function/text site rather than exempting this persistence descendant.
  ['crates/catcoms-app/src/store/persistence/v2_text/promotion/work.rs|sync_exact_control|let writable = OpenOptions::new()']=1
  ['crates/catcoms-app/src/store/persistence/v2_text/promotion/work.rs|sync_exact_control|writable.sync_all().map_err(io)?;']=1
  ['crates/catcoms-app/src/store/persistence/v2_text/promotion/work.rs|write_same|let file = OpenOptions::new()']=1
  ['crates/catcoms-app/src/store/persistence/v2_text/promotion/work.rs|write_same|let created_file = OpenOptions::new()']=1
  ['crates/catcoms-app/src/store/persistence/v2_text/promotion/work.rs|write_same|.create_new(true)']=1
  ['crates/catcoms-app/src/store/persistence/v2_text/promotion/work.rs|write_same|file.sync_all().map_err(io)?;']=1
  ['crates/catcoms-app/src/store/persistence/v2_text/promotion/work.rs|ensure_seed_main|fs::rename(&stage, &main).map_err(io)?;']=1
  ['crates/catcoms-app/src/store/persistence/v2_text/promotion/work.rs|stage_effects|fs::create_dir(&pages).map_err(io)?;']=1
  ['crates/catcoms-app/src/store/persistence/v2_text/promotion/work.rs|commit_effects|fs::rename(&stage_path, &head_path).map_err(io)?;']=1
  # Distinct same-owner promotion recovery: a COMPLETE authenticated seed plus fresh
  # Current/idle-lane admission fixes the nonselected children. Repair creates only a
  # listed absent child, or writes the missing EXACT suffix through a matching held file;
  # no truncate, selected head mutation or epoch-family path is reachable. The capture's
  # Windows directory open is read-only. Scratch unlink has its own pre-MAIN/no-dependent
  # purpose/version proof and actual WorkEnd; any known authenticated frame is preserved.
  ['crates/catcoms-app/src/store/persistence/v2_text/promotion/exact_repair.rs|repair_children|fs::create_dir(&source.plan.paths.pages).map_err(recovery::io)?;']=1
  ['crates/catcoms-app/src/store/persistence/v2_text/promotion/exact_repair.rs|repair_child|DependentSelection::Absent => OpenOptions::new()']=1
  ['crates/catcoms-app/src/store/persistence/v2_text/promotion/exact_repair.rs|repair_child|.create_new(true)']=1
  ['crates/catcoms-app/src/store/persistence/v2_text/promotion/exact_repair.rs|repair_child|let file = OpenOptions::new()']=1
  ['crates/catcoms-app/src/store/persistence/v2_text/promotion/exact_repair.rs|repair_child|file.sync_all().map_err(recovery::io)?;']=1
  ['crates/catcoms-app/src/store/persistence/v2_text/promotion/recovery.rs|capture|std::fs::OpenOptions::new()']=1
  ['crates/catcoms-app/src/store/persistence/v2_text/promotion/scratch.rs|run|fs::remove_file(&self.plan.paths.seed_stage).map_err(recovery::io)?;']=1
  # These two Windows directory opens are read-only metadata/identity observations,
  # structurally retained in their actual same-vault Source/physical-lease/end bundles.
  # No create/truncate/append or writable directory handle is permitted at either site.
  ['crates/catcoms-app/src/store/persistence/v2_text/query.rs|capture|fs::OpenOptions::new()']=1
  # Accepted lookup is independently paid and retains the same actual native lease,
  # finite IO end and affine root pin through these readonly directory handles.
  ['crates/catcoms-app/src/store/persistence/v2_text/accepted_query/io.rs|capture|fs::OpenOptions::new()']=1
  ['crates/catcoms-app/src/store/persistence/v2_text/workspace/io.rs|capture|fs::OpenOptions::new()']=1
  # Current namespace3 pre-exposure transaction. Only its original fixed declaration seed,
  # whole catalog current/stage and marker under SAME held ID2 Root are reachable; these are
  # non-epoch metadata. Keep exact sites rather than exempting the child directory or module.
  ['crates/catcoms-app/src/store/persistence/current_layout.rs|install_current_layout_marker|file.sync_all().map_err(current_io)?;']=1
  ['crates/catcoms-app/src/store/persistence/current_layout.rs|install_current_layout_marker|root.directory.sync_all().map_err(current_io)?;']=1
  ['crates/catcoms-app/src/store/persistence/current_layout.rs|write_current_layout_declaration|actual.sync_all().map_err(current_io)?;']=1
  ['crates/catcoms-app/src/store/persistence/current_layout.rs|write_current_layout_declaration|stage_file.sync_all().map_err(current_io)?;']=1
  ['crates/catcoms-app/src/store/persistence/current_layout.rs|write_current_layout_declaration|fs::rename(&stage_path, &current_path).map_err(current_io)?;']=1
  ['crates/catcoms-app/src/store/persistence/current_layout.rs|write_current_layout_declaration|stage.sync_all().map_err(current_io)?;']=1
  ['crates/catcoms-app/src/store/persistence/current_layout.rs|write_current_layout_declaration|fs::remove_file(&stage_path).map_err(current_io)?;']=1
  ['crates/catcoms-app/src/store/persistence/current_layout.rs|write_current_layout_declaration|current.sync_all().map_err(current_io)?;']=1
  ['crates/catcoms-app/src/store/persistence/current_layout.rs|write_current_layout_declaration|saved.sync_all().map_err(current_io)?;']=1
  ['crates/catcoms-app/src/store/persistence/current_layout.rs|write_current_layout_declaration|fs::remove_file(&seed_path).map_err(current_io)?;']=1
  ['crates/catcoms-app/src/store/persistence/current_layout.rs|read_verified_current_seed|file.sync_all().map_err(current_io)?;']=1
  ['crates/catcoms-app/src/store/persistence/current_layout.rs|sync_current_namespace_root|root.directory.sync_all().map_err(current_io)']=1
  # Original negative V2 group directory only. The concrete creating admission/task guard
  # fixes its g- destination under the same held Root; no epoch/source/page file is written.
  ['crates/catcoms-app/src/store/persistence/v2_group_directory.rs|write_v2_genesis_group_directory|fs::create_dir(&output.path).map_err(directory_io)?;']=1
  ['crates/catcoms-app/src/store/persistence/v2_group_directory.rs|write_v2_genesis_group_directory|file.sync_all().map_err(directory_io)?;']=1
  ['crates/catcoms-app/src/store/persistence/v2_group_directory.rs|write_v2_genesis_group_directory|.sync_all()']=1
  ['crates/catcoms-app/src/store/persistence/v2_group_directory.rs|open_created_group_directory|OpenOptions::new()']=1
  # Original V2 genesis head promotion only: the distinct actual Initializing1 group guard
  # holds admission/pins/canonical/Root, and the complete activation seed plus SAME held
  # stage are verified before promotion. Both names are closed local-source destinations,
  # outside epoch inventories. The warm worker is paired with its actual unexposed caller.
  ['crates/catcoms-app/src/store/persistence/cold_source_read/v2_genesis_activation.rs|save_head|fs::rename(&stage_path, &head_path).map_err(io)?;']=1
  # Connected cold genesis Current2 closure; Initializing1-only entry points stay test-only.
  # Its concrete read guard holds admission/pins/canonical/Root and lends
  # these SAME existing non-epoch fixed objects; no creation, rename, removal or user path.
  ['crates/catcoms-app/src/store/persistence/cold_source_read/v2_genesis_activation/cold_capture.rs|sync_existing|seed.object.file.sync_all().map_err(io)?;']=1
  ['crates/catcoms-app/src/store/persistence/cold_source_read/v2_genesis_activation/cold_capture.rs|sync_existing|self.pin.object.file.sync_all().map_err(io)?;']=1
  ['crates/catcoms-app/src/store/persistence/cold_source_read/v2_genesis_activation/cold_capture.rs|sync_existing|self.head.object.file.sync_all().map_err(io)?;']=1
  ['crates/catcoms-app/src/store/persistence/cold_source_read/v2_genesis_activation/cold_capture.rs|sync_existing|self.group_directory.sync_all().map_err(io)?;']=1
  ['crates/catcoms-app/src/store/persistence/cold_source_read/v2_genesis_activation/cold_capture.rs|sync_existing|self.target_directory.sync_all().map_err(io)?;']=1
  ['crates/catcoms-app/src/store/persistence/cold_source_read/v2_genesis_activation/cold_capture.rs|sync_existing|root.directory.sync_all().map_err(io)?;']=1
  # Authenticated cold Initializing1 repair, under its concrete new original-read capture
  # and SAME actual AD/pins/canonical/Root owner. Capture synchronizes existing fixed
  # non-epoch objects only. The distinct installed Init1 writer, after an even-clean scope
  # barrier, may strict-create/complete only the selected pin and head.stage, promote the
  # SAME complete stage to absent head.bin, and sync their original parents. No source,
  # activation seed, private/unknown work or epoch-family file is rewritten or removed.
  # The readonly selected-target opener has no create/truncate/append option; its handle
  # is cached before identity queries and can never reopen a failed original selection.
  ['crates/catcoms-app/src/store/persistence/cold_source_read/v2_genesis_activation/initializing_repair.rs|capture_initializing|files.seed.object.file.sync_all().map_err(io)?;']=1
  ['crates/catcoms-app/src/store/persistence/cold_source_read/v2_genesis_activation/initializing_repair.rs|capture_initializing|files.group_directory.sync_all().map_err(io)?;']=1
  ['crates/catcoms-app/src/store/persistence/cold_source_read/v2_genesis_activation/initializing_repair.rs|capture_initializing|files.target_directory.sync_all().map_err(io)?;']=1
  ['crates/catcoms-app/src/store/persistence/cold_source_read/v2_genesis_activation/initializing_repair.rs|capture_initializing|guard.archive_root().directory.sync_all().map_err(io)?;']=1
  ['crates/catcoms-app/src/store/persistence/cold_source_read/v2_genesis_activation/initializing_repair.rs|open_selected_target|let file = std::fs::OpenOptions::new()']=1
  ['crates/catcoms-app/src/store/persistence/cold_source_read/v2_genesis_activation/initializing_repair.rs|create_target|let file = std::fs::OpenOptions::new()']=1
  ['crates/catcoms-app/src/store/persistence/cold_source_read/v2_genesis_activation/initializing_repair.rs|create_target|.create_new(true)']=1
  ['crates/catcoms-app/src/store/persistence/cold_source_read/v2_genesis_activation/initializing_repair.rs|finish_target|selected.file.sync_all().map_err(io)?;']=1
  ['crates/catcoms-app/src/store/persistence/cold_source_read/v2_genesis_activation/initializing_repair.rs|repair|files.group_directory.sync_all().map_err(io)?;']=1
  ['crates/catcoms-app/src/store/persistence/cold_source_read/v2_genesis_activation/initializing_repair.rs|repair|fs::rename(&files.stage_path, &files.head_path).map_err(io)?;']=1
  ['crates/catcoms-app/src/store/persistence/cold_source_read/v2_genesis_activation/initializing_repair.rs|repair|.sync_all()']=1
  ['crates/catcoms-app/src/store/persistence/cold_source_read/v2_genesis_activation/initializing_repair.rs|repair|files.target_directory.sync_all().map_err(io)?;']=1
  # Complete original activation journal on actual group-wide no-row admission. These
  # barriers touch ONLY the already selected original non-epoch journal/directories under
  # AD/pins/canonical/Root. There is no create/write/rename/unlink and no row permission;
  # the distinct Born scope file+parent barrier still precedes the normal repair worker.
  ['crates/catcoms-app/src/store/persistence/cold_source_read/v2_genesis_activation/pre_initializing.rs|capture_vacant|files.seed.object.file.sync_all().map_err(io)?;']=1
  ['crates/catcoms-app/src/store/persistence/cold_source_read/v2_genesis_activation/pre_initializing.rs|capture_vacant|files.group_directory.sync_all().map_err(io)?;']=1
  ['crates/catcoms-app/src/store/persistence/cold_source_read/v2_genesis_activation/pre_initializing.rs|capture_vacant|files.target_directory.sync_all().map_err(io)?;']=1
  ['crates/catcoms-app/src/store/persistence/cold_source_read/v2_genesis_activation/pre_initializing.rs|capture_vacant|guard.archive_root().directory.sync_all().map_err(io)?;']=1
  # Actual seed-present Current2 cleanup only, under its SAME successful covering scope
  # admission, concrete pins/canonical/Root guard and retained original selection. The worker
  # can unlink only its fixed original activation.seed and sync that held group directory;
  # source.seed, source pages, pin/head, epoch files and unknown private work are preserved.
  ['crates/catcoms-app/src/store/persistence/cold_source_read/v2_genesis_activation/settlement.rs|settle|fs::remove_file(&self.body.paths.seed).map_err(io)?;']=1
  # Chat V2 ordinary-member outbox (docs/communication-recovery/CHAT-V2-MEMBER-OUTBOX.md).
  # Its own separately versioned `mo-<context>` namespace only, never an epoch, founder or
  # group inventory file. Every caller holds the exclusive per-context owner claim and the
  # actual VaultPhysicalWorkLease through all of these Files and barriers.
  # ensure_exact: create_new, or append ONLY the missing suffix of an existing exact prefix
  # of authenticated expected bytes (our presealed output or an authenticated WAL's frame);
  # never truncate, never adopt other content.
  ['crates/catcoms-app/src/store/persistence/v2_member_outbox/fs.rs|ensure_exact|OpenOptions::new()']=1
  ['crates/catcoms-app/src/store/persistence/v2_member_outbox/fs.rs|ensure_exact|Err(error) if error.kind() == std::io::ErrorKind::NotFound => OpenOptions::new()']=1
  ['crates/catcoms-app/src/store/persistence/v2_member_outbox/fs.rs|ensure_exact|.create_new(true)']=1
  ['crates/catcoms-app/src/store/persistence/v2_member_outbox/fs.rs|ensure_exact|file.sync_all().map_err(io)?;']=1
  # rename_fixed: WAL stage -> WAL MAIN, and head stage -> head.bin (the selector CAS), each
  # after re-verifying the source's exact bytes and the parent still selected.
  ['crates/catcoms-app/src/store/persistence/v2_member_outbox/fs.rs|rename_fixed|fs::rename(from, to).map_err(io)']=1
  # remove_listed: a WAL-listed retiree proven unreachable, the retired WAL after its cleanup,
  # or a demonstrably owned torn pre-commit stage (exact rebuildable preamble prefix bound to
  # the authenticated selected head). Unknown or foreign state is preserved instead.
  ['crates/catcoms-app/src/store/persistence/v2_member_outbox/fs.rs|remove_listed|match fs::remove_file(path) {']=1
  # create_namespace: one new `mo-<context>` directory under the separately held parent.
  ['crates/catcoms-app/src/store/persistence/v2_member_outbox/fs.rs|create_namespace|fs::create_dir(path).map_err(io)']=1
  ['crates/catcoms-app/src/store/persistence/cold_source_read/v2_genesis_activation/settlement.rs|settle|files.directory.1.sync_all().map_err(io)?;']=1
)

# Print `file<TAB>line<TAB>enclosing fn<TAB>trimmed text` for every match in non-test code. The
# enclosing fn is the nearest `fn` item declared above the match, or `-` before any. Skipped: whole test files, the body of
# a `#[cfg(test)]` item (a module, function, impl or other item, which rustfmt closes with `}` or
# `};` at the attribute's own indentation; a one-line item or declaration ends at its `;`), and
# `mod persistence { .. }` in store.rs.
#
# Every skip is designed to fail closed. A `#[cfg(test)]` on anything that is not an item (a
# struct-field initializer, a statement or an expression) skips nothing, so whatever it covers is
# still scanned and can only add hits; the first version of this gate skipped such an attribute
# as a block and silently swallowed production lines after it. A skip that never finds its
# closing line makes the scanner fail rather than swallow the rest of the file.
#
# The one assumption: an item's closing brace sits at the attribute's own indentation, which is
# rustfmt's layout, and CI enforces rustfmt (`cargo fmt --all -- --check`). Hand-misindented code
# could end a skip early at a later brace, so run this gate on formatted code only.
scan() {
  # The pattern travels through the environment: `awk -v` would process its backslashes.
  PATTERN="$pattern" awk -v file="$1" '
    BEGIN { pattern = ENVIRON["PATTERN"]; current = "-" }
    function indent(s) { match(s, /^[ \t]*/); return substr(s, 1, RLENGTH) }
    function trim(s) { sub(/^[ \t]+/, "", s); sub(/[ \t\r]+$/, "", s); return s }
    function skip(from) { skipping = 1; skip_indent = from; skip_start = NR }
    # A flag, not an empty indent: a top-level item has no indentation at all.
    skipping {
      if ($0 == skip_indent "}" || $0 == skip_indent "};") { skipping = 0 }
      next
    }
    pending {
      pending = 0
      if ($0 ~ /^[ \t]*#\[/) { pending = 1; next }   # further attributes on the same item
      if ($0 ~ /;[ \t]*$/) { next }                   # declaration or one-line item
      if ($0 ~ /^[ \t]*(pub(\([^)]*\))? )?((async|unsafe|const|extern) )*(mod|fn|impl|struct|enum|union|trait|static|const|type|macro_rules!)[ \t<({]/) {
        skip(attr_indent)
        next
      }
      # Not an item: fall through, so the line (and what follows) is scanned as usual.
    }
    /^[ \t]*#\[cfg\(test\)\][ \t]*$/ { pending = 1; attr_indent = indent($0); next }
    /^(pub(\([^)]*\))? )?mod persistence \{/ { skip(indent($0)); next }
    # The enclosing function: the same item shape the skip rule recognises, restricted to `fn`.
    /^[ \t]*(pub(\([^)]*\))? )?((async|unsafe|const|extern) )*fn [A-Za-z_]/ {
      name = $0
      sub(/^.*fn /, "", name)
      match(name, /^[A-Za-z_][A-Za-z_0-9]*/)
      current = substr(name, 1, RLENGTH)
    }
    $0 ~ pattern { printf "%s\t%d\t%s\t%s\n", file, NR, current, trim($0) }
    END {
      if (skipping) {
        printf "%s:%d: skip never closed; the scanner cannot tell test code from production here\n", file, skip_start > "/dev/stderr"
        exit 2
      }
    }
  ' "$1"
}

fail=0
declare -A counts=()
while IFS= read -r file; do
  case "$file" in
    */tests/*|*/tests.rs|*_tests.rs) continue ;;
  esac
  # Captured, not streamed, so a scanner failure stops the gate instead of reading as "no hits".
  hits=$(scan "$file")
  while IFS=$'\t' read -r hit_file line fn text; do
    [ -z "$hit_file" ] && continue
    key="$hit_file|$fn|$text"
    counts["$key"]=$(( ${counts["$key"]:-0} + 1 ))
    if [ "${counts["$key"]}" -le "${allowed["$key"]:-0}" ]; then
      continue
    fi
    echo "FORBIDDEN raw filesystem mutation in the store:"
    echo "    $hit_file:$line: in fn $fn: $text"
    fail=1
  done <<< "$hits"
done < <( { echo crates/catcoms-app/src/store.rs; find crates/catcoms-app/src/store -name '*.rs'; } | sort)

# A reviewed site that disappeared, moved to another function or changed its text must leave the
# allowlist too, or the allowance outlives it.
for key in "${!allowed[@]}"; do
  if [ "${counts["$key"]:-0}" -lt "${allowed["$key"]}" ]; then
    echo "STALE allowance, no longer matched exactly once: $key"
    fail=1
  fi
done

if [ "$fail" -ne 0 ]; then
  echo
  echo "Store raw-fs gate FAILED. Write epoch files only through the persistence module's"
  echo "guarded primitives (EpochMutation), so inventory_generation rotates first (I-4)."
  exit 1
fi

echo "Store raw-fs gate passed."
