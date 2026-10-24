// The joiner's view of one join attempt, as the start surface draws it while the attempt runs.
//
// `join_server` stays pending for as long as the dial, the reply window and the admission take,
// and the bridge now sends its step list as it grows (`join-progress`). This module folds that
// list into a handful of ROUTES (the invite, the direct dial, a relay circuit, a member
// switchboard, the two-way reply, the admission) with a state each, plus a phase and a headline
// for the verdict panel. Pure, so the copy a person reads under stress can be tested without a
// network or a webview.
//
// Two rules the copy keeps. Routes are named by KIND, never by address or by person: the
// addresses stay in the connection check for whoever needs them, and nothing here names the
// inviter or a helping member. And nothing here claims more than the backend recorded: a dial
// the transport issued is "tried" until a connect step says how it ended.

/// One step of an attempt, exactly as the bridge serialises it (`DiagStep`).
export type JoinStep = {
  at: number;
  kind: string;
  target: string;
  detail: string;
  status: string;
};

export type RouteKind = "invite" | "direct" | "relay" | "switchboard" | "reply" | "admission";

/// `idle` = not tried yet, `active` = being tried now, `ok` = answered, `failed` = did not,
/// `skipped` = deliberately not tried (the fallback the joiner did not allow).
export type RouteState = "idle" | "active" | "ok" | "failed" | "skipped";

export type RouteRow = {
  kind: RouteKind;
  /// What the row is called, in kind terms.
  label: string;
  state: RouteState;
  /// What happened, in one sentence; empty while idle.
  note: string;
};

export type JoinPhase = "idle" | "dialling" | "connected" | "failed";

export type JoinAttemptView = {
  phase: JoinPhase;
  rows: RouteRow[];
  /// The verdict's first line once the phase is `connected` or `failed`; empty otherwise.
  headline: string;
  /// Which route the failure landed on, for the verdict's advice; empty unless `failed`.
  failedOn: RouteKind | "";
};

export type JoinAttemptInput = {
  steps: JoinStep[];
  /// Whether the command is still pending (its promise has not settled).
  pending: boolean;
  /// The error the command settled with, or empty.
  error: string;
  /// Whether the invite offered a member fallback at all.
  fallbackOffered: boolean;
  /// Whether the joiner allowed that fallback.
  fallbackAllowed: boolean;
};

const LABELS: Record<RouteKind, string> = {
  invite: "the invite",
  direct: "direct to the inviter",
  relay: "relay circuit",
  switchboard: "member switchboard",
  reply: "two-way reply",
  admission: "admission",
};

function row(kind: RouteKind, state: RouteState, note = ""): RouteRow {
  return { kind, label: LABELS[kind], state, note };
}

function plural(n: number, one: string, many: string): string {
  return `${n} ${n === 1 ? one : many}`;
}

