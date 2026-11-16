/** Canonical child Input, only after original-channel authentication and prepaid intake.
 * The expected binding belongs to the retained original derivation chain, never a new key
 * selection. Parsing these bytes does not admit an engine, consume a token or authorize RTC. */
import type { VoiceIpcActionIdentity } from "./voice-ipc-actions.ts";
import type { VoiceIpcBootstrap } from "./voice-ipc-bootstrap.ts";
import { isVoiceIpcActionRow } from "./voice-ipc-replies.ts";
import type { VoiceCryptoChainBinding, VoiceProtectedConfirmationInput } from "./voice-crypto.ts";

export type VoiceIpcConfirmationInputExpected = Readonly<{
  row: string; parent: string; identity: VoiceIpcActionIdentity;
  bootstrap: VoiceIpcBootstrap; binding: VoiceCryptoChainBinding;
}>;
const fail = (): never => { throw new Error("invalid native confirmation input"); };

/** Copies AAD/body only after every length, scope, direction and identity check. The caller
 * retains its same intake while transferring these bytes into the original crypto chain. */
export function decodeVoiceIpcConfirmationInput(bytes: Uint8Array,
  expected: VoiceIpcConfirmationInputExpected): VoiceProtectedConfirmationInput {
  if (!(bytes instanceof Uint8Array) || bytes.length < 430 || bytes.length > 2048 ||
      !isVoiceIpcActionRow(expected.row) || !isVoiceIpcActionRow(expected.parent) ||
      expected.row === expected.parent) return fail();
  let offset = 0;
  const take = (count: number) => {
    if (offset + count > bytes.length) return fail();
    const part = bytes.subarray(offset, offset + count); offset += count; return part;
  };
  const integer = (count: number) => { let n = 0n; for (const byte of take(count)) n = n * 256n + BigInt(byte); return n; };
  const hex = (count: number) => { let s = ""; for (const byte of take(count)) s += byte.toString(16).padStart(2, "0"); return s; };
  if (integer(1) !== 1n || integer(1) !== 7n || hex(16) !== expected.row) return fail();
  const action = integer(8).toString(), version = integer(8).toString(), inputTag = hex(32);
  if (action === "0" || version === "0" || action !== expected.identity.action ||
      version !== expected.identity.version || inputTag !== expected.identity.inputTag ||
      hex(16) !== expected.parent) return fail();
  const chain = BigInt("0x" + expected.parent.slice(16)).toString();
  const group = hex(32), policyFlag = integer(1), policyBytes = hex(32), room = integer(16);
  if (policyFlag > 1n || (policyFlag === 0n && policyBytes !== "0".repeat(64))) return fail();
  const policy = policyFlag === 1n ? policyBytes : null;
  const epoch = integer(8).toString(), localDevice = hex(32), localJoin = hex(32), localKey = hex(32);
  const peerDevice = hex(32), peerJoin = hex(32), peerKey = hex(32), handshake = hex(32);
  const generation = integer(8).toString(), nativeDeadline = integer(8).toString(), low = integer(1);
  const engineBytes = Number(integer(4)), operation = Number(integer(1)), row = Number(integer(1));
  const sequence = integer(8).toString(), aadLength = Number(integer(2)), bodyLength = Number(integer(2));
  if (low > 1n || generation === "0" || nativeDeadline === "0" || sequence === "0" ||
      row < 1 || row > 9 || group !== expected.bootstrap.group || policy !== expected.bootstrap.policy ||
      room !== expected.bootstrap.room || aadLength > 4096) return fail();
  const binding: VoiceCryptoChainBinding = { chain, epoch, localDevice, localJoin, localKey,
    peerDevice, peerJoin, peerKey, handshake, generation, nativeDeadline, localIsLow: low === 1n };
  for (const key of Object.keys(binding) as (keyof VoiceCryptoChainBinding)[]) {
    if (binding[key] !== expected.binding[key]) return fail();
  }
  if (localDevice !== expected.bootstrap.device || localJoin !== expected.bootstrap.join) return fail();
  const step = (row - 1) % 3, seal = low === 1n ? step !== 1 : step === 1;
  const ciphertextBytes = bodyLength + (seal ? 16 : 0);
  if (operation !== (seal ? 1 : 2) || ciphertextBytes < 16 || ciphertextBytes > 528 ||
      engineBytes !== 4 * (aadLength + ciphertextBytes) + 3 * 512 + 2048) return fail();
  const aad = take(aadLength), body = take(bodyLength);
  if (offset !== bytes.length) return fail();
  return Object.freeze({ owner: expected.bootstrap.owner, action, version, inputTag,
    binding: Object.freeze(binding), row, operation: seal ? "seal" : "open",
    kind: (["hello", "proof", "finish"] as const)[step], sequence, engineBytes,
    aad: new Uint8Array(aad), body: new Uint8Array(body) });
}
