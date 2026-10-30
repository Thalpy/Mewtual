/** The native result distinguishes a locked vault from a failed continuity save. */
export interface NativeVaultLockOutcome {
  /** Present only when native locking completed but the last UI snapshot could not be saved. */
  continuity_error: string | null;
}

/** Result returned only when native close deliberately leaves the window alive or cannot destroy it. */
export interface NativeVaultCloseOutcome extends NativeVaultLockOutcome {
  /** A server snapshot failed. A second close retries; it never discards message history. */
  history_error?: string | null;
  /** True when the first close paused so the user can acknowledge lost latest screen state. */
  deferred: boolean;
  /** Present when native locking completed but native window destruction failed. */
  destroy_error: string | null;
}

/**
 * Owns the one native lock request for an unlocked UI session.
 *
 * Ctrl+L clears the webview immediately, while native persistence finishes asynchronously. A
 * subsequent OS close must await that exact request: issuing a second request with an already
 * cleared snapshot could overwrite or race the last useful continuity state.
 */
export class NativeVaultLockCoordinator {
  private inFlight: Promise<NativeVaultLockOutcome> | null = null;
  private capturedSnapshot: string | null | undefined;
  private readonly invokeLock: (uiStateJson: string | null) => Promise<NativeVaultLockOutcome>;

  constructor(
    invokeLock: (uiStateJson: string | null) => Promise<NativeVaultLockOutcome>,
  ) {
    this.invokeLock = invokeLock;
  }

  begin(uiStateJson: string | null): Promise<NativeVaultLockOutcome> {
    if (!this.inFlight) {
      this.capturedSnapshot = uiStateJson;
      this.inFlight = this.invokeLock(uiStateJson);
    }
    return this.inFlight;
  }

  /**
   * The immutable snapshot Ctrl+L submitted, for a native-owned close racing that request.
   * `null` also covers a cold/remounted lock gate; native close still crosses the commit mutex and
   * establishes the lock boundary before it destroys the webview.
   */
  snapshot(): string | null {
    return this.capturedSnapshot ?? null;
  }

  /**
   * Join the current Ctrl+L transaction before attempting to unlock a new UI generation.
   * A rejected bridge attempt keeps its immutable snapshot and may be retried on the next call.
   */
  async settle(): Promise<NativeVaultLockOutcome | null> {
    if (!this.inFlight && this.capturedSnapshot !== undefined) {
      this.inFlight = this.invokeLock(this.capturedSnapshot);
    }
    const attempt = this.inFlight;
    if (!attempt) return null;
    try {
      return await attempt;
    } catch (error) {
      if (this.inFlight === attempt) this.inFlight = null;
      throw error;
    }
  }

  /**
   * Retire a fulfilled lock under the current UI unlock attempt, immediately before
   * authentication is dispatched (no intervening await). This permits one fresh negative
   * lock while authentication is pending. Never call after a rejected settle or stale reply.
   * Native retains valid failed-continuity bytes across a fulfilled warning and retries
   * them before opening IPC; this does not promise retention of malformed JSON.
   */
  reset(): void {
    this.inFlight = null;
    this.capturedSnapshot = undefined;
  }
}

/** Snapshot failure must never stop Ctrl+L before native authority and private UI are revoked. */
export function captureLockContinuity(capture: () => string | null): { json: string | null; error: string } {
  try { return { json: capture(), error: "" }; }
  catch (e) { return { json: null, error: `Locked, but the latest Studio edits could not be saved: ${String(e)}` }; }
}
