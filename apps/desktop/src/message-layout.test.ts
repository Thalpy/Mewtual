import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const css = readFileSync(new URL("./app.css", import.meta.url), "utf8");

test("chat messages shrink and wrap tokens that have no natural break point", () => {
  const messageTextRule = css.match(/\.messages li \.text\s*\{(?<body>[^}]*)\}/)?.groups?.body;

  assert.ok(messageTextRule, "the chat message text rule should exist");
  assert.match(messageTextRule, /min-width:\s*0\s*;/);
  assert.match(messageTextRule, /max-width:\s*100%\s*;/);
  assert.match(messageTextRule, /overflow-wrap:\s*anywhere\s*;/);
});
