import { base64ToBytes, bytesToBase64 } from "./studio-native.ts";

/** Stateful, development-only file bridge for the real Studio → attachment → chat flow.
 * This mocks file custody, never Studio authoring or an owner receipt. Keep unknown commands
 * unhandled so the visual fixture still fails loudly when a UI acquires a new dependency. */
export function createVisualFileUploads(author: string) {
  const sliceBytes = 64 * 1024, limit = 9 * 1024 * 1024;
  type Upload = { server: number; token: string; mime: string; bytes: Uint8Array; received: number };
  let pending: Upload | null = null, serial = 0;
  const saved = new Map<string, { file: { cid: string; name: string; mime: string; size: number; path: string; author: string; author_identity: string; author_verified: boolean; held: number; total: number; expires: null; expires_known: boolean }; bytes: Uint8Array; server: number }>();
  return {
    files(server: number) { return [...saved.values()].filter(v => v.server === server).map(v => ({ ...v.file })); },
    response(command: string, args: Record<string, unknown>): { handled: true; value: unknown } | { handled: false } {
      const handled = (value?: unknown) => ({ handled: true as const, value });
      if (command === "begin_file_upload") {
        if (pending || saved.size >= 4 || args.server !== 1 || typeof args.uploadId !== "string" || !args.uploadId || typeof args.mime !== "string" || !Number.isSafeInteger(args.size) || Number(args.size) < 1 || Number(args.size) > limit) throw new Error("Invalid visual upload");
        const token = `visual-upload-${++serial}`, size = Number(args.size);
        pending = { server: 1, token, mime: args.mime, bytes: new Uint8Array(size), received: 0 };
        return handled({ token, chunkTotal: Math.ceil(size / sliceBytes), sliceBytes });
      }
      if (command === "push_file_chunk" || command === "finish_file_upload" || command === "cancel_file_upload") {
        const upload = pending;
        if (!upload || args.server !== upload.server || args.token !== upload.token) throw new Error("Unknown visual upload");
        if (command === "cancel_file_upload") { pending = null; return handled(); }
        if (command === "push_file_chunk") {
          if (typeof args.data !== "string" || args.data.length > Math.ceil(sliceBytes / 3) * 4 || args.offset !== upload.received) throw new Error("Invalid visual upload slice");
          const bytes = base64ToBytes(args.data), expected = Math.min(sliceBytes, upload.bytes.length - upload.received);
          if (!expected || bytes.length !== expected) throw new Error("Invalid visual upload slice length");
          upload.bytes.set(bytes, upload.received); upload.received += bytes.length;
          return handled();
        }
        if (upload.received !== upload.bytes.length || typeof args.name !== "string" || typeof args.path !== "string") throw new Error("Incomplete visual upload");
        // Stable fixture identifiers deliberately differ from both source Studio blob CIDs and
        // the preloaded sample file. No test may infer real content authentication from them.
        const cid = "d".repeat(56) + serial.toString(16).padStart(8, "0");
        saved.set(cid, { server: upload.server, bytes: upload.bytes, file: { cid, name: args.name, mime: upload.mime, size: upload.bytes.length, path: args.path, author: author.slice(0, 32), author_identity: author, author_verified: true, held: 1, total: 1, expires: null, expires_known: true } });
        pending = null;
        return handled(cid);
      }
      if (command === "download_file" && typeof args.cid === "string" && saved.has(args.cid)) {
        const value = saved.get(args.cid)!;
        if (args.server !== value.server || typeof args.maxBytes !== "number" || value.bytes.length > args.maxBytes) throw new Error("Visual download scope or limit mismatch");
        return handled(bytesToBase64(value.bytes));
      }
      return { handled: false };
    },
  };
}
