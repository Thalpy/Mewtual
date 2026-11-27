import test from "node:test";
import assert from "node:assert/strict";
import { flushSync, mount, unmount } from "svelte";
import NativeTextAudit from "./NativeTextAudit.svelte";
import type { NativeAuditRecord } from "./native-text-history.ts";
import { createVisualNativeTextFixture } from "./visual-native-text-fixture.ts";

test("the Chat V2 owner audit renders saved edits as text and clears them when closed", async () => {
  const invokeFixture = createVisualNativeTextFixture();
  const calls: Record<string, unknown>[] = [];
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
    button("Close").click();
    await settle();
    assert.ok(!dialog.open);
    assert.doesNotMatch(dialog.textContent ?? "", /A retained edit from the same device/);
  } finally {
    await unmount(instance);
    target.remove();
    Reflect.deleteProperty(window, "__TAURI_INTERNALS__");
    Reflect.deleteProperty(globalThis, "HTMLMediaElement");
  }
});
