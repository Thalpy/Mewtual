# Gate 7: Flipnote production adapter handoff

Baseline: owner-accepted Gate 6, PR #44 merged at `82631a81` (2026-10-17).
This is the current contract; dated candidate records elsewhere retain their original evidence.
The detailed envelopes/examples remain in [FLIPNOTE-UI-HOOKS](FLIPNOTE-UI-HOOKS.md),
[INTERFACES](INTERFACES.md) and [design-creative-suite](design-creative-suite.md).
Verification is recorded separately in [GATE7-ACCEPTANCE](GATE7-ACCEPTANCE.md).

The current UI completion candidate adds shared-deck PIXA and a global file viewer, explicit
source/Registry repair controls, and a separate ticketed Closing local-draft Save. These are
new command/UI contracts. Dated checkpoints in other documents that call them unavailable
are historical; final verification is recorded separately from implementation.

## Canonical surface and its actual owner

| Surface | Production hook | Required caller behavior |
|---|---|---|
| Channel library / open / create | `studio_list`, `studio_read`, `studio_create` | Use the existing `StudioSession` and `studio-native.ts`. Index is `object:null`; explicit `objectKind:"score"` selects the Score family. Absent objects return null; an absent Index supplies the existing empty epoch-zero Index view. |
| Art / title / FPS / frame order | `publish_pix`, then `studio_apply`; Index edits use `studio_apply_index` | Flatten local layers into validated PIX1. Save the actual returned CID/length with the current epoch, stable nonce and canonical body. Editing/rendering/layout remain frontend-owned. |
| Score composer | `studio_create/read/apply` with `objectKind:"score"` | Preserve cell removals, conflicts and overflow. A saved Score cannot be reinterpreted as art, even when the same object id/body shape overlaps. |
| Frame Sound / recipe library | `studio_apply` with `set_patch/remove_patch/set_sfx/remove_sfx` | Recipes use verified jam-patch identities; holds are authored durations. Missing frames/recipes, conflicts and overflow are export/playback readiness problems, never grounds for dropping retained evidence. |
| Linked Music | `studio_apply` with `set_header` for `score`; typed Score Read | Linking needs a real typed Score in the same group/channel. A label, Index hint or known foreign object id is insufficient. |
| Preview / Score / saved take playback | Existing `studio-playback.ts`, `jam-playback.ts` and JamEngine | Require explicit gesture and resolved inputs; retain authored holds and Score grid/trailing rests. Scope, membership, lock, Deafen, cancellation and pending-download fencing apply. Remote traffic cannot resume a suspended synth. |
| PIXA publish / record / preview / download / share | `publish_pixa`, `studio_apply` (`set_export`), `request_blob_bounded`; existing file-share path | Publication holds validated actual bytes; recording the reference is a separate durable Save. Check exact CID/length and validate the fetched package. Sharing is not remote custody or a receipt. `.pixa`/`application/x-mewtual-pixa` files can open in the global viewer or queue in the existing jukebox. |
| Shared-deck PIXA / global viewer | Existing Jukebox transport, `download_file` with `maxBytes:9437184`, `begin_inline_download/cancel_inline_download`, JamTakeTransport/StudioPlaybackController | Keep existing call/trust/local-consent/DJ rules; no new consensus or media network protocol. Validate complete bytes before synthesis, cache at most two encoded packages per call and expand only the current raster. Remote traffic cannot resume audio. Global viewer opens explicitly from file details (including chat/wiki file references), owns one current package and explicit Play, and cancels on close/navigation/lock/membership/Deafen edges. |
| Local visual GIF | `export_studio_gif` | Native owns the safe filename/path and final save fence. `displayed:false` with a path still acknowledges saved output; reveal failure must not invite duplicate export. GIF is visual-only. |
| Collaborator frame claims / Ask / Pass | Existing call data channel, `StudioClaimSnapshot` and `claimActions` | Advisory only; no native edit lock or Save authority. Preserve full device identity, receiver-observed countdowns and refused-send retries. |
| Background join / missed edits / rotation | Existing actor Studio receiver, authenticated pages/head/seed, native idle worker | UI listens and re-reads; it must not implement another catch-up scheduler, source cache or receipt owner. Watches and provider service remain bounded. |
| Phase / recovery / receive warnings | `studio-updated`, `settlement-changed`, `studio-receive-paused` | Invalidate matching observations, coalesce reads and fence old navigation/session results. Events are neither projections nor receipts. |
| Retained version inspect / backup | `studio_recovery_list/read/export/acknowledge` | Display historical versions separately. Export is `p1-recovery-v1`, not PIXA, fetched-media backup or an implemented import format. Acknowledge only the actual persisted warning pair. |
| Per-item Restore / Copy / pointer restore | `studio_recovery_preview/apply/restore_pointer` | Preview again before each item; Apply echoes the exact fingerprint, epoch, choice, nonce and canonical body. Restore is additive; Copy explicitly permits its supported replacement/removal. Content recovery is ordinary provisional Save. Pointer restoration is separate. |
| Local draft inspect / export / preserve / copy / dispose | `studio_overlay_read/lifecycle/export/archive/archive_read/archive_export/archive_release/dispose`, `studio_overlay_copy_preview/apply` | Use actual branch/content generations and typed eligibility. Copy never proves branch preservation; only an archive acknowledgement does. Destructive operations require their exact confirmation literals. |
| Awaiting-tenure local Save | `studio_unconfirmed_overlay_save_begin`, `studio_unconfirmed_overlay_save` | Base preview stays read-only shared history. The ticketed branch is local-only, non-transferable and non-authoritative. Follow typed retry instructions; never invent a new request to resolve ambiguous success. |
| Closing local-draft Save | `studio_closing_overlay_save_begin`, `studio_closing_overlay_save` | Separate from Unconfirmed and ordinary Apply. Echo only `basis,branch,nonce,body`; actor derives the actual signed close from its sealed owner journal. Fresh authoring requires that close, Known tenure and current eligible Closing source; normally only the journal-holding device can do this. Missing close refuses without discarding pixels. Accepted/terminal exact retries classify before current close/source/tenure; pending/busy/paused retain the original identity. Automatic handoff, not this local Save, changes shared history. |
| Explicit Fault repair | `studio_fault_read`, `studio_fault_repair` with `scope:"source"|"registry"` | Index and selected/failed-object controls remain accessible. Show both actual candidate receipts and echo only `receiptA,receiptB,selected`; actor/store rederive the pair and current durable owner tenure. Report scheduled/busy and re-read actual status; a withheld scheduled/dropped reply is uncertain. Restore/Copy cannot clear Fault. Invalidation or imported package labels grant no repair authority. |

