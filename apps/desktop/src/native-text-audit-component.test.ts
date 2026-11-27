import test from "node:test";
import assert from "node:assert/strict";
import { flushSync, mount, unmount } from "svelte";
import NativeTextAudit from "./NativeTextAudit.svelte";
import type { NativeAuditRecord, NativeOriginal } from "./native-text-history.ts";
import { createVisualNativeTextFixture } from "./visual-native-text-fixture.ts";

test("the Chat V2 owner audit renders saved edits as text and clears them when closed", async () => {
  const invokeFixture = createVisualNativeTextFixture();
  const calls: Record<string, unknown>[] = [];
  let pauseOwn = false;
  let paginateOwn = false;
  let finishOwn: (() => void) | undefined;
  const native = {
    async invoke(command: string, args: Record<string, unknown>) {
      calls.push(args);
      const reply = invokeFixture(command, args);
      assert.ok(reply.handled);
      if (typeof args.auditAction === "string") {
        const records = (reply.value as { items: NativeAuditRecord[] }).items;
        const edit = records.find(record => record.body.kind === "edit")?.body;
        if (edit?.kind === "edit") edit.text += " <img src=x onerror=alert(1)>";
      }
      if ("start" in args && pauseOwn) {
        await new Promise<void>(resolve => { finishOwn = resolve; });
      }
      if ("start" in args && paginateOwn) {
        const create = "b1".repeat(32);
        const actions: NativeOriginal["body"][] = [
          { kind: "edit", create, text: "The earlier local edit." },
          { kind: "delete", create },
          { kind: "join", create },
        ];
        const items: NativeOriginal[] = args.start === "8" ? actions.map((body, index) => ({
          ordinal: String(8 + index), action: String(index + 1).repeat(64),
          signer: "c1".repeat(32), channel: "1", presented_ms: "1787227200000", status: "submitted", body,
        })) : [];
        return { backend: "text-v2", scope: "own-saved-originals",
          next: args.start === "0" ? "8" : "12", end_exclusive: "12", items };
      }
      return reply.value;
    },
  };
  Object.defineProperty(window, "__TAURI_INTERNALS__", { value: native, configurable: true });
  Object.defineProperty(globalThis, "HTMLMediaElement", { value: window.HTMLMediaElement, configurable: true });
  const target = document.createElement("div");
  document.body.append(target);
  const instance = mount(NativeTextAudit, {
    target,
    props: { server: 1, channel: "1", context: "a1".repeat(32) },
  });
  const button = (text: string) => {
    const found = [...target.querySelectorAll("button")].find(node => node.textContent?.trim() === text);
    assert.ok(found, `missing button ${text}`);
    return found;
  };
  const settle = async () => {
    for (let i = 0; i < 8; i++) {
      await Promise.resolve();
      flushSync();
    }
  };
  try {
    await settle();
    const dialog = target.querySelector("dialog");
    assert.ok(dialog);
    // jsdom models the DOM but leaves the browser's modal methods unimplemented.
    dialog.showModal = () => dialog.setAttribute("open", "");
    dialog.close = () => dialog.removeAttribute("open");
    button("Chat history").click();
    await settle();
    assert.ok(dialog.open);
    assert.match(dialog.textContent ?? "", /A message recovered after reconnecting/);
    button("View changes").click();
    await settle();
    assert.match(dialog.textContent ?? "", /A retained edit from the same device/);
    assert.match(dialog.textContent ?? "", /<img src=x onerror=alert\(1\)>/);
    assert.equal(dialog.querySelector("img"), null);
    assert.match(dialog.textContent ?? "", /Signed record details/);
    assert.deepEqual(calls.map(call => call.auditAction), [null, "d3".repeat(32)]);
    button("Saved on this device").click();
    await settle();
    assert.match(dialog.textContent ?? "", /A message saved on this device/);
    assert.match(dialog.textContent ?? "", /Saved here/);
    assert.match(dialog.textContent ?? "", /Held on this device/);
    assert.doesNotMatch(dialog.textContent ?? "", /A retained edit from the same device/);
    assert.deepEqual(calls.at(-1), {
      server: 1, channel: "1", expectedContext: "a1".repeat(32),
      start: "0", endExclusive: null, limit: 8,
    });
    pauseOwn = true;
    button("Saved on this device").click();
    await settle();
    assert.equal(typeof finishOwn, "function");
    button("Received history").click();
    await settle();
    finishOwn?.();
    await settle();
    assert.match(dialog.textContent ?? "", /A message recovered after reconnecting/);
    assert.doesNotMatch(dialog.textContent ?? "", /A message saved on this device/);
    paginateOwn = true;
    pauseOwn = false;
    button("Saved on this device").click();
    await settle();
    assert.match(dialog.textContent ?? "", /No originals saved on this device on this page/);
    button("Next page").click();
    await settle();
    assert.equal(calls.at(-1)?.start, "8");
    assert.equal(calls.at(-1)?.endExclusive, "12");
    assert.match(dialog.textContent ?? "", /The earlier local edit/);
    assert.match(dialog.textContent ?? "", /Deletion/);
    assert.match(dialog.textContent ?? "", /Combined revisions/);
    assert.match(dialog.textContent ?? "", /Submitted/);
    pauseOwn = true;
    paginateOwn = false;
    finishOwn = undefined;
    button("First page").click();
    await settle();
    assert.equal(typeof finishOwn, "function");
    button("Close").click();
    finishOwn?.();
    await settle();
    assert.doesNotMatch(dialog.textContent ?? "", /The earlier local edit/);
    assert.ok(!dialog.open);
    assert.doesNotMatch(dialog.textContent ?? "", /A message saved on this device/);
    assert.doesNotMatch(dialog.textContent ?? "", /A retained edit from the same device/);
    button("Chat history").click();
    dialog.dispatchEvent(new window.Event("close"));
    await settle();
    assert.ok(dialog.open);
    assert.match(dialog.textContent ?? "", /A message recovered after reconnecting/);
  } finally {
    await unmount(instance);
    target.remove();
    Reflect.deleteProperty(window, "__TAURI_INTERNALS__");
    Reflect.deleteProperty(globalThis, "HTMLMediaElement");
  }
});
