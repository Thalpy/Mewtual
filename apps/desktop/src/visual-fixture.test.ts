import assert from "node:assert/strict";
import test from "node:test";
import { VISUAL_FIXTURE_NOW, visualFixtureResponse } from "./visual-fixture.ts";
import { VISUAL_SAVED_FILES, VISUAL_TRANSFER_ROWS } from "./visual-transfers-fixture.ts";
import { checkedPrivateDownloads, checkedSavedCheck, type SavedCheck } from "./private-downloads.ts";
import { savedRowView } from "./transfer-rows.ts";
import { sanitizeUiContinuity } from "./ui-continuity.ts";
import {
  checkedSavedFileChecks, checkedSavedFiles, checkedSavedFilesLimit, checkedSavedLimitChange, savedListRowView,
} from "./saved-files.ts";

test("oversized recovery screenshot uses the real continuity decoder and preserves its original", () => {
  const original = JSON.parse(visualFixtureResponse("get_ui_state", {}, false, false, true) as string);
  const state = sanitizeUiContinuity(original);
  assert.equal(state.version, 2);
  assert.deepEqual(state.pendingSends, original.pendingSends);
  const entries = Object.values(state.pendingSends);
  assert.equal(entries.length, 1);
  assert.ok(new TextEncoder().encode(entries[0].text).length > 64 * 1024);
  assert.equal(entries[0].resolution, "recover");
  assert.equal(entries[0].acceptance, "ambiguous");
});

test("recovery screenshot opt-in leaves ordinary chat fixture continuity unchanged", () => {
  const ordinary = JSON.parse(visualFixtureResponse("get_ui_state") as string);
  assert.equal(ordinary.version, 1);
  assert.equal(ordinary.pendingSends, undefined);
  assert.deepEqual(ordinary.drafts, { "1:1": "A deterministic composer draft" });
});