/// Fold an attempt's steps into the routes the surface draws.
export function joinAttemptView(input: JoinAttemptInput): JoinAttemptView {
  const { steps, pending, error } = input;
  const of = (kind: string) => steps.filter((s) => s.kind === kind);
  const inviteSteps = of("invite");
  const dialSteps = of("dial");
  const connectSteps = of("connect");
  const switchboardSteps = of("switchboard");
  const replySteps = of("reply");
  const joinSteps = of("join");
  const settledFailed = !pending && !!error;

  // Nothing has happened: the surface is waiting for Join.
  if (!steps.length && !settledFailed) {
    return { phase: "idle", rows: [], headline: "", failedOn: "" };
  }

  const rows: RouteRow[] = [];
  let failedOn: RouteKind | "" = "";

  // --- the invite itself ---
  const inviteFailed = inviteSteps.find((s) => s.status === "failed");
  if (inviteFailed) {
    rows.push(row("invite", "failed", "could not be read: " + inviteFailed.detail));
    failedOn = "invite";
  } else if (inviteSteps.some((s) => s.status === "ok")) {
    rows.push(row("invite", "ok", "signature checked"));
  } else if (settledFailed) {
    rows.push(row("invite", "failed", "could not be read: " + error));
    failedOn = "invite";
  } else {
    rows.push(row("invite", "active", "reading"));
  }

  // --- the direct dial, and a relay circuit if the invite carried one ---
  const dialled = dialSteps.filter((s) => s.status === "unknown" && s.target);
  const relayDialled = dialled.filter((s) => s.target.includes("p2p-circuit"));
  const directDialled = dialled.length - relayDialled.length;
  const unusable = dialSteps.filter((s) => s.status === "failed" && !s.target);
  const connectFailed = connectSteps.find((s) => s.status === "failed");
  const connectOk = connectSteps.find((s) => s.status === "ok");
  const viaInviter = !!connectOk && connectOk.detail.includes("named inviter");
  const unusableNote = unusable.length ? " " + unusable.map((s) => s.detail).join("; ") + "." : "";

  if (failedOn === "invite") {
    rows.push(row("direct", "idle"));
  } else if (connectFailed) {
    rows.push(row("direct", "failed", connectFailed.detail + "." + unusableNote));
    if (relayDialled.length) rows.push(row("relay", "failed", "the relay did not produce an answer either."));
    if (!failedOn && !switchboardSteps.length && !replySteps.length) failedOn = "direct";
  } else if (viaInviter && !replySteps.some((s) => s.status === "unknown")) {
    rows.push(row("direct", "ok", "answered"));
    if (relayDialled.length) rows.push(row("relay", "ok", "not needed"));
  } else if (connectOk) {
    // Connected some other way (a helper, or the reply below): the direct dial did not answer.
    rows.push(row("direct", "failed", "did not answer"));
    if (relayDialled.length) rows.push(row("relay", "failed", "did not answer"));
  } else if (dialled.length) {
    rows.push(
      row(
        "direct",
        "active",
        `dialling ${plural(directDialled, "address", "addresses")}${unusableNote}`,
      ),
    );
    if (relayDialled.length) rows.push(row("relay", "active", "dialling through the relay"));
  } else if (settledFailed) {
    rows.push(row("direct", "failed", error + unusableNote));
    if (!failedOn) failedOn = "direct";
  } else {
    rows.push(row("direct", "active", "preparing" + unusableNote));
  }

  // --- a member switchboard, when the invite offered one ---
  if (input.fallbackOffered || switchboardSteps.length) {
    const sbFailed = switchboardSteps.find((s) => s.status === "failed");
    const sbOk = switchboardSteps.find((s) => s.status === "ok" && s.detail.includes("connected"));
    const sbTrying = switchboardSteps.find((s) => s.status === "unknown" && s.detail.includes("trying"));
    const sbDeclined = switchboardSteps.find((s) => s.status === "unknown" && s.detail.includes("did not consent"));
    if (!input.fallbackAllowed || sbDeclined) {
      rows.push(row("switchboard", "skipped", "not allowed above, so not tried"));
    } else if (sbOk) {
      rows.push(row("switchboard", "ok", "a member forwarded the handshake"));
    } else if (sbFailed) {
      rows.push(row("switchboard", "failed", sbFailed.detail + "."));
      if (!replySteps.length) failedOn = "switchboard";
    } else if (sbTrying || connectFailed) {
      rows.push(row("switchboard", "active", "asking a member to forward the handshake"));
    } else {
      rows.push(row("switchboard", "idle", "tried only if the direct routes fail"));
    }
  }

  // --- the two-way reply ---
  if (replySteps.length) {
    const replyFailed = replySteps.find((s) => s.status === "failed");
    const replyWaiting = replySteps.find((s) => s.status === "unknown");
    if (replyFailed) {
      rows.push(row("reply", "failed", replyFailed.detail + "."));
      failedOn = "reply";
    } else if (replyWaiting && connectOk) {
      rows.push(row("reply", "ok", "the inviter dialled back"));
    } else if (replyWaiting) {
      rows.push(row("reply", "active", "waiting for the inviter to paste the reply; keep this window open"));
    }
  }

  // --- admission ---
  const joinFailed = joinSteps.find((s) => s.status === "failed");
  const joinOk = joinSteps.some((s) => s.status === "ok");
  if (joinOk) {
    rows.push(row("admission", "ok", "admitted to the group"));
  } else if (joinFailed) {
    rows.push(row("admission", "failed", joinFailed.detail + "."));
    failedOn = "admission";
  } else if (connectOk && pending) {
    rows.push(row("admission", "active", "connected; waiting for the group to admit you"));
  } else if (connectOk && settledFailed) {
    rows.push(row("admission", "failed", error));
    failedOn = "admission";
  }

  // --- the verdict ---
  let phase: JoinPhase;
  if (joinOk) phase = "connected";
  else if (pending) phase = "dialling";
  else if (settledFailed || rows.some((r) => r.state === "failed")) phase = "failed";
  else phase = "connected";

  if (phase === "failed" && !failedOn) {
    failedOn = rows.find((r) => r.state === "failed")?.kind ?? "direct";
  }
  if (phase !== "failed") failedOn = "";

  const headline =
    phase === "connected"
      ? "You are in."
      : phase === "failed"
        ? HEADLINES[failedOn || "direct"]
        : "";

  return { phase, rows, headline, failedOn };
}

const HEADLINES: Record<RouteKind, string> = {
  invite: "This invite could not be read. Ask for a fresh one.",
  direct: "Nobody answered. Your invite is fine; the routes it carried did not reach the inviter.",
  relay: "Nobody answered. Your invite is fine; the routes it carried did not reach the inviter.",
  switchboard: "Nobody answered, and the member fallback did not reach the inviter either.",
  reply: "The reply window closed before the inviter dialled back.",
  admission: "Connected, but the group did not admit you.",
};
