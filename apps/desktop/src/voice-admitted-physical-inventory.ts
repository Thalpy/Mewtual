/** Logical application-owned graph, not provider/GC/WebView RSS. This full
 * payment includes bounded parser/copy frames, four retained key references,
 * the original SDK job overlap and an independently retained final-ACK retry.
 * No native allocation is paid by this browser ledger. */
export const VOICE_ADMITTED_PHYSICAL_BYTES = 32 * 1024;
export const VOICE_ADMITTED_PHYSICAL_RECEIPT_BYTES = 4 * 1024;
const receiptParts = Object.freeze({
  entry: 10 * 16,
  // Actual fixed adapter object/methods plus both original identity closures;
  // it can outlive the refunded selected-key record and must pay for itself.
  adapter: 11 * (16 + 64) + 2 * 64,
  // Conservatively retain the reused key-adapter construction environment's
  // old object and unused admission closure; never assume optimizer elision.
  adapterConstructionEnvironment: 64 + 64,
  strings: 2 * (64 + 64 + 64 + 64),
  nativeFlight: 128 + 256,
  timer: 26 * 16 + 64 + 2 * 128 + 3 * 64 + 64,
  // Full bounded abandon snapshot and Promise input arrays, charged even if
  // this is the last descriptor; no optimistic one-entry table fraction.
  indexesAndShutdown: 64 + 2 * 32 * 8 + 128,
});
const receiptInventory = Object.freeze({ ...receiptParts, total: Object.values(receiptParts).reduce((sum, value) => sum + value, 0) });
export function voiceAdmittedPhysicalReceiptInventory() { return receiptInventory; }
const parts = Object.freeze({
  driverCellsHandles: 40 * 16 + 3 * 128 + 6 * 128,
  signalsQueueCallbacks: 6 * 128 + 8 * 64 + 1024,
  originalAndParsedTranscript: 2 * 779 + 2 * 1558,
  // Current pure parser's largest boxed hex workspace is the raw group256.
  // Include number slots AND short hex-string shells; do not invoice only text.
  parserScratch: 256 * (8 + 32) + 2 * 512,
  sdkFramesAndCopies: 2 * 1024 + 4 * 128 + 4 * 65 + 4 * 32,
  identitiesMetadata: 2 * (64 + 64 + 64 + 64) + 16 * 16,
  receiptDescriptor: VOICE_ADMITTED_PHYSICAL_RECEIPT_BYTES,
});
const inventory = Object.freeze({ ...parts, total: Object.values(parts).reduce((sum, value) => sum + value, 0) });
export function voiceAdmittedPhysicalInventory() { return inventory; }
