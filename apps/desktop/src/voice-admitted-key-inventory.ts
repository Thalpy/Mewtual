/** Logical application-owned graph, not WebView RSS or opaque provider/GC memory.
 * Each category conservatively prepays the full overlapping construction/return
 * graph. Caller-owned inert shells may survive after their targets are detached;
 * they retain no broker, SDK key, native adapter or original cleanup obligation. */
export const VOICE_ADMITTED_SELECTED_KEY_BYTES = 8 * 1024;
const parts = Object.freeze({
    // Selected record (40 slots), key record, private callback targets.
    records: 40 * 16 + 6 * 16 + 2 * 64,
    cells: 2 * 128,
    handlesAndClosures: 2 * 128 + 4 * 64 + 2 * 128,
    signalsQueueAndFrames: 3 * 128 + 128 + 512 + 256 + 128,
    // UTF-16 worst case for retained executor/permit/join and copied terminal
    // owner/executor/permit/join/keyRef, including the original shared executor.
    immutableStrings: 2 * (128 + 2 * 64 + 2 * 128 + 2 * 64 + 64),
    // Original exported point and one caller snapshot. The encoder uses a fixed
    //130-byte ASCII buffer, NOT65 boxed numbers plus65 temporary hex strings.
    pointAndEncoding: 2 * 65 + 130 + 260 + 64,
    terminalAndOutcome: 2 * 6 * 16 + 128,
    adapterAndCodec: 4 * 64 + 96 + 64,
    indexes: 3 * 64,
});
const inventory = Object.freeze({ ...parts, total: Object.values(parts).reduce((sum, value) => sum + value, 0) });
/** The preallocation fence itself creates no per-owner object or array. */
export function voiceAdmittedKeyInventory() { return inventory; }
