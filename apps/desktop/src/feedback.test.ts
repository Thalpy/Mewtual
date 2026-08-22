import assert from "node:assert/strict";
import test from "node:test";
import {
  FEEDBACK_TEXT_MAX_CHARS,
  ISSUE_URL_MAX_CHARS,
  buildFeedbackIssue,
  feedbackSubject,
  isAllowedIssueUrl,
} from "./feedback.ts";

test("feedback subjects prefer the title and remain bounded", () => {
  assert.equal(feedbackSubject("bug", "  A title  ", "first line"), "A title");
  assert.equal(feedbackSubject("feature", "", "  first line\nsecond"), "first line");
  assert.equal(feedbackSubject("bug", "", ""), "Bug report");
  assert.equal(feedbackSubject("bug", "x".repeat(200), ""), "x".repeat(120));
});

test("ordinary reports produce an allowlisted tracker URL", () => {
  const issue = buildFeedbackIssue("bug", "Crash", "Steps here", "1.2.3", "test-agent");
  assert.equal(issue.truncated, false);
  assert.equal(isAllowedIssueUrl(issue.url), true);
  const parsed = new URL(issue.url);
  assert.equal(parsed.searchParams.get("labels"), "bug");
  assert.equal(parsed.searchParams.get("title"), "Crash");
  assert.match(parsed.searchParams.get("body") ?? "", /Steps here/);
});

test("large reports are bounded without losing the clipboard copy", () => {
  const issue = buildFeedbackIssue("feature", "Large", "🐈".repeat(FEEDBACK_TEXT_MAX_CHARS), "1", "agent");
  assert.equal(issue.truncated, true);
  assert.ok(issue.url.length <= ISSUE_URL_MAX_CHARS);
  assert.ok(issue.report.length > issue.url.length);
  assert.match(new URL(issue.url).searchParams.get("body") ?? "", /full text is on your clipboard/);
});

test("report and URL truncation never split a Unicode surrogate pair", () => {
  const issue = buildFeedbackIssue(
    "bug",
    "Unicode",
    `a${"\u{1f408}".repeat(FEEDBACK_TEXT_MAX_CHARS)}`,
    "1",
    "agent",
  );
  const body = new URL(issue.url).searchParams.get("body") ?? "";
  assert.equal(issue.report.includes("\ufffd"), false);
  assert.equal(body.includes("\ufffd"), false);
});

test("lookalike, credentialed, fragmented and non-https destinations are refused", () => {
  assert.equal(isAllowedIssueUrl("https://github.com/Thalpy/Mewtual/issues/new?title=x"), true);
  assert.equal(isAllowedIssueUrl("https://github.com.evil.test/Thalpy/Mewtual/issues/new?title=x"), false);
  assert.equal(isAllowedIssueUrl("https://github.com/Thalpy/Other/issues/new?title=x"), false);
  assert.equal(isAllowedIssueUrl("https://user@github.com/Thalpy/Mewtual/issues/new?title=x"), false);
  assert.equal(isAllowedIssueUrl("https://github.com/Thalpy/Mewtual/issues/new?title=x#fragment"), false);
  assert.equal(isAllowedIssueUrl("http://github.com/Thalpy/Mewtual/issues/new?title=x"), false);
});
