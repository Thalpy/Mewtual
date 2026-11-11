# Native member saves: production prerequisites

11 November design handoff. This document records implementation work still to
do; it introduces no API, factory, native command or activation. The root member
save driver is in draft PR #86. Its test native custody is not authentication.
See [HANDOVER](../HANDOVER.md) for execution evidence and the
[A/B/F contract](CHAT-V2-MEMBER-SAVE-HANDOFF.md) for the approved save boundary.
Neither the joined-member source nor native integration has an external agent
running; the primary owns the next native slice.

## What the current production paths actually provide

- Native `unlock_cold` opens `ServerStore::open`, and `AppState.store` retains
  `Option<ServerStore>`. `ServerEntry` requires a legacy `ServerActor`.
- `V2CurrentLayoutStore` owns its private Store. Its public historical source
  reader does not provide a complete public source-to-seed/bootstrap driver.
  Existing root fixtures reach Store-private methods unavailable to native code.
- A whole `PreparedV2TextHost` retains its unbound node. That node deliberately
  provides no runnable legacy `Server` extraction. Cloning a legacy actor's SDK
  or adding a host alongside that SDK is not the missing integration.
- `ArchiveFounderCreation::group_data` is Store-private. The native
  `ServerRecord` currently contains only id, name, invite and DM metadata; it
  has no durable V2 backend/source binding. Historical source lookup requires
  the exact group and signer from the genuine saved producer.
- The existing cold-genesis capture is restricted to genesis heads. Its
  seed-present path compares the original activation head ciphertext; its
  seed-absent path checks the strict genesis head/image and fixed inventory.
  Even an empty operational promotion changes this head. Consequently,
  `PreparedV2TextRead::from_cold` cannot reopen the promoted operational host.

The relevant source families are `apps/desktop/src-tauri/src/{lib,keep_session}.rs`,
`store/critical_namespaces/{current_layout,archive_bootstrap/founder}.rs`, and
`store/persistence/cold_source_read/genesis_seed/current/` and
`store/persistence/cold_source_read/v2_genesis_activation/cold_capture/`. A seed-forwarding
wrapper alone does not close these gaps.

## Required coherent implementation

1. Provide a closed typed Root bootstrap driver using the real existing
   archive/source/current owners. Its owning retries retain the same source,
   device, clock/RNG inputs, outputs and payments. Return the actual whole Host
   with a narrow historical source binding, not a raw Store/SDK/node accessor,
   generic effect callback or caller-constructed selection receipt.
2. Give the native vault and row explicit V2 backend custody. One physical mount
   must serve its actual owner; do not open the same vault twice to obtain the
   typed wrapper or silently interpret a V2 row as a legacy actor. Persist the
   genuine binding produced by the completed bootstrap, with bounded allocation
   and fail-closed handling of unknown/incomplete backend state. Numeric server
   ids and caller labels do not establish incarnation or source identity.
   The mounted-vault backend must also support its concrete common operations:
   passphrase verification, registry/network persistence, UI continuity, pairing,
   backup and secret changes. Do not expose the wrapped Store through `Deref`, a
   raw getter or a generic callback to avoid implementing those operations.
3. Implement a distinct closed cold-operational owner factory. Authenticate the
   actual selected O/H, original pin and governing prefix, acquire the real
   scope/writer/ownership admissions, and replay through the appropriate paid
   verifier. Preserve the exact retained device/source once. Parsed head labels,
   a historical receipt or a cast to cold genesis cannot issue that ownership.
   Keep the cold-genesis rejection intact. This is a prerequisite for claiming
   warm-save/cold-reopen or restart-safe native operation.
4. Bind an invocation to the original native session and row before its first
   authorization wait. Resolve the still-open native command-credential/key-cut
   boundary as well as final save admission: phase 2's UI guard alone does not
   establish what may enter the signer during phase 1. Distinguish permitted
   completion of already admitted DATA work from fresh work after revocation;
   retain the genuine admitted key reader through once-only raw caching, with
   the actual source enforcement intact.
5. Connect B's two phases through a private checked native factory. Phase 1
   performs blocking lookup/sign/preseal without UI commit custody. Obtain
   original per-entry custody, then release global registry guards before IO.
   Release host/Store/registry guards before awaiting the actual owned UI commit
   guard.
   Recheck the original generation/row, obtain remaining custody fail-fast in
   the established lock order, then call the finite root final edge. The complete
   `keep_session` authority must survive composition with the original row.
6. Immediately move the admitted job to native-owned `spawn_blocking`, with no
   intervening await. IPC receiver loss cannot own or cancel the write. Keep the
   real UI/source/persistence custody until outcome caching and cleanup finish;
   check the original generation/row again before reporting to the UI. Lock,
   leave and replacement retire original lifetimes irreversibly.
   Drain admitted work before disposing of the physical mount and preserve the
   Store's exact original/outcome DATA during retirement. V2 critical history
   must not enter legacy leaving/cache deletion.

Keep these consumers together; unused forwarding methods or an arbitrary
`install(host)` fixture do not establish production installation. No pool or
stack increase is assumed. Account for actual native entries/guards and their
destruction order before allocation or admission. Joined-member/later-prefix
support and governance4 operational ownership remain separate required consumers.

M4 retains bounded signed-original DATA across UI lock. It renews no authorization
and is not a promise of RAM erasure. A later save still needs fresh actual native
and source checks; memory custody never reports SavedLocal.

## Acceptance through the actual producer

Use the genuine public typed bootstrap and native registry factory in the native
fixture. Do not inject a fabricated Host, arbitrary custody box or literal
authorization boolean and call it native acceptance. Use deterministic Clock/RNG
seams and barriers rather than sleeps.

- Save, close the whole owner and mount, reopen the actual operational backend,
  and recover the same receipt/original without another signature.
- Race lock with initial invocation authorization, the signer boundary, final
  admission and paused physical work. Exercise both orderings and a complete
  lock/unlock cycle; the new generation cannot revive the old invocation.
- Replace/remove the actual row at the same boundaries, including reuse of its
  numeric id. No old observer may acquire the replacement's authority.
- Drop an IPC waiter before dispatch and during IO. Admitted work retains its
  owner and outcome; unadmitted work retains DATA without positive authority.
- Prove lock cannot report completion ahead of admitted IO, and completion is
  cached before native custody drops. Test unrun drop, uncertain write and unwind
  with the actual factory, not only root's test box.

Run the required root/native/frontend suites and separate native check in CI,
alongside strict formatting/lint/dependency gates. A root focused pass, native
type check or one-desktop fixture does not establish two-device/network
acceptance. Chat V2 remains disabled until its actual production consumers and
required acceptance are complete.
