import { readFileSync } from "node:fs";
import assert from "node:assert/strict";
import test from "node:test";
import { voiceAudioSdpShape } from "./voice-audio-profile.ts";

const sdp = (session = "a=group:BUNDLE 0\r\n", media = "") => "v=0\r\no=- 1 2 IN IP4 127.0.0.1\r\ns=-\r\nt=0 0\r\n" +
  session + "m=audio 9 UDP/TLS/RTP/SAVPF 111 0 8\r\nc=IN IP4 0.0.0.0\r\na=mid:0\r\na=rtcp-mux\r\n" +
  "a=rtpmap:111 opus/48000/2\r\na=ice-ufrag:ufrag\r\na=ice-pwd:password\r\na=setup:actpass\r\n" +
  "a=msid:- streamless-track\r\na=ssrc:123 cname:cname\r\na=extmap:1 urn:example:test\r\n" + media;

test("normal streamless browser audio has one small frozen shape and preserves the input", () => {
  const input = sdp(), before = input;
  const shape = voiceAudioSdpShape(input);
  assert.deepEqual(shape, { mid: "0", protocol: "UDP/TLS/RTP/SAVPF" });
  assert.ok(Object.isFrozen(shape)); assert.equal(input, before);
  assert.deepEqual(voiceAudioSdpShape(input.replaceAll("\r\n", "\n")), shape);
  assert.deepEqual(voiceAudioSdpShape(sdp("")), shape, "one audio section does not require BUNDLE");
});

for (const declared of ["", "sendrecv", "sendonly", "recvonly", "inactive"]) {
  test(`ordinary direction inheritance and override are accepted: ${declared || "default"}`, () => {
    const line = declared ? `a=${declared}\r\n` : "";
    assert.equal(voiceAudioSdpShape(sdp(line)).mid, "0");
    assert.equal(voiceAudioSdpShape(sdp(line, "a=recvonly\r\n")).mid, "0");
    assert.equal(voiceAudioSdpShape(sdp("", line)).mid, "0");
  });
}

test("extra audio, video and application sections are refused even when rejected", () => {
  for (const kind of ["audio", "video", "application"]) for (const port of ["0", "9"]) {
    assert.throws(() => voiceAudioSdpShape(sdp() + `m=${kind} ${port} UDP/TLS/RTP/SAVPF 111\r\n`), /unsupported voice audio description/);
  }
});

test("MID, one BUNDLE reference and RTCP multiplexing are required without duplicating codec negotiation", () => {
  for (const bad of [
    sdp().replace("a=mid:0\r\n", ""), sdp().replace("a=rtcp-mux\r\n", ""),
    sdp().replace("a=mid:0", "a=mid:0\r\na=mid:1"), sdp().replace("a=mid:0", "a=mid:bad mid"),
    sdp("a=group:BUNDLE 0 1\r\n"), sdp("a=group:BUNDLE 1\r\n"), sdp("a=group:BUNDLE\r\n"),
    sdp("a=group:BUNDLE 0\r\na=group:BUNDLE 0\r\n"), sdp("", "a=group:BUNDLE 0\r\n"),
    sdp("a=rtcp-mux\r\n"), sdp().replace("m=audio 9", "m=audio 0"),
    sdp().replace("m=audio 9", "m=audio 65536"), sdp().replace("m=audio 9", "m=audio 9/2"),
    sdp().replace("UDP/TLS/RTP/SAVPF", "RTP/AVP"),
    sdp("a=sendrecv\r\na=inactive\r\n"), sdp("", "a=sendonly\r\na=recvonly\r\n"),
  ]) assert.throws(() => voiceAudioSdpShape(bad), /unsupported voice audio description/);
  assert.equal(voiceAudioSdpShape(sdp().replace("opus/48000/2", "future-codec/48000/2")).mid, "0", "browser owns codec validation");
});

test("answer must preserve the original offer's MID and RTP transport profile", () => {
  const offer = voiceAudioSdpShape(sdp());
  assert.deepEqual(voiceAudioSdpShape(sdp("", "a=recvonly\r\n"), offer), offer);
  assert.throws(() => voiceAudioSdpShape(sdp().replaceAll("mid:0", "mid:1").replace("BUNDLE 0", "BUNDLE 1"), offer));
  assert.throws(() => voiceAudioSdpShape(sdp(), { mid: "0", protocol: "wrong" } as unknown as typeof offer));
});

test("profile boundaries accept port65535/MID64 and intentionally refuse TCP, empty formats and MID65", () => {
  const limit = "x".repeat(64), atLimit = sdp().replace("m=audio 9", "m=audio 65535").replace("mid:0", "mid:" + limit).replace("BUNDLE 0", "BUNDLE " + limit);
  assert.equal(voiceAudioSdpShape(atLimit).mid, limit);
  for (const input of [atLimit.replaceAll(limit, limit + "x"),
    sdp().replace("111 0 8", ""), sdp().replace("UDP/TLS/RTP/SAVPF", "TCP/DTLS/RTP/SAVPF")])
    assert.throws(() => voiceAudioSdpShape(input), /unsupported voice audio description/);
});

test("bounded scanner refuses control/oversize/empty media while tolerating ordinary unknown attributes", () => {
  for (const input of ["", "x".repeat(8193), "v=0\r\n", sdp() + "a=unknown:\0bad\r\n", sdp() + "a=unknown:bad\rhidden\n",
    sdp() + " m=video 9 UDP/TLS/RTP/SAVPF 111\r\n", sdp() + "M=video 9 UDP/TLS/RTP/SAVPF 111\r\n"])
    assert.throws(() => voiceAudioSdpShape(input));
  assert.equal(voiceAudioSdpShape(sdp() + "a=unknown:" + "x".repeat(4096) + "\r\n").mid, "0");
});

test("retained MID materialization has no substring/rope backedge to the original SDP", () => {
  // Portable structural guard for a representation invariant: nondeterministic GC is
  // not an oracle. Edge/V8 can otherwise keep an8KiB parent through a13..64-char slice.
  const source = readFileSync(new URL("./voice-audio-profile.ts", import.meta.url), "utf8");
  const detached = source.slice(source.indexOf("function ownedToken("), source.indexOf("export function voiceAudioSdpShape("));
  assert.ok(detached.includes("charCodeAt") && detached.includes("String.fromCharCode(...bytes)"));
  assert.ok(!/\.(?:slice|substring|substr)\(/.test(detached), "cached token must be made from bounded code values");
  assert.ok(source.includes("mid = ownedToken(text, from + 6, until)"));
  const unicode = sdp().replace("BUNDLE 0", "BUNDLE \u0130").replace("mid:0", "mid:\u0130");
  assert.throws(() => voiceAudioSdpShape(unicode), "Unicode must not truncate into ASCII0");
});
