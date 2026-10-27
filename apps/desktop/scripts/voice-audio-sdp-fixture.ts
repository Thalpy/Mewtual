/** Controlled SDK fixtures only: enough structure for the shipped audio profile guard,
 * not proof of browser SDP/ICE/DTLS negotiation. The real-browser gate generates real SDP. */
export const VOICE_AUDIO_FIXTURE_SDP = "v=0\r\no=- 1 2 IN IP4 127.0.0.1\r\ns=-\r\nt=0 0\r\na=group:BUNDLE 0\r\n" +
  "m=audio 9 UDP/TLS/RTP/SAVPF 111\r\nc=IN IP4 0.0.0.0\r\na=mid:0\r\na=rtcp-mux\r\na=rtpmap:111 opus/48000/2\r\n";
const padding = "a=x-fixture:";
/** Preserve the actual8KiB-byte boundary while exercising a supported audio shape. */
export const VOICE_AUDIO_FIXTURE_MAX_SDP = VOICE_AUDIO_FIXTURE_SDP + padding +
  "s".repeat(8192 - VOICE_AUDIO_FIXTURE_SDP.length - padding.length - 2) + "\r\n";