## Identity, retries and present-time truth

- Channels are canonical decimal u128 strings; epochs are decimal u64 strings; object/frame/
  nonce/epoch ids are 32 lowercase hex, CIDs/device ids/fingerprints are 64 lowercase hex.
  Keep the stable full group identifier distinct from a numeric local server alias.
- Bodies are bounded canonical JSON strings, not renderer-authored signed envelopes or
  Automerge changes. Backend derives author, signing and admission authority.
- Ordinary successful views are `provisional:true` and `publication:"local"`. Open, a
  nonzero epoch, successful publication or an invalidation must not become “delivered”,
  “settled”, a receipt author/time, or “all alternatives recovered”.
- Keep the same nonce, body, target and epoch on uncertain retries. Retain unsaved editor
  work after timeout/cancellation or rejected admission. Re-author only after a typed
  known-no-write result and the existing explicit user action. A lock/navigation/actor
  replacement revokes delivery currency; a late result cannot populate a newer session.
- Recovery walks are sequences of fresh previews and ordinary Saves, not atomic branch
  replacement. Retained removal evidence blocks SFX/export resurrection through recovery,
  including Copy. Recipe Copy can explicitly re-add a removed recipe. This is distinct
  from ordinary new authoring after a compact seed has omitted a deleted id.
- Score historical OBJECT edits remain manual. Own-intent replay preserves original identity
  and refuses unsafe/newer state; it is not automatic replay of every historical alternative.

## Events

`studio-updated` carries `{server,channel,object,objectKind?}`. `settlement-changed` adds
`{docType,logicalKey,state}`; Score carries `objectKind:"score"`. Its complete state vocabulary
is `open`, `closing`, `settled`, `fault`, `recoveryAvailable`, `recoveryEvictionPending`,
`refreshRequired`, `repairing`, `storageRefused`. `studio-receive-paused` carries `{server}`.
Phase and recovery notices are independent; always re-read the actual relevant state.
An explicit successful access resumes a paused receiver; read/list calls do not emit an
invalidation loop. Dispose listeners on session/component teardown.

## Enforced limits and honest recovery boundaries

| Boundary | Current limit / behavior | Enforcement source |
|---|---|---|
| PIX / art | 192×144 pixels, at most 999 frames, 1–24 FPS; PIX1 at most 64 KiB | `creative.rs`, replication Studio frames, frontend PIX codec |
| Score | 256 steps, 25 rows, 4096 cells, 64 recipes | `studio/score.rs`, typed native projection and `studio-export.ts` |
| Sound / export recipe union | 4096 SFX, 8 live export references; at most 64 distinct verified recipes across Score+SFX; authored holds at most 600,000 ms | `studio/frames/sound.rs`, `studio-export.ts`, jam validators |
| PIXA / bounded fetch | 9 MiB complete package/fetch bound; PIXA encoded frames sum at most 8 MiB, take at most 512 KiB, SFX entries at most 32 KiB; complete framing and embedded validation | `creative/pixa.rs`, `pixa.ts`, native creative blob/export commands |
| Domain operation | 64 KiB complete DomainOp envelope, including framing; the body must fit within the remaining space. Independent body precheck and exact checkpoint/recovery preflight also apply | replication epoch constants, Studio request validation |
| Catch-up | 32 operations / 512 KiB per page; authenticated scope/cursor/deadline; shared bounded queues and detached preparation | Studio page/receiver and existing Registry exchange |
| Retained recovery | Two retained versions plus one staged warning; persisted deadline and exact warning pair survive restart | recovery store and `studio_recovery_list/acknowledge` |

These are admission/work rails, not arbitrary-size latency guarantees. Retained recovery
references protect actual held media from existing reclamation; they cannot manufacture
missing peer bytes. Recovery backup does not bundle PIX blobs. Header-only Index Copy retains
its accepted preservation limitation. Local bounded emoji JSON import does not supply a
verified shared emoji catalog/chat-click feature.

Native audible hardware checks, two desktop UI sessions and platform compatibility observations
need separately recorded release evidence. Actor/vault, TCP and mocked frontend results have
their own scopes and cannot stand in for those observations.
