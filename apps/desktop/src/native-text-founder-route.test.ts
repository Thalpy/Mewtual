import assert from "node:assert/strict";
import test from "node:test";
import { flushSync, mount, unmount } from "svelte";
import NativeTextFounderRoute from "./NativeTextFounderRoute.svelte";
import { createVisualNativeTextFixture } from "./visual-native-text-fixture.ts";

test("explicit founder address commands use only the original row and ignore a retired view", async () => {
  const fixture = createVisualNativeTextFixture(false, true);
  const context = "a1".repeat(32);
  const calls: Array<{ command: string; args: Record<string, unknown> }> = [];
  let finish: (() => void) | undefined;
  let pause = false;
  let live = true;
  const native = {
    async invoke(command: string, args: Record<string, unknown>) {
      calls.push({ command, args });
      const reply = fixture(command, args);
      assert.ok(reply.handled);
      if (pause) await new Promise<void>(resolve => { finish = resolve; });
      return reply.value;
    },
  };
  const saved = Object.getOwnPropertyDescriptor(window, "__TAURI_INTERNALS__");
  const savedMedia = Object.getOwnPropertyDescriptor(globalThis, "HTMLMediaElement");
  Object.defineProperty(window, "__TAURI_INTERNALS__", { value: native, configurable: true });
  Object.defineProperty(globalThis, "HTMLMediaElement", { value: window.HTMLMediaElement, configurable: true });
  const target = document.createElement("div");
  document.body.append(target);
  let active = mount(NativeTextFounderRoute, { target, props: { server: 1, context, isUiLive: () => live } });
  const button = (label: string) => {
    const result = [...target.querySelectorAll("button")].find(node => node.textContent?.trim() === label);
    assert.ok(result);
    return result;
  };
  const fill = (invite: string, host: string) => {
    const inputs = [...target.querySelectorAll("input")];
    assert.equal(inputs.length, 2);
    for (const [index, value] of [invite, host].entries()) {
      inputs[index].value = value;
      inputs[index].dispatchEvent(new window.Event("input", { bubbles: true }));
    }
    flushSync();
  };
  const settle = async () => {
    for (let i = 0; i < 6; i++) { await Promise.resolve(); flushSync(); }
  };
  try {
    await settle();
    const details = target.querySelector("details");
    assert.ok(details);
    assert.equal(details.open, false);
    details.open = true;
    const remember = button("Connect and remember");
    assert.ok(remember.disabled);
    assert.deepEqual(calls, []);
    const inviteLimit = target.querySelector("input")!.maxLength;
    const form = target.querySelector("form");
    assert.ok(form);
    assert.equal(inviteLimit, 6254);
    fill("a".repeat(inviteLimit + 1), "2001:db8::1");
    assert.ok(remember.disabled);
    form.dispatchEvent(new window.Event("submit", { bubbles: true, cancelable: true }));
    await settle();
    assert.deepEqual(calls, []);
    fill("ab0123", "1".repeat(46));
    assert.ok(remember.disabled);
    form.dispatchEvent(new window.Event("submit", { bubbles: true, cancelable: true }));
    await settle();
    assert.deepEqual(calls, []);
    fill(" ab0123 ", " 2001:db8::1 ");
    remember.click();
    await settle();
    assert.deepEqual(calls[0], {
      command: "remember_native_chat_founder_route",
      args: { server: 1, expectedContext: context, inviteHex: "ab0123", approvedHost: "2001:db8::1" },
    });
    assert.equal(target.querySelector('[role="status"]')?.textContent, "Founder address saved on this device.");
    assert.ok(remember.disabled);

    button("Forget founder address").click();
    await settle();
    assert.deepEqual(calls[1], {
      command: "forget_native_chat_founder_route",
      args: { server: 1, expectedContext: context },
    });
    assert.equal(target.querySelector('[role="status"]')?.textContent, "Founder address cleared.");

    live = false;
    fill(" ab0123 ", " 2001:db8::1 ");
    remember.click();
    await settle();
    assert.equal(calls.length, 2);
    live = true;

    pause = true;
    fill("a".repeat(inviteLimit), "2001:db8::1");
    remember.click();
    await settle();
    assert.ok(remember.disabled);
    assert.ok(button("Forget founder address").disabled);
    assert.equal(typeof finish, "function");
    assert.equal(String(calls[2].args.inviteHex).length, inviteLimit);
    await unmount(active);
    active = mount(NativeTextFounderRoute, {
      target, props: { server: 2, context: "b2".repeat(32), isUiLive: () => true },
    });
    finish?.();
    await settle();
    assert.equal(target.querySelector('[role="status"]'), null);
    assert.equal(calls.length, 3);
  } finally {
    await unmount(active);
    target.remove();
    if (saved) Object.defineProperty(window, "__TAURI_INTERNALS__", saved);
    else Reflect.deleteProperty(window, "__TAURI_INTERNALS__");
    if (savedMedia) Object.defineProperty(globalThis, "HTMLMediaElement", savedMedia);
    else Reflect.deleteProperty(globalThis, "HTMLMediaElement");
  }
});
