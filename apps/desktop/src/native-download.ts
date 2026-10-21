export type NativeInvoker = <T>(
  command: string,
  args?: Record<string, unknown>,
) => Promise<T>;

export type SavedFileResult = {
  path: string;
  displayed: boolean;
  warning?: string;
  contentValidation?: "matched" | "mismatch" | "unrecognized";
};

function checkedResult(value: unknown): SavedFileResult {
  if (!value || typeof value !== "object") {
    throw new Error("the desktop returned an invalid save result");
  }
  const result = value as Record<string, unknown>;
  if (typeof result.path !== "string" || !result.path.trim() || typeof result.displayed !== "boolean") {
    throw new Error("the desktop returned an invalid save result");
  }
  if (result.warning !== undefined && result.warning !== null && typeof result.warning !== "string") {
    throw new Error("the desktop returned an invalid save result");
  }
  if (result.contentValidation !== undefined && result.contentValidation !== null &&
      result.contentValidation !== "matched" && result.contentValidation !== "mismatch" &&
      result.contentValidation !== "unrecognized") {
    throw new Error("the desktop returned an invalid save result");
  }
  return {
    path: result.path,
    displayed: result.displayed,
    ...(typeof result.warning === "string" ? { warning: result.warning } : {}),
    ...(typeof result.contentValidation === "string"
      ? { contentValidation: result.contentValidation as SavedFileResult["contentValidation"] }
      : {}),
  };
}

/**
 * Save a shared file to Downloads.
 *
 * The bytes are never handed to the webview: the native side fetches the file chunk by chunk and
 * writes it straight to disk, reporting the same download-progress events the Transfers panel
 * already listens for. Pulling a whole file into JS just to hand it back for saving moved it
 * across the IPC bridge twice, which is what made large transfers freeze the app.
 */
export async function saveGroupFile(
  invoke: NativeInvoker,
  server: number,
  cid: string,
  name: string,
  options?: {
    resume?: string; expectedIntent?: string; operation?: string; freshCopy?: boolean;
    /** Native opens the OS "Save as" dialog itself; no path or folder is ever sent from here. */
    saveAs?: boolean;
  },
): Promise<SavedFileResult> {
  return checkedResult(await invoke<unknown>("save_group_file", {
    server, cid, name,
    ...(options?.resume ? { resume: options.resume, expectedIntent: options.expectedIntent } : {}),
    // Only meaningful with a resume ID: native removes the partly saved file it can prove it
    // created, forgets that journal under the same intent CAS, then saves a separate file.
    ...(options?.resume && options.freshCopy ? { freshCopy: true } : {}),
    ...(options?.saveAs ? { saveAs: true } : {}),
    ...(options?.operation ? { operation: options.operation } : {}),
  }));
}

/** Explicit resume selects saved encrypted work by ID/revision. Native revalidates the saved
 * destination (or captures a fresh one for a new copy). A browser path, plaintext offset or new
 * allowance is never sent. */
export async function resumeGroupFile(
  invoke: NativeInvoker, server: number, cid: string, name: string,
  resume: string, expectedIntent: string, operation?: string, freshCopy = false, saveAs = false,
): Promise<SavedFileResult> {
  return saveGroupFile(invoke, server, cid, name, { resume, expectedIntent, operation, freshCopy, saveAs });
}

/** Native's exact reply when the user closes the "Save as" dialog without choosing. Nothing was
 * started or recorded, so the caller drops its pending row quietly instead of showing an error. */
export const SAVE_AS_CANCELLED = "Save as was cancelled.";
export function isSaveAsCancelled(error: unknown): boolean {
  return (error instanceof Error ? error.message : String(error)) === SAVE_AS_CANCELLED;
}

export async function saveSpaceGuide(
  invoke: NativeInvoker,
  pngBase64: string,
): Promise<SavedFileResult> {
  return checkedResult(await invoke<unknown>("save_and_open_space_guide", { pngBase64 }));
}

export function completedDownload(
  transfer: { total: number; bytesTotal: number },
  saved: SavedFileResult,
  updatedAt: number,
) {
  return {
    status: "done" as const,
    progress: 1,
    done: transfer.total,
    bytesDone: transfer.bytesTotal,
    savedPath: saved.path,
    error: saved.warning,
    updatedAt,
  };
}

/** `toDownloads` is true only when the save provably went to the Downloads folder (a first
 * one-click save). A "Save as" choice, a remembered choice or a resumed partial may be anywhere,
 * so those notices name no folder. */
export function downloadSavedNotice(name: string, saved: SavedFileResult, toDownloads = true) {
  const warning = saved.warning ? ` ${saved.warning}` : "";
  if (!toDownloads && !saved.contentValidation) {
    return saved.displayed
      ? { text: `Saved ${name}.${warning}`, kind: saved.warning ? "info" as const : "ok" as const, ms: saved.warning ? 8_000 : 4_000 }
      : { text: `Saved to ${saved.path}.${warning}`, kind: "info" as const, ms: 8_000 };
  }
  if (saved.contentValidation === "mismatch") {
    return {
      text: `Saved ${name}, but its bytes do not match the declared media type. Treat the unlocked copy as untrusted.${warning}`,
      kind: "warn" as const,
      ms: 10_000,
    };
  }
  if (saved.contentValidation === "unrecognized") {
    return {
      text: `Saved ${name}, but Mewtual could not recognize its declared media format. Treat the unlocked copy as untrusted.${warning}`,
      kind: "warn" as const,
      ms: 10_000,
    };
  }
  return saved.warning && saved.displayed ? { text: `Saved ${name} to Downloads. ${saved.warning}`, kind: "info" as const, ms: 8_000 }
    : saved.displayed
    ? { text: `Saved ${name} to Downloads`, kind: "ok" as const, ms: 4_000 }
    : {
        text: `Saved to ${saved.path}. ${saved.warning ?? "Open Downloads to view it."}`,
        kind: "info" as const,
        ms: 8_000,
      };
}

export function guideSavedNotice(saved: SavedFileResult) {
  return saved.displayed
    ? {
        note: `Saved to ${saved.path} and opened in your image viewer.`,
        text: "Guide saved and opened",
        kind: "ok" as const,
      }
    : {
        note: `Saved to ${saved.path}. ${saved.warning ?? "Open it from Downloads."}`,
        text: "Guide saved to Downloads",
        kind: "info" as const,
      };
}
