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
): Promise<SavedFileResult> {
  return checkedResult(await invoke<unknown>("save_group_file", { server, cid, name }));
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

export function downloadSavedNotice(name: string, saved: SavedFileResult) {
  if (saved.contentValidation === "mismatch") {
    return {
      text: `Saved ${name}, but its bytes do not match the declared media type. Treat the unlocked copy as untrusted.`,
      kind: "warn" as const,
      ms: 10_000,
    };
  }
  if (saved.contentValidation === "unrecognized") {
    return {
      text: `Saved ${name}, but Mewtual could not recognize its declared media format. Treat the unlocked copy as untrusted.`,
      kind: "warn" as const,
      ms: 10_000,
    };
  }
  return saved.displayed
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