test("the Transfers fixture shows every row state, closed and path free", () => {
  // Transfers UX (PR #51): `?fixture=chat&transfers=1` lists one row per state for screenshots.
  const rows = checkedPrivateDownloads(visualFixtureResponse("get_private_downloads", { server: 1 }, false, true));
  assert.equal(rows.length, VISUAL_TRANSFER_ROWS.length);
  const statuses = new Set(rows.map((row) => row.status));
  for (const status of ["receiving", "incomplete", "ready", "paused", "exhausted", "blocked", "removing", "saving",
    "save-interrupted", "save-paused", "destination-unavailable", "destination-changed", "saved"]) {
    assert.ok(statuses.has(status as never), status);
  }
  const checks = rows.filter((row) => row.status === "saved").map((row) =>
    checkedSavedCheck(visualFixtureResponse("check_saved_download",
      { server: 1, download: row.id, expectedIntent: row.intent }, false, true)));
  assert.deepEqual([...checks].sort(), ["missing", "present", "replaced", "unknown"]);
  // Every row has a listed name, and the rows fill all three groups.
  const files = visualFixtureResponse("get_files", { server: 1 }, false, true) as { files: { cid: string; name: string }[] };
  const views = rows.map((row) => {
    const name = files.files.find((file) => file.cid === row.cid)?.name;
    assert.ok(name, row.status);
    const check = row.status === "saved"
      ? visualFixtureResponse("check_saved_download", { download: row.id }, false, true) as SavedCheck : null;
    return savedRowView(row, check, name!, null, { busy: false, saving: false });
  });
  assert.deepEqual([...new Set(views.map((view) => view.group))].sort(), ["needs", "progress", "saved"]);
  assert.equal(new Set(views.map((view) => `${view.state}:${view.check}`)).size, VISUAL_TRANSFER_ROWS.length);
  assert.equal(visualFixtureResponse("reveal_saved_download", { server: 1, download: rows[0].id, expectedIntent: "1" }, false, true), null);
  assert.doesNotMatch(JSON.stringify(rows), /path|folder|[A-Z]:\\|\/Users\//);
  // Without the flag the chat fixture stays as it was.
  assert.deepEqual(visualFixtureResponse("get_private_downloads"), []);
  assert.equal(visualFixtureResponse("reveal_saved_download", { server: 1, download: "ab".repeat(32), expectedIntent: "1" }), null);
});

test("the Transfers fixture lists saved files in every list-row state, display data only", () => {
  // PR #56: the saved-files list, its batch check and the chat card that shows the mark.
  const list = checkedSavedFiles(visualFixtureResponse("get_saved_files", { server: 1 }, false, true));
  assert.equal(list.state, "ok");
  assert.equal(list.files.length, VISUAL_SAVED_FILES.length);
  const answers = checkedSavedFileChecks(visualFixtureResponse("check_saved_files", { server: 1 }, false, true));
  assert.deepEqual(answers.map((a) => a.check).sort(), ["missing", "present", "replaced", "unknown"]);
  const listed = visualFixtureResponse("get_files", { server: 1 }, false, true) as { files: { cid: string }[] };
  const views = list.files.map((file) => savedListRowView(file, answers.find((a) => a.id === file.id)?.check ?? null,
    listed.files.some((f) => f.cid === file.cid), () => "23 Oct", { busy: false, saving: false }));
  // Present, missing, replaced, could not check, and a folder Mewtual cannot check.
  assert.equal(new Set(views.map((view) => `${view.word}:${view.hint}`)).size, 5);
  assert.ok(views.some((view) => view.actions.some((action) => action.kind === "again")));
  assert.doesNotMatch(JSON.stringify(list), /path|folder|[A-Z]:\\|\/Users\//);
  for (const command of ["reveal_saved_file", "forget_saved_file", "reset_saved_files"]) {
    assert.equal(visualFixtureResponse(command, { server: 1, entry: list.files[0].id }, false, true), null);
  }
  // A chat message shares the first saved file, so its card shows the mark.
  const messages = visualFixtureResponse("get_messages", { server: 1, channel: "general" }, false, true) as { text: string }[];
  assert.ok(messages.some((m) => m.text.includes(`(file:${VISUAL_SAVED_FILES[0].file.cid})`)));
  // Without the flag the chat fixture has saved nothing and its messages are unchanged.
  assert.deepEqual(visualFixtureResponse("get_saved_files"), { state: "ok", files: [] });
  assert.deepEqual(visualFixtureResponse("check_saved_files"), []);
  assert.ok(!(visualFixtureResponse("get_messages", { server: 1, channel: "general" }) as { text: string }[])
    .some((m) => m.text.includes("(file:")));
});

test("the fixture answers the Remember saved files setting like native, clamped and never persisted", () => {
  const view = checkedSavedFilesLimit(visualFixtureResponse("get_saved_files_limit", { server: 1 }));
  assert.deepEqual(view, { state: "ok", limit: 256, listed: 212, min: 16, max: 4096, default: 256 });
  // A lower choice is previewed with what it forgets; out-of-range choices are clamped.
  assert.deepEqual(checkedSavedLimitChange(visualFixtureResponse("set_saved_files_limit",
    { server: 1, limit: 128, apply: false, expectedForgets: null })), { limit: 128, forgets: 84, applied: false });
  assert.deepEqual(checkedSavedLimitChange(visualFixtureResponse("set_saved_files_limit",
    { server: 1, limit: 1, apply: true, expectedForgets: 196 })), { limit: 16, forgets: 196, applied: true });
  assert.deepEqual(checkedSavedLimitChange(visualFixtureResponse("set_saved_files_limit",
    { server: 1, limit: 99999, apply: true, expectedForgets: 0 })), { limit: 4096, forgets: 0, applied: true });
  // An apply for another count than the choice forgets changes nothing, as natively.
  assert.throws(() => visualFixtureResponse("set_saved_files_limit", { server: 1, limit: 128, apply: true, expectedForgets: 80 }),
    /changed meanwhile/);
  assert.equal(checkedSavedFilesLimit(visualFixtureResponse("get_saved_files_limit", { server: 1 })).limit, 256);
});

test("visual fixture explicitly supports native call controller lifecycle", () => {
  const candidate = visualFixtureResponse("prepare_call_mapping_controller");
  assert.equal(visualFixtureResponse("activate_call_mapping_owner", { candidate }), candidate);
  assert.equal(visualFixtureResponse("end_call_mapping_owner", { owner: candidate }), null);
  assert.equal(visualFixtureResponse("unmap_call_port", { owner: candidate, grant: "fixture-grant" }), null);
});

test("visual fixture resumes into a deterministic unlocked server", () => {
  const servers = visualFixtureResponse("resume_session") as Array<{
    server: number;
    name: string;
    channels: Array<{ id: string; name: string }>;
  }>;

  assert.equal(servers[0]?.name, "Lantern Room");
  assert.deepEqual(servers[0]?.channels.map((channel) => channel.id), ["1", "2", "3"]);
  assert.deepEqual(visualFixtureResponse("get_channels", { server: 1 }), servers[0].channels);
  assert.deepEqual(visualFixtureResponse("get_messages", { server: 1, channel: "1" }),
    visualFixtureResponse("get_messages", { server: 1, channel: "general" }));
  assert.deepEqual(visualFixtureResponse("get_messages", { server: 2, channel: "1" }),
    visualFixtureResponse("get_messages", { server: 2, channel: "dm" }));
});

test("visual fixture returns isolated channel message copies", () => {
  const first = visualFixtureResponse("get_messages", { server: 1, channel: "general" }) as Array<{
    id: string;
    ts: number;
  }>;
  const second = visualFixtureResponse("get_messages", { server: 1, channel: "general" }) as Array<{
    id: string;
    ts: number;
  }>;

  assert.equal(first.at(-1)?.id, "msg-5");
  assert.equal(first.at(-1)?.ts, VISUAL_FIXTURE_NOW - 7 * 60_000);
  first.pop();
  assert.equal(second.length, 5);
});

test("visual fixture supplies every read used by the empty Wiki and lazy Help route", () => {
  assert.deepEqual(visualFixtureResponse("get_wiki_pages"), []);
  assert.deepEqual(visualFixtureResponse("get_wiki_map"), {});
  assert.deepEqual(visualFixtureResponse("get_wiki_meta"), {});
  assert.equal(visualFixtureResponse("get_wiki_review_days"), 0);
  assert.deepEqual(visualFixtureResponse("get_wiki_pending"), []);
});

test("visual fixture supplies the full degraded connectivity assistant state", () => {
  const report = visualFixtureResponse("get_connectivity") as {
    action: string;
    advertised: string[];
    upnp: string;
    steps: unknown[];
  };
  assert.equal(report.action, "found");
  assert.equal(report.advertised.length, 2);
  assert.match(report.upnp, /PCP unavailable/);
  assert.equal(report.steps.length, 2);
  const routes = visualFixtureResponse("get_member_routes", { server: 1 }) as Array<{
    indirect_health: string;
    indirect_witnesses: number;
    reciprocal_pending: boolean;
  }>;
  assert.equal(routes[1]?.indirect_health, "reachable_via_member");
  assert.equal(routes[1]?.indirect_witnesses, 1);
  assert.equal(routes[1]?.reciprocal_pending, true);
  assert.equal(visualFixtureResponse("manual_fallback_redial", { server: 1 }), "submitted");
});

test("visual fixture exposes explicit standing switchboard consent and hosts", () => {
  const status = visualFixtureResponse("get_switchboard_status", { server: 1 }) as {
    offered: boolean;
    eligible: boolean;
    online: Array<{ fingerprint: string }>;
  };
  assert.equal(status.offered, false);
  assert.equal(status.eligible, true);
  assert.equal(status.online.length, 2);
});

test("visual fixture fails loudly for an unsupported native dependency", () => {
  assert.throws(
    () => visualFixtureResponse("new_native_command"),
    /does not implement Tauri command: new_native_command/,
  );
});

test("file fixture saves to Downloads, treats Save as as a closed dialog and refuses any path", () => {
  assert.deepEqual(
    visualFixtureResponse("save_group_file", { server: 1, cid: "55aa", name: "notes.md" }),
    // The native result asserts a one-click save is in Downloads (picker re-review LOW 1b).
    { path: "C:\\Users\\fixture\\Downloads\\notes.md", displayed: true, inDownloads: true },
  );
  assert.throws(
    () => visualFixtureResponse("save_group_file", { server: 1, cid: "55aa", name: "notes.md", saveAs: true }),
    /^Error: Save as was cancelled\.$/,
  );
  for (const key of ["path", "folder", "destination"]) {
    assert.throws(
      () => visualFixtureResponse("save_group_file", { server: 1, cid: "55aa", name: "n", [key]: "C:\\x" }),
      /never accepts a path/,
    );
  }
});

test("file fixture separates cached bytes from explicitly kept copies", () => {
  assert.deepEqual(visualFixtureResponse("get_private_downloads"), []);
  assert.equal(visualFixtureResponse("check_saved_download", { server: 1, download: "ab".repeat(32), expectedIntent: "1" }), "present");
  const health = visualFixtureResponse("get_storage_health") as { referenced_chunks: number; verified_chunks: number; categories: unknown[] };
  assert.equal(health.verified_chunks, health.referenced_chunks);
  assert.deepEqual(health.categories, []);
  assert.equal(visualFixtureResponse("file_available"), true);
  assert.deepEqual(visualFixtureResponse("get_kept_files"), {
    supported: true, allocated_bytes: 0, limit_bytes: 1073741824, files: [], transfers: [], error: null,
  });
  assert.deepEqual(visualFixtureResponse("get_file_usage"), {
    wiki_pages: [], status_count: 0, chat_count: 0, event_count: 0, pinned: false,
  });
});

test("cache fixture shows a nearly full server and a calm DM, both valid native statuses", async () => {
  const { checkedCacheStatus, cacheFooter } = await import("./cache-status.ts");
  const lantern = checkedCacheStatus(visualFixtureResponse("get_cache_status", { server: 1 }));
  assert.ok(lantern, "the fixture status passes the same validation as native's");
  assert.equal(lantern.nearly_full, true);
  assert.equal(cacheFooter(lantern).chip, "3 leaving soon");
  const dm = checkedCacheStatus(visualFixtureResponse("get_cache_status", { server: 2 }));
  assert.equal(dm?.nearly_full, false);
  assert.equal(cacheFooter(dm!).chip, null);
  const unlimited = checkedCacheStatus(visualFixtureResponse("set_cache_limit", { server: 1, bytes: null }));
  assert.equal(unlimited?.unlimited, true);
  assert.equal(unlimited?.leaving_soon_files, 0);
  const health = visualFixtureResponse("get_storage_health") as { not_cached_chunks: number };
  assert.equal(health.not_cached_chunks, 0);
});

test("visual fixture role maps name exact roster identities", () => {
  const members = visualFixtureResponse("get_members", { server: 1 }) as Array<{ identity: string; you: boolean; fingerprint: string }>;
  const roles = visualFixtureResponse("get_roles", { server: 1 }) as Record<string, string>;
  const me = members.find((m) => m.you)!;
  assert.equal(roles[me.identity], "owner");
  assert.equal(roles[me.fingerprint], undefined);
});
