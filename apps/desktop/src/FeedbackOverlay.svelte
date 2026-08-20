<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import {
    FEEDBACK_TEXT_MAX_CHARS,
    buildFeedbackIssue,
    feedbackReport,
    isAllowedIssueUrl,
    type FeedbackKind,
  } from "./feedback";

  let { version, onclose, onerror } = $props<{
    version: string;
    onclose: () => void;
    onerror: (message: string) => void;
  }>();
  let kind = $state<FeedbackKind>("bug");
  let title = $state("");
  let description = $state("");
  let copied = $state(false);
  let opened = $state(false);

  async function copyReport(report = feedbackReport(kind, description, version, navigator.userAgent)) {
    try {
      await navigator.clipboard.writeText(report);
      copied = true;
      setTimeout(() => (copied = false), 2_000);
    } catch (cause) {
      onerror(String(cause));
    }
  }

  async function openIssue() {
    const issue = buildFeedbackIssue(kind, title, description, version, navigator.userAgent);
    if (!isAllowedIssueUrl(issue.url)) {
      onerror("The feedback destination failed its local security check.");
      return;
    }
    if (issue.truncated) await copyReport(issue.report);
    try {
      // The Rust command repeats the exact-origin/path allowlist before launching the OS browser.
      await invoke("open_issue_url", { url: issue.url });
      opened = true;
      setTimeout(() => (opened = false), 4_000);
    } catch (cause) {
      onerror(String(cause));
    }
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
<div class="overlay" role="presentation" onclick={(event) => { if (event.target === event.currentTarget) onclose(); }}>
  <div class="overlay-card">
    <header class="overlay-head">
      <h2>💬 Send feedback</h2>
      <button class="ghost" onclick={onclose}>✕</button>
    </header>
    <div class="overlay-body feedback">
      <div class="seg fb-seg">
        <button class:active={kind === "bug"} onclick={() => (kind = "bug")}>🐞 Bug report</button>
        <button class:active={kind === "feature"} onclick={() => (kind = "feature")}>✨ Feature request</button>
      </div>
      <label class="fb-label" for="fb-title">Title</label>
      <input
        id="fb-title"
        class="fb-text"
        bind:value={title}
        maxlength="120"
        placeholder={kind === "bug" ? "Short summary of the problem" : "Short summary of the idea"}
      />
      <label class="fb-label" for="fb-text">
        {kind === "bug"
          ? "What went wrong? Steps to reproduce, and what you expected to happen."
          : "What would you like Mewtual to do?"}
      </label>
      <textarea
        id="fb-text"
        class="fb-text"
        bind:value={description}
        maxlength={FEEDBACK_TEXT_MAX_CHARS}
        rows="7"
        placeholder="Describe it here…"
      ></textarea>
      <p class="muted small">
        Filing opens a prefilled issue on the
        <strong>{kind === "bug" ? "bug tracker" : "feature request tracker"}</strong>
        in your browser: review it and press Submit there. Mewtual sends nothing on its own and holds no GitHub
        account of yours, so nothing is posted until you submit it. Your app version and environment are included
        to help debugging. No GitHub account? Copy the report and send it to the maintainer instead.
      </p>
      <div class="file-info-actions">
        <button class="primary" disabled={!description.trim()} onclick={openIssue}>
          {opened ? "✓ Opened in your browser" : kind === "bug" ? "🐞 File on GitHub" : "✨ File on GitHub"}
        </button>
        <button class="ghost" disabled={!description.trim()} onclick={() => copyReport()}>
          {copied ? "✓ Copied to clipboard" : "Copy report"}
        </button>
      </div>
    </div>
  </div>
</div>

<style>
  .feedback .fb-seg {
    display: flex;
    gap: 8px;
    margin-bottom: 14px;
  }
  .feedback .fb-seg button {
    flex: 1;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: var(--r);
    background: var(--bg-elev);
    cursor: pointer;
  }
  .feedback .fb-seg button.active {
    border-color: var(--accent);
    background: var(--accent-dim);
  }
  .fb-label {
    display: block;
    margin-bottom: 6px;
    color: var(--muted);
    font-size: 0.9em;
  }
  .fb-text {
    width: 100%;
    resize: vertical;
    font: inherit;
    padding: 8px;
    border-radius: var(--r);
    border: 1px solid var(--border);
    background: var(--bg-elev);
    color: inherit;
    box-sizing: border-box;
    margin-bottom: 10px;
  }
</style>
