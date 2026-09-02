/** The native result distinguishes a locked vault from a failed continuity save. */
export interface NativeVaultLockOutcome {
  /** Present only when native locking completed but the last UI snapshot could not be saved. */
  continuity_error: string | null;
}

/** Result returned only when native close deliberately leaves the window alive or cannot destroy it. */
export interface NativeVaultCloseOutcome extends NativeVaultLockOutcome {
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

  /** A successful unlock starts a new UI session and therefore permits one new lock request. */
  reset(): void {
    this.inFlight = null;
    this.capturedSnapshot = undefined;
  }
}
