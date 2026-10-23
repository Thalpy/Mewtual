/**
 * The host side of the pre-upload video editor: which video is being reviewed, which are waiting,
 * and how each review settles.
 *
 * App.svelte can run more than one composer batch at a time (a paste into the status composer
 * while a chat drop is still uploading, for example). Each batch awaits `request` for every video
 * it carries. The rules that keep those batches from trampling each other live here, apart from
 * App, so they can be tested directly:
 *
 * - One editor is open at a time. A later request waits its turn; it never closes the open one.
 *   (Closing it would discard edits the person is in the middle of making.)
 * - An answer only counts from the editor it was asked of. Every opening gets a fresh id, and
 *   `finish` with any other id is ignored, so a closed editor's late answer (an export resolving
 *   as it was torn down) can never be taken as the next file's answer.
 * - Every request settles. A review bound to a place (lock state, server, channel) that is no
 *   longer current settles as `skip`, whether it is open or still waiting; nothing is left pending
 *   for a batch to hang on.
 */
import type { VideoEditorContext, VideoEditorResult } from "./video-export.ts";

export type ReviewPosition = { readonly index: number; readonly count: number };

/** What the host mounts the editor with. */
export type OpenReview = {
  readonly id: number;
  readonly file: File;
  readonly position: ReviewPosition;
  /** The host's scope key when the review was requested. */
  readonly scope: string;
  /** Where the upload is going (wording, and whether "post now" is offered). */
  readonly context: VideoEditorContext;
};

type Pending = OpenReview & { resolve: (result: VideoEditorResult) => void };

const SKIP: VideoEditorResult = { kind: "skip" };

export class VideoReviewQueue {
  #open: Pending | null = null;
  #waiting: Pending[] = [];
  #seq = 0;
  readonly #show: (open: OpenReview | null) => void;

  /** `show` is told what to mount (or that nothing is open) every time that changes. */
  constructor(show: (open: OpenReview | null) => void) {
    this.#show = show;
  }

  /** The review whose editor is showing, if any. */
  get current(): OpenReview | null {
    return this.#open ? publicView(this.#open) : null;
  }

  /** How many requests are waiting behind the open one. */
  get waiting(): number {
    return this.#waiting.length;
  }

  /** Ask for a review of `file`, bound to `scope`. Resolves once, with the person's answer or `skip`. */
  request(file: File, position: ReviewPosition, scope: string, context: VideoEditorContext): Promise<VideoEditorResult> {
    return new Promise((resolve) => {
      this.#waiting.push({ id: ++this.#seq, file, position, scope, context, resolve });
      if (!this.#open) this.#advance();
    });
  }

  /**
   * The editor opened as `id` answered. Returns false, and changes nothing, when `id` is not the
   * open review: that editor was already closed or replaced and its answer belongs to nobody.
   */
  finish(id: number, result: VideoEditorResult): boolean {
    const open = this.#open;
    if (!open || open.id !== id) return false;
    this.#open = null;
    open.resolve(result);
    this.#advance();
    return true;
  }

  /**
   * The host's scope is now `scope`. Every review bound to another scope settles as `skip`.
   * Reports whether an OPEN editor was closed (the person may have been mid-edit, which is worth
   * telling them) and how many waiting requests were dropped.
   */
  rescope(scope: string): { closedOpen: boolean; closedContext: VideoEditorContext | null; dropped: number } {
    let closedOpen = false;
    let closedContext: VideoEditorContext | null = null;
    const open = this.#open;
    if (open && open.scope !== scope) {
      this.#open = null;
      open.resolve(SKIP);
      closedOpen = true;
      closedContext = open.context;
    }
    let dropped = 0;
    this.#waiting = this.#waiting.filter((pending) => {
      if (pending.scope === scope) return true;
      pending.resolve(SKIP);
      dropped++;
      return false;
    });
    if (closedOpen) this.#advance();
    return { closedOpen, closedContext, dropped };
  }

  #advance() {
    this.#open = this.#waiting.shift() ?? null;
    this.#show(this.#open ? publicView(this.#open) : null);
  }
}

/** The mountable fields only: the resolver never leaves this module. */
function publicView(pending: Pending): OpenReview {
  return { id: pending.id, file: pending.file, position: pending.position, scope: pending.scope, context: pending.context };
}
