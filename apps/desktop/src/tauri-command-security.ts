/**
 * Review ledger for every command exposed by the desktop IPC bridge.
 *
 * This does not grant authority. Rust and `catcoms-app` remain the enforcement points. Its job is
 * to make the exposed surface enumerable: the companion test fails whenever a command is added,
 * removed or invoked without updating this review classification.
 */
export const TAURI_COMMAND_GROUPS = {
  local_session_and_vault: {
    boundary: "Local vault/session state; secrets and filesystem paths must remain native-side.",
    commands: [
      "vault_exists", "resume_session", "lock_session", "unlock", "get_ui_state", "save_ui_state",
      "create_backup", "change_vault_secret", "get_debug_logging", "set_debug_logging",
      // Writes a frontend line to the local debug log and nothing else. Deliberately outside the
      // unlocked-session gate: the errors most worth capturing are the ones from startup and from
      // unlock itself failing, which happen before there is a session to check.
      "log_ui",
    ],
  },
  server_lifecycle_and_membership: {
    boundary: "Signed invites, bounded pre-join reply dialing, group membership and role-changing operations; protocol checks are authoritative.",
    commands: [
      "found_server", "preview_invite", "join_server", "apply_join_reply", "leave_server", "get_invite", "mint_invite_fresh",
      "set_admin", "remove_member", "revoke_device",
    ],
  },
  authenticated_server_reads: {
    boundary: "Read projections from a registered per-server actor; never bypass MLS/CRDT validation.",
    commands: [
      "get_channels", "get_members", "get_profiles", "get_livery", "get_badges", "get_devices",
      "get_files", "get_storage_health", "get_online_members", "get_delivery", "dm_stats",
      "get_dm_requests", "file_available", "get_file_usage", "get_wiki_pinned_cids", "get_statuses",
      "get_events", "get_wiki_pages", "get_wiki_map", "get_wiki_page", "get_wiki_meta",
      "get_wiki_history", "get_wiki_pending", "get_wiki_review_days", "get_roles", "get_moderation",
      "get_join_attempts", "get_connectivity", "get_call_transport", "get_switchboard_status", "set_switchboard_offered", "get_channel_topic", "get_jukebox", "get_inbox",
      // Route booleans + the mapping status line only; concrete addresses stay native-side.
      "check_invite_routes",
      "get_messages",
    ],
  },
  authenticated_content_writes: {
    boundary: "Member/content mutations through the actor; validate sizes, ids and wire-safe fields natively.",
    commands: [
      "open_channel", "set_profile", "repair_storage", "send_dm_invite",
      // The streamed upload replaces the old single-shot add_file: begin reserves a slot, each
      // push seals exactly one chunk into the vault, finish publishes the index entry, cancel
      // releases the reservation and collects whatever was sealed. Same authority as add_file
      // had, split so neither the webview nor the server actor is occupied for a whole file.
      "begin_file_upload", "push_file_chunk", "finish_file_upload", "cancel_file_upload",
      "send_call_signal", "dismiss_dm_request", "download_file", "post_status", "create_event",
      "save_wiki_page", "send_message", "edit_message", "delete_message", "toggle_reaction",
      "set_channel_topic", "jukebox_add", "jukebox_remove",
    ],
  },
  policy_controlled_writes: {
    boundary: "Role/author/policy-sensitive mutations; UI visibility is never authorization.",
    commands: [
      "rename_server", "set_livery", "set_server_icon", "set_server_cursor", "set_member_badge",
      "delete_file", "set_file_expiry", "delete_event", "set_wiki_format", "delete_wiki_page",
      "rename_wiki_page", "set_wiki_review_days", "approve_wiki_edit", "reject_wiki_edit",
      "restore_wiki_page", "warn_message", "create_kick_case", "cast_kick_vote",
      "resolve_kick_case", "set_pin",
    ],
  },
  media_key_material: {
    boundary: "Call signalling/key material exposed to the trusted webview only for the active server.",
    commands: ["call_media_key"],
  },
  router_boundary: {
    boundary: "Best-effort router port mappings for the active call's media sockets; exact caller-named ports, bounded leases, no other router state.",
    commands: ["map_call_port", "unmap_call_port"],
  },
  device_pairing: {
    boundary: "Single-use, SAS-confirmed, passphrase-sealed device grant ceremony.",
    commands: [
      "pairing_begin", "pairing_read", "pairing_mint", "pairing_decline", "pairing_open", "pairing_join",
    ],
  },
  operating_system_boundary: {
    boundary: "URLs and files cross into the OS; exact allowlists, bounds and non-shell launch/write paths required.",
    commands: [
      "open_issue_url", "open_external_url", "save_and_open_space_guide", "save_space_layout",
      // Writes a shared file to Downloads by streaming it from the actor: the name is sanitized
      // native-side and the reserved path is the only thing that reaches the webview.
      "save_group_file",
    ],
  },
} as const;

export const REVIEWED_TAURI_COMMANDS = Object.freeze(
  Object.values(TAURI_COMMAND_GROUPS).flatMap((group) => group.commands),
);
