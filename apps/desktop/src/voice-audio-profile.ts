/** Frontend audio-only RTC shape, not an additional native/wire authentication
 * guarantee. CompleteSdpV1 specifies gathered SDP transport; it does not constrain
 * a remote endpoint to one audio receiver. The paid audio owner must call this on
 * the original already8KiB-byte-bounded text BEFORE a HIGH-side PC constructor or
 * any description SDK entry, then recheck its original currentness afterward.
 *
 * This is deliberately not a second SDP negotiator. The browser still validates
 * codecs, DTLS/ICE, candidates, direction negotiation and full SDP grammar. Keep
 * original bytes unchanged. Runtime transceiver/receiver checks are also required.
 * The inactive Source now enforces this before HIGH construction/remote SDK entry
 * and on gathered local output. Runtime track/currentDirection checks remain a
 * separate prerequisite for future audio actuation. */
export type VoiceAudioSdpShape = Readonly<{ mid: string; protocol: "UDP/TLS/RTP/SAVPF" }>;
// Persistent offer shape: <=128B UTF16 MID +string/object/field/alignment allowances.
// Parser/answer tokens,64B typed backing/view/buffer, <=64 numeric arguments and
// synchronous wrappers are serial with the original UTF16/SDK-description stage.
export const VOICE_AUDIO_SDP_RETAINED_BYTES = 224;
export const VOICE_AUDIO_SDP_PARSE_BYTES = 2048;
const unavailable = (): never => { throw new Error("unsupported voice audio description"); };
const token = /^[a-zA-Z0-9!#$%&'*+\-.^_`{|}~]{1,64}$/;

/** Detach a bounded ASCII token from its input backing. A cached String.slice MID
 * can retain the complete old8KiB SDP in Edge/V8. Construct from numeric code units,
 * never a substring/rope of that input. Scratch is <=64 bytes plus64 numeric arguments;
 * only the offer record persists in BOOK; temporary tokens/scratch belong to the
 * separate serial UTF16/SDK-text stage inventory. No limit or charge is increased. */
function ownedToken(text: string, from: number, until: number): string {
  const length = until - from;
  if (length < 1 || length > 64) return unavailable();
  const bytes = new Uint8Array(length);
  for (let index = 0; index < length; index++) {
    const code = text.charCodeAt(from + index);
    if (code > 127) return unavailable(); // Never truncate Unicode into a valid ASCII token.
    bytes[index] = code;
  }
  const owned = String.fromCharCode(...bytes);
  if (!token.test(owned)) return unavailable();
  return owned;
}

export function voiceAudioSdpShape(text: string, offer: VoiceAudioSdpShape | null = null): VoiceAudioSdpShape {
  if (typeof text !== "string" || text.length === 0 || text.length > 8192) return unavailable();
  let sections = 0, mid: string | null = null, bundle: string | null = null, mux = false;
  let sessionDirection = false, mediaDirection = false;
  // No whole-line copies: an unknown attribute or malformed m-line can occupy
  // almost all8KiB. Only MID/BUNDLE tokens (<=64) and a port (<=5) are copied.
  for (let from = 0; from < text.length;) {
    const newline = text.indexOf("\n", from);
    let until = newline < 0 ? text.length : newline;
    if (until > from && text.charCodeAt(until - 1) === 13) until--;
    for (let at = from; at < until; at++) {
      const code = text.charCodeAt(at);
      if ((code < 32 && code !== 9) || code === 127) return unavailable();
    }
    const length = until - from;
    const begins = (value: string) => length >= value.length && text.startsWith(value, from);
    const equals = (value: string) => length === value.length && text.startsWith(value, from);
    // Reject whitespace/folded or capitalized field framing before SDK parsing,
    // rather than hoping both parsers ignore an extra disguised media section.
    if (length && (length < 2 || text.charCodeAt(from) < 97 || text.charCodeAt(from) > 122 || text[from + 1] !== "=")) return unavailable();
    if (begins("m=")) {
      if (++sections !== 1 || !begins("m=audio ")) return unavailable();
      const portFrom = from + 8, portEnd = text.indexOf(" ", portFrom);
      if (portEnd < portFrom + 1 || portEnd > portFrom + 5 || portEnd >= until) return unavailable();
      const port = text.slice(portFrom, portEnd);
      const protocol = "UDP/TLS/RTP/SAVPF ", protocolFrom = portEnd + 1;
      const formats = protocolFrom + protocol.length;
      if (!/^[0-9]+$/.test(port) || Number(port) < 1 || Number(port) > 65535 ||
          formats >= until || !text.startsWith(protocol, protocolFrom) || /\s/.test(text[formats])) return unavailable();
    } else if (begins("a=mid:")) {
      if (sections !== 1 || mid !== null || length > 70) return unavailable();
      mid = ownedToken(text, from + 6, until);
    } else if (equals("a=rtcp-mux")) {
      if (sections !== 1) return unavailable();
      mux = true;
    } else if (begins("a=group:BUNDLE")) {
      if (sections !== 0 || bundle !== null || !begins("a=group:BUNDLE ") || length > 79) return unavailable();
      bundle = ownedToken(text, from + 15, until);
    } else if (equals("a=sendrecv") || equals("a=sendonly") || equals("a=recvonly") || equals("a=inactive")) {
      // Normal session default and media override are legal; absence is sendrecv.
      // Only exact negotiated runtime currentDirection can enable a sender.
      if (sections === 0) {
        if (sessionDirection) return unavailable();
        sessionDirection = true;
      } else {
        if (mediaDirection) return unavailable();
        mediaDirection = true;
      }
    }
    from = newline < 0 ? text.length : newline + 1;
  }
  if (sections !== 1 || mid === null || !mux || (bundle !== null && bundle !== mid)) return unavailable();
  if (offer && (offer.mid !== mid || offer.protocol !== "UDP/TLS/RTP/SAVPF")) return unavailable();
  return Object.freeze({ mid, protocol: "UDP/TLS/RTP/SAVPF" });
}
