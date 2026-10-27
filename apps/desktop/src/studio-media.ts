// Native immutable-media custody. These helpers never save a Studio reference on behalf of a
// publication: the session queues that separately after the validated blob's durable result.
import { decodePixa, encodePixa } from "./pixa.ts";
import { bytesToBase64, isHex64, StudioNativeError, type PixPublication, type StudioIpc } from "./studio-native.ts";

export type StudioGifSaved = { path: string; displayed: boolean; warning: string | null; contentValidation: "matched" };

function ownedPackage(bytes: Uint8Array): Uint8Array {
  // Reuse the byte-capped package's intrinsic-length/Buffer/shared-memory snapshot and canonical
  // validators. This owns every byte before invoking an asynchronous native worker.
  return encodePixa(decodePixa(bytes));
}
function record(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new StudioNativeError("media result is not an object");
  return value as Record<string, unknown>;
}

/** Return only an exact native durable blob acknowledgement; no index/export record is implied. */
export async function publishStudioPixa(ipc: StudioIpc, server: number, bytes: Uint8Array): Promise<PixPublication> {
  const owned = ownedPackage(bytes);
  const result = record(await ipc.invoke<unknown>("publish_pixa", { server, bytesB64: bytesToBase64(owned) }));
  if (!isHex64(result.cid) || result.bytes !== owned.length) throw new StudioNativeError("pixa publication does not match the submitted bytes");
  return { cid: result.cid, bytes: owned.length };
}

/** Save a visual-only infinite GIF through the native fixed-name, collision-safe Downloads path. */
export async function exportStudioGif(ipc: StudioIpc, server: number, bytes: Uint8Array): Promise<StudioGifSaved> {
  const owned = ownedPackage(bytes);
  const result = record(await ipc.invoke<unknown>("export_studio_gif", { server, bytesB64: bytesToBase64(owned) }));
  if (result.contentValidation !== "matched" || typeof result.path !== "string" || !result.path.length || result.path.length > 32_768 ||
    typeof result.displayed !== "boolean" || (result.warning !== null && (typeof result.warning !== "string" || result.warning.length > 4096))) {
    throw new StudioNativeError("GIF save result is not a validated native acknowledgement");
  }
  // A reveal failure is still a successful local save. The UI must show the real path and the
  // warning, not invite a second write by claiming that no file was saved.
  return { path: result.path, displayed: result.displayed, warning: result.warning, contentValidation: "matched" };
}
