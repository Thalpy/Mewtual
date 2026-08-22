import { test } from "node:test";
import assert from "node:assert/strict";

import {
  MAX_CUSTOM_TONE_BYTES, MAX_CUSTOM_TONE_SECONDS, customToneError, customToneMime,
  defaultGlobalSoundPrefs, defaultServerSoundPrefs, parseGlobalSoundPrefs, parseServerSoundPrefs,
  resolveNotificationSound, type StoredTone,
} from "./notification-sounds.ts";

const tone: StoredTone = {
  name: "tiny.mp3",
  mime: "audio/mpeg",
  dataUrl: "data:audio/mpeg;base64,SUQz",
};

test("custom tone validation accepts common audio and bounds localStorage cost and duration", () => {
  assert.equal(customToneMime("audio/mpeg", "anything.bin"), "audio/mpeg");
  assert.equal(customToneMime("", "RCTTone.MP3"), "audio/mpeg");
  assert.equal(customToneMime("application/octet-stream", "cue.ogg"), "audio/ogg");
  assert.equal(customToneMime("", "cue.exe"), null);
  assert.equal(customToneError("audio/mpeg", 29_256, 1.8), null);
  assert.match(customToneError(null, 20, 1) ?? "", /MP3/);
  assert.match(customToneError("text/html", 20, 1) ?? "", /MP3/);
  assert.match(customToneError("audio/mpeg", MAX_CUSTOM_TONE_BYTES + 1, 1) ?? "", /KiB/);
  assert.match(customToneError("audio/mpeg", 20, MAX_CUSTOM_TONE_SECONDS + 0.01) ?? "", /seconds/);
});

test("global preferences parse valid custom tones and reject malformed persisted data", () => {
  const parsed = parseGlobalSoundPrefs(JSON.stringify({
    message: { enabled: false, tone: "custom", custom: tone },
    mention: { enabled: "no", tone: "custom", custom: { ...tone, dataUrl: "javascript:alert(1)" } },
    news: { enabled: true, tone: "default", custom: tone },
  }));
  assert.equal(parsed.message.enabled, false);
  assert.equal(parsed.message.tone, "custom");
  assert.deepEqual(parsed.message.custom, tone);
  assert.equal(parsed.mention.enabled, true);
  assert.equal(parsed.mention.tone, "default");
  assert.equal(parsed.mention.custom, null);
  assert.equal(parsed.news.tone, "default");
  assert.deepEqual(parsed.news.custom, tone); // choosing Built-in does not delete an imported file
  assert.deepEqual(parseGlobalSoundPrefs("{"), defaultGlobalSoundPrefs());
});

test("server preferences accept only the override vocabulary", () => {
  const parsed = parseServerSoundPrefs(JSON.stringify({
    news: { enabled: "off", tone: "custom", custom: tone },
    message: { enabled: true, tone: "wat", custom: tone },
  }));
  assert.equal(parsed.news.enabled, "off");
  assert.equal(parsed.news.tone, "custom");
  assert.equal(parsed.message.enabled, "inherit");
  assert.equal(parsed.message.tone, "inherit");
  assert.deepEqual(parsed.message.custom, tone); // retained so choosing Custom later is instant
  assert.deepEqual(parseServerSoundPrefs("null"), defaultServerSoundPrefs());
});

test("resolution applies master, enable, and tone precedence", () => {
  const global = defaultGlobalSoundPrefs();
  const server = defaultServerSoundPrefs();
  global.news.tone = "custom";
  global.news.custom = tone;

  assert.deepEqual(resolveNotificationSound(true, global, server, "news"), {
    enabled: true, custom: tone, builtIn: "default", source: "global custom",
  });
  server.news.tone = "default";
  assert.deepEqual(resolveNotificationSound(true, global, server, "news"), {
    enabled: true, custom: null, builtIn: "default", source: "built-in",
  });
  server.news.tone = "custom";
  server.news.custom = { ...tone, name: "server.mp3" };
  assert.equal(resolveNotificationSound(true, global, server, "news").source, "server custom");
  server.news.enabled = "off";
  assert.equal(resolveNotificationSound(true, global, server, "news").enabled, false);
  server.news.enabled = "on";
  global.news.enabled = false;
  assert.equal(resolveNotificationSound(true, global, server, "news").enabled, true);
  assert.equal(resolveNotificationSound(false, global, server, "news").enabled, false);
});

test("the crunch built-in survives persistence and wins tone precedence like default does", () => {
  const global = parseGlobalSoundPrefs(JSON.stringify({
    message: { enabled: true, tone: "crunch" },
  }));
  assert.equal(global.message.tone, "crunch");

  // Global crunch reaches resolution when no server override intervenes.
  assert.deepEqual(resolveNotificationSound(true, global, null, "message"), {
    enabled: true, custom: null, builtIn: "crunch", source: "built-in",
  });

  // A server-level crunch overrides a global custom tone, same as "default" does.
  global.message.tone = "custom";
  global.message.custom = tone;
  const server = parseServerSoundPrefs(JSON.stringify({
    message: { enabled: "inherit", tone: "crunch" },
  }));
  assert.equal(server.message.tone, "crunch");
  assert.deepEqual(resolveNotificationSound(true, global, server, "message"), {
    enabled: true, custom: null, builtIn: "crunch", source: "built-in",
  });
});
