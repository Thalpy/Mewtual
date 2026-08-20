export type FeedbackKind = "bug" | "feature";

const ISSUE_ORIGIN = "https://github.com";
const ISSUE_PATH = "/Thalpy/Mewtual/issues/new";
export const FEEDBACK_TEXT_MAX_CHARS = 50_000;
export const ISSUE_URL_MAX_CHARS = 6_000;
const TRUNCATION_NOTE = "\n\n_(Report truncated: the full text is on your clipboard.)_";

/** Return a prefix without bisecting a UTF-16 surrogate pair at the requested boundary. */
function safePrefix(text: string, length: number): string {
  let end = Math.max(0, Math.min(text.length, Math.trunc(length)));
  const before = text.charCodeAt(end - 1);
  const after = text.charCodeAt(end);
  if (before >= 0xd800 && before <= 0xdbff && after >= 0xdc00 && after <= 0xdfff) end -= 1;
  return text.slice(0, end);
}

export function feedbackSubject(kind: FeedbackKind, title: string, text: string): string {
  const typed = title.trim();
  const first = text.trim().split("\n")[0].trim();
  return (typed || first || (kind === "bug" ? "Bug report" : "Feature request")).slice(0, 120);
}

export function feedbackReport(kind: FeedbackKind, text: string, version: string, userAgent: string): string {
  return [
    `**Type:** ${kind === "bug" ? "Bug report" : "Feature request"}`,
    `**App:** Mewtual desktop ${version}`,
    `**Environment:** ${userAgent}`,
    "",
    safePrefix(text.trim(), FEEDBACK_TEXT_MAX_CHARS),
  ].join("\n");
}

function issueUrl(kind: FeedbackKind, subject: string, body: string): string {
  const params = new URLSearchParams({
    labels: kind === "bug" ? "bug" : "enhancement",
    title: subject,
    body,
  });
  return `${ISSUE_ORIGIN}${ISSUE_PATH}?${params}`;
}

/**
 * Build a bounded prefilled issue URL without cutting a percent-encoded sequence. A binary search
 * avoids repeatedly shaving a potentially large report in tiny increments on the UI thread.
 */
export function buildFeedbackIssue(
  kind: FeedbackKind,
  title: string,
  text: string,
  version: string,
  userAgent: string,
): { url: string; report: string; truncated: boolean } {
  const report = feedbackReport(kind, text, version, userAgent);
  const subject = feedbackSubject(kind, title, text);
  const complete = issueUrl(kind, subject, report);
  if (complete.length <= ISSUE_URL_MAX_CHARS) return { url: complete, report, truncated: false };

  let low = 0;
  let high = report.length;
  while (low < high) {
    const middle = Math.ceil((low + high) / 2);
    if (issueUrl(kind, subject, safePrefix(report, middle) + TRUNCATION_NOTE).length <= ISSUE_URL_MAX_CHARS) low = middle;
    else high = middle - 1;
  }
  return {
    url: issueUrl(kind, subject, safePrefix(report, low) + TRUNCATION_NOTE),
    report,
    truncated: true,
  };
}

/** Frontend defence in depth; the native command independently enforces the same destination. */
export function isAllowedIssueUrl(candidate: string): boolean {
  if (!candidate || candidate.length > ISSUE_URL_MAX_CHARS || /[\u0000-\u001f\u007f]/.test(candidate)) return false;
  try {
    const parsed = new URL(candidate);
    return (
      parsed.origin === ISSUE_ORIGIN &&
      parsed.pathname === ISSUE_PATH &&
      parsed.username === "" &&
      parsed.password === "" &&
      parsed.hash === "" &&
      parsed.search.length > 1
    );
  } catch {
    return false;
  }
}
