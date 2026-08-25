<script lang="ts">
  /**
   * The in-app debug console (`docs/design-debug-console.md`).
   *
   * The app used to fail silently: a call died while the roster still showed the peer online, and a
   * node sat isolated for an hour submitting dial batches with no connection and none of that
   * activity surfaced anywhere. This is the live view of what the diagnostics know, segmented so the failing
   * layer is findable in seconds: is it the network, the voice stack, the backend, or the frontend?
   *
   * Capture is native and always on (`catcoms-log`'s ring). This polls it while open, so a closed
   * console costs nothing and one that has just opened still shows the run-up to whatever the user
   * came here about.
   *
   * A separate component rather than more of App.svelte, which is already 20,000 lines. The
   * adversarial review's DBG-014 is the reason: a diagnostic suite embedded in the largest file in
   * the codebase cannot be tested on its own, and every change to it is a change to the file
   * everything else also lives in. Nothing application-owned crosses the boundary either. The props
   * are plain snapshots, so the console cannot reach into state it has no business touching.
   */
  import { onMount, untrack } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import {
    BRIDGED_CODE,
    CAPTURE_LEVELS,
    CAPTURE_MODES,
    DBG_SECTIONS,
    DBG_VIEW_CAP,
    DEFAULT_LEVELS,
    LEVELS,
    PRIVACY_NOTE,
    appendEvents,
    captureModeIsRevealing,
    captureModeNote,
    collectAllEvents,
    copyBundle,
    deviceLines,
    eventLine,
    eventParts,
    filterEvents,
    formatDuration,
    inView,
    isFrontend,
    latestSeq,
    levelClass,
    retentionStatus,
    sinkLines,
    sinkSummary,
    makeAliases,
    maybeRedact,
    mediaPathChip,
    mergeMemberRoutePoll,
    routeDisplayChip,
    routeActionLabel,
    routeActionScopeLabel,
    routeExplanation,
    routeFindings,
    routeGroupState,
    routeOverviewCounts,
    routeIsConnected,
    routeIsDisconnected,
    routeLines,
    routePathLabel,
    routeState,
    shownCount,
    taskChip,
    taskConsequence,
    voiceLines,
    webrtcChip,
    type CaptureConfig,
    type DebugLogSink,
    type DbgSection,
    type DebugDevice,
    type DebugServer,
    type DebugVoicePeer,
    type LogEvent,
    type LogStats,
    type MemberRoute,
    type TaskHealth,
  } from "./debug-console";

  let {
    section = "overview",
    servers,
    activeServerId,
    activeFileCount,
    device,
    version,
    nameOf,
    voice,
    onclose,
    onrefreshdevice,
    oncopy,
  }: {
    /** Which section opens first. Error banners can deep-link straight to the relevant one. */
    section?: DbgSection;
    servers: DebugServer[];
    activeServerId: number | null;
    /** Files known for the open server. Per-server totals live in Settings, Storage. */
    activeFileCount: number;
    device: DebugDevice | null;
    version: string;
    /** A member's display name, which the console shows but never logs or copies. */
    nameOf: (fingerprint: string) => string;
    /**
     * A snapshot of the live call, taken on demand.
     *
     * A function rather than a value because `RTCPeerConnection` state changes without notifying
     * anything: holding the objects would render whatever happened to be true at the last redraw,
     * and the redraws come from an unrelated poll. Asking on our own tick is both simpler and
     * honest about how fresh the answer is.
     */
    voice: () => DebugVoicePeer[];
    onclose: () => void;
    onrefreshdevice: () => void;
    oncopy: (text: string) => Promise<void> | void;
  } = $props();

  // `section` is where the console *opens*, not where it stays: the rail owns the choice from the
  // first click onward, and letting the prop reassert itself would drag the user back out of
  // whatever they navigated to. Untracked so that reading it here is deliberate rather than a
  // reactivity mistake.
  let active = $state<DbgSection>(untrack(() => section));
  let events = $state<LogEvent[]>([]);
  let stats = $state<LogStats>({
    errors: 0,
    warnings: 0,
    dropped: 0,
    filtered: 0,
    latest_seq: 0,
    capacity: 0,
    capture: "",
    session_id: "",
  });
  let capture = $state<CaptureConfig | null>(null);
  // What the log file is actually doing, as opposed to what was asked of it. Null until the first
  // read succeeds, and the retention notice says so rather than assuming either way: the command
  // needs an unlocked session, so a console opened over a locked vault genuinely does not know.
  let logSink = $state<DebugLogSink | null>(null);
  /** Which mode the user has asked for but not yet confirmed. Empty when nothing is pending. */
  let pendingMode = $state("");
  let captureError = $state("");
  let showSections = $state(false);
  let redact = $state(false);
  let paused = $state(false);
  let frozen = $state<LogEvent[]>([]);
  let backFilter = $state("");
  let backTarget = $state("");
  let backLevels = $state<string[]>([...DEFAULT_LEVELS]);
  let frontFilter = $state("");
  let frontLevels = $state<string[]>([...DEFAULT_LEVELS]);
  /** Shared by every feed: a trace pasted from an error banner narrows all of them at once. */
  let traceFilter = $state("");
  let routes = $state<Record<number, MemberRoute[]>>({});
  /** Servers whose latest route read failed. Prior rows remain visible but are explicitly stale. */
  let routeUnavailable = $state<Set<number>>(new Set());
  let taskHealth = $state<TaskHealth[]>([]);
  let voicePeers = $state<DebugVoicePeer[]>([]);
  let expanded = $state("");
  let copied = $state("");
  let saving = $state(false);
  /** The file the last save produced, or why it did not. Shown in the footer, not as a toast. */
  let saved = $state("");
  // Aliases live for the console's lifetime rather than per render, so the same address is the same
  // `[ip 2]` every time it appears. Correlation is the evidence: "it keeps dialling the same two
  // addresses" is the whole diagnosis of the hour-long isolation, and a screenshot where both read
  // `[redacted]` no longer says it.
  const aliases = makeAliases();

  /**
   * How often reachability is re-read, as a multiple of the log tick.
   *
   * The log poll is one small message. Reachability is one command per server, and it used to run
   * on the same one-second tick with no guard: five servers meant five round trips per second for
   * as long as the console stayed open, and a tick that overran simply started another on top of
   * it. Dial backoff moves on the scale of tens of seconds, so three is generous. Found by
   * adversarial review (P3-007).
   */
  const ROUTE_TICKS = 3;
  /** Which sections actually render reachability. The others do not pay for it. */
  const ROUTE_SECTIONS: DbgSection[] = ["overview", "network"];

  /**
   * Re-entrancy guards.
   *
   * A poll that takes longer than the interval must not have a second one started on top of it. The
   * previous version could stack them without limit, which is worst precisely when the app is
   * already struggling: the console would then be adding to the load it exists to explain.
   */
  let pollingLog = false;
  let pollingRoutes = false;
  let tick = 0;
  /**
   * Bumped whenever the capture mode changes.
   *
   * A page is rendered natively at whatever the mode was when the read started, so one that was
   * already in flight when the mode changed describes the old setting. Without this it would land
   * in the freshly cleared feed and put two modes' output in one list with nothing saying which was
   * which, which is the confusion the whole mode-on-every-event design exists to prevent.
   */
  let captureGeneration = 0;

  async function pollLog() {
    if (pollingLog) return;
    pollingLog = true;
    const generation = captureGeneration;
    try {
      const page = await invoke<{ events: LogEvent[] } & LogStats>("get_console_log", {
        afterSeq: latestSeq(events),
        limit: 500,
      });
      if (generation !== captureGeneration) return;
      events = appendEvents(events, page.events, DBG_VIEW_CAP);
      stats = page;
    } catch {
      /* the console must never be able to break the app it is observing */
    } finally {
      pollingLog = false;
    }
  }

  async function pollRoutes() {
    if (pollingRoutes) return;
    pollingRoutes = true;
    try {
      // Concurrently rather than one after another: these are independent local reads, and a
      // sequential walk made the whole refresh as slow as the sum of every server on the list.
      // Bind every answer to the id captured before the await. Re-reading `servers` by array index
      // afterward can put A's fingerprints and addresses under B when props reorder mid-poll.
      const requested = servers.map((server) => ({
        id: server.id,
        answer: invoke<MemberRoute[]>("get_member_routes", { server: server.id })
          .then((rows) => ({ ok: true as const, rows }))
          .catch(() => ({ ok: false as const, rows: [] as MemberRoute[] })),
      }));
      const answers = await Promise.all(
        requested.map(async ({ id, answer }) => ({ id, ...(await answer) })),
      );
      const merged = mergeMemberRoutePoll(servers.map((server) => server.id), routes, answers);
      routes = merged.routes;
      routeUnavailable = merged.unavailable;
    } finally {
      pollingRoutes = false;
    }
  }

  async function pollTasks() {
    try {
      taskHealth = await invoke<TaskHealth[]>("get_task_health");
    } catch {
      /* the panel simply does not render; supervision itself is unaffected */
    }
  }

  async function pollSink() {
    try {
      logSink = await invoke<DebugLogSink>("get_debug_logging");
    } catch {
      // Left as it was rather than cleared: a single failed read is not evidence the file stopped,
      // and dropping to "unknown" on a transient error would be its own false statement.
    }
  }

  async function loadCapture() {
    try {
      capture = await invoke<CaptureConfig>("get_capture_config");
    } catch {
      /* the panel simply does not render; capture itself is unaffected */
    }
  }

  onMount(() => {
    // This device's own reachability is the other half of Overview, and it answers "can anyone
    // reach me at all". Fetched on open rather than polled: it changes on the scale of a router
    // lease, not a second.
    onrefreshdevice();
    void loadCapture();
    void pollLog();
    void pollRoutes();
    void pollTasks();
    void pollSink();
    const timer = setInterval(() => {
      // Paused freezes the view, not the capture: the ring keeps filling natively and a resume
      // shows what arrived. Skipping the poll instead would lose it.
      if (paused) return;
      tick += 1;
      void pollLog();
      if (tick % ROUTE_TICKS === 0 && ROUTE_SECTIONS.includes(active)) void pollRoutes();
      // Task health is a small list that changes only when something starts or stops, so it does
      // not need the log's cadence. It is polled everywhere rather than only on Overview, because
      // the rail badge is the point: a dead forwarder should be visible from whichever section
      // somebody happened to open.
      if (tick % ROUTE_TICKS === 0) void pollTasks();
      // The sink's own counters move only when it writes, fails or hits its quota, so this does
      // not need the log's cadence either. It has to keep being read rather than sampled once:
      // a sink that fails or fills up mid-session is exactly when the retention notice matters.
      if (tick % ROUTE_TICKS === 0) void pollSink();
      voicePeers = voice();
    }, 1000);
    return () => clearInterval(timer);
  });

  /**
   * Change what is being captured.
   *
   * Enhanced and Full are confirmed rather than applied on the first click. They are the settings
   * that start writing this device's addresses and protocol detail into something the user is
   * likely to paste to a stranger, and a control that does that on a stray click is not a control.
   */
  async function chooseMode(mode: string) {
    captureError = "";
    if (captureModeIsRevealing(mode) && capture?.mode !== mode) {
      pendingMode = mode;
      return;
    }
    await applyMode(mode);
  }

  async function applyMode(mode: string) {
    pendingMode = "";
    try {
      capture = await invoke<CaptureConfig>("set_capture_mode", { mode });
      // The mode decides how every value is rendered, so what is already on screen was drawn under
      // the old one. Re-reading from the start is the only honest answer: leaving the old lines
      // would show two modes' output in one feed with nothing saying which was which.
      captureGeneration += 1;
      events = [];
      await pollLog();
    } catch (e) {
      captureError = String(e);
    }
  }

  async function setSectionLevel(id: string, level: string) {
    captureError = "";
    try {
      capture = await invoke<CaptureConfig>("set_section_capture", {
        section: id,
        level: level === "off" ? null : level,
      });
    } catch (e) {
      captureError = String(e);
    }
  }

  /**
   * The visible timeline, which is the frozen copy while paused.
   *
   * Every section below is a filter over this one list. They are views of one record rather than
   * separate feeds, so their counts cannot drift apart, and a failure that crossed two layers stays
   * in one order: a send failure usually depends on the exact interleaving, and per-subsystem logs
   * destroy exactly that.
   */
  const shown = $derived(paused ? frozen : events);
  /**
   * The backend section: everything that is not one of the four sections with a view of its own.
   *
   * Decided by the section each event states, not by which crate emitted it. The old rule split on
   * the tracing target, which put every structured webview event, including the voice stack's, into
   * "frontend" because of the process it ran in.
   */
  const backend = $derived(shown.filter((e) => inView(e, "backend")));
  const frontend = $derived(shown.filter(isFrontend));
  const backShown = $derived(
    filterEvents(
      backend,
      { levels: backLevels, target: backTarget, text: backFilter, trace: traceFilter },
      aliases,
      redact,
    ),
  );
  const frontShown = $derived(
    filterEvents(frontend, { levels: frontLevels, text: frontFilter, trace: traceFilter }, aliases, redact),
  );
  /** Per-section error counts for the rail badges, from the same events the feeds render. */
  const backErrors = $derived(backend.filter((e) => e.level === "ERROR").length);
  const backWarns = $derived(backend.filter((e) => e.level === "WARN").length);
  const frontErrors = $derived(frontend.filter((e) => e.level === "ERROR").length);
  const frontWarns = $derived(frontend.filter((e) => e.level === "WARN").length);
  /** Members this node cannot currently reach, excluding servers whose retained rows are stale. */
  const unreachable = $derived(
    Object.entries(routes)
      .filter(([server]) => !routeUnavailable.has(Number(server)))
      .flatMap(([, rows]) => rows)
      .filter(routeIsDisconnected).length,
  );
  /**
   * The network section: transport, reachability, discovery and join.
   *
   * Four canonical sections under one console heading, because from the outside they are one
   * question. The old rule was `target === "catcoms_net"`, which meant a join failure and a relay
   * reservation that timed out were somewhere else entirely, and the transport's own answer to "why
   * can this node not connect" sat next to neither of them.
   */
  const netEvents = $derived(shown.filter((e) => inView(e, "network")).slice(-300));
  /**
   * What the voice path logged this session.
   *
   * Was matched by searching each rendered line for the word "voice", because the voice stack runs
   * in the webview and logged through one target. That is exactly the heuristic the canonical
   * section replaces: it caught unrelated lines that happened to mention voice, and it would have
   * caught nothing at all once the codes stopped spelling the word out.
   */
  const voiceEvents = $derived(shown.filter((e) => inView(e, "voice")).slice(-300));
  /** Vault, blob store and transfers: the section that says what is on this disk. */
  const storageEvents = $derived(shown.filter((e) => inView(e, "storage")).slice(-300));
  /** How much of the record still comes from un-migrated call sites. */
  const bridged = $derived(shown.filter((e) => e.code === BRIDGED_CODE).length);
  /** The newest few loud events, so "why is the badge red" is one glance rather than a hunt. */
  const attention = $derived(
    shown
      .filter((e) => e.level === "ERROR" || e.level === "WARN")
      .slice(-4)
      .reverse(),
  );
  /** An inbound/public candidate observation only; deliberately not an outbound IPv6 route test. */
  const hasPublicIpv6Observation = $derived((device?.public_ipv6.length ?? 0) > 0);
  /**
   * Background tasks in a state somebody should be told about.
   *
   * Shown first and badged on the rail, because a stopped task is the one failure the rest of this
   * console cannot show you: everything else keeps reporting normally while the thing that was
   * meant to be doing the work is gone.
   */
  const brokenTasks = $derived(taskHealth.filter((t) => t.fault));

  function toggleLevel(which: "back" | "front", level: string) {
    const held = which === "back" ? backLevels : frontLevels;
    const next = held.includes(level) ? held.filter((l) => l !== level) : [...held, level];
    if (which === "back") backLevels = next;
    else frontLevels = next;
  }
  function togglePause() {
    paused = !paused;
    // Freeze a copy rather than stopping the poll, so resuming reveals the backlog instead of a
    // hole where the paused interval was.
    if (paused) frozen = [...events];
  }
  function line(e: LogEvent): string {
    return eventLine(e, aliases, redact);
  }
  /** The spans a feed row renders. The joined line is built from these, so they cannot disagree. */
  function parts(e: LogEvent) {
    return eventParts(e, aliases, redact);
  }
  function text(s: string): string {
    return maybeRedact(s, aliases, redact);
  }
  async function copy(what: string, body: string) {
    await oncopy(body);
    copied = what;
    setTimeout(() => (copied = ""), 1500);
  }
  /**
   * Build the report.
   *
   * `full` decides where the log sections come from. Copy uses what is on screen, because that is
   * what the person is looking at and about to paste. Save pages the whole native ring, because a
   * saved report is evidence and evidence that stops at the view boundary arrives missing the
   * run-up to the failure it describes.
   */
  async function buildReport(full: boolean): Promise<string> {
    let sections: LogEvent[][] = [backShown, netEvents, voiceEvents, storageEvents, frontShown];
    if (full) {
      const every = await collectAllEvents((afterSeq, limit) =>
        invoke<{ events: LogEvent[] }>("get_console_log", { afterSeq, limit }).then((p) => p.events),
      );
      sections = (["backend", "network", "voice", "storage", "frontend"] as DbgSection[]).map((v) =>
        every.filter((e) => inView(e, v)),
      );
    }
    const [back, net, vox, store, front] = sections;
    return copyBundle(
      { version, at: Date.now(), redacted: redact, capture: stats.capture, session: stats.session_id },
      [
        { title: "this device", lines: deviceLines(device, aliases, redact) },
        { title: "reachability", lines: routeLines(servers, routes, aliases, redact, hasPublicIpv6Observation, routeUnavailable) },
        { title: "call peers", lines: voiceLines(voicePeers, aliases, redact) },
        // The event sections follow the console's own rail order, so a reader who has seen the
        // screen knows where to look in the file.
        { title: "network", lines: net.map(line) },
        { title: "voice", lines: vox.map(line) },
        { title: "backend", lines: back.map(line) },
        { title: "frontend", lines: front.map(line) },
        { title: "storage", lines: store.map(line) },
      ],
    );
  }

  async function copyReport() {
    await copy("report", await buildReport(false));
  }

  /**
   * Write the report to a file next to the debug log.
   *
   * The clipboard is the fast path for pasting into a chat, and it survives exactly until the next
   * thing you copy. A bug report written the following morning needs the evidence to still exist,
   * and someone reading it would rather have a file to open than a wall of text in a message.
   */
  async function saveReport() {
    if (saving) return;
    saving = true;
    saved = "";
    try {
      const report = await buildReport(true);
      const written = await invoke<{ file: string; bytes: number }>("save_diagnostics_report", {
        text: report,
      });
      saved = written.file;
    } catch (e) {
      saved = `could not save: ${String(e)}`;
    } finally {
      saving = false;
    }
  }
</script>

<div class="dbg" role="dialog" aria-label="Debug console">
  <div class="dbg-head">
    <div class="dbg-title">
      <div class="stx-crumb">DIAGNOSTICS // {DBG_SECTIONS.find((s) => s.id === active)?.label.toUpperCase()}</div>
      <h1>Debug console</h1>
    </div>
    <!-- Session totals, counted natively before the ring evicts anything, so this stays true
         after the offending line has aged out of the view below. -->
    <div class="dbg-sev">
      <span class="dbg-sev-chip" class:err={stats.errors > 0} class:quiet={stats.errors === 0}>
        {stats.errors} ERRORS
      </span>
      <span class="dbg-sev-chip" class:warn={stats.warnings > 0} class:quiet={stats.warnings === 0}>
        {stats.warnings} WARNINGS
      </span>
      <!-- What is being recorded, next to what has been recorded. A Safe view and an Enhanced one
           look alike and mean very different things, so the screen says which it is rather than
           leaving a reader to infer it from whether an address looks complete. -->
      {#if stats.capture}
        <span class="dbg-sev-chip" class:warn={capture?.reveals_addresses} class:quiet={!capture?.reveals_addresses}>
          {stats.capture.toUpperCase()} CAPTURE
        </span>
      {/if}
    </div>
    <div class="dbg-head-actions">
      <!-- Paste the four characters off an error banner and every feed narrows to that one
           operation. The question this console exists for is "what happened when I pressed send",
           and a trace is the only thing that answers it across ten stages and two processes. -->
      <input class="dbg-feed-filter" bind:value={traceFilter} placeholder="Trace, e.g. 7f2c" size="12" />
      <button class="ghost small" onclick={copyReport}>{copied === "report" ? "Copied" : "Copy report"}</button>
      <button class="ghost small" disabled={saving} onclick={saveReport}>
        {saving ? "Saving" : "Save report"}
      </button>
    </div>
    <button type="button" class="stx-esc" onclick={onclose} title="Close (Esc)">
      <span class="stx-esc-ring">✕</span>
      <span>ESC</span>
    </button>
  </div>

  <div class="dbg-body">
    <nav class="dbg-rail">
      {#each DBG_SECTIONS as s (s.id)}
        <button type="button" class="dbg-rail-item" class:active={active === s.id} onclick={() => (active = s.id)}>
          <span>{s.label}</span>
          {#if s.id === "overview" && brokenTasks.length}
            <span class="dbg-rail-count err">{brokenTasks.length}</span>
          {:else if s.id === "backend" && (backErrors || backWarns)}
            <span class="dbg-rail-count" class:err={backErrors > 0} class:warn={backErrors === 0}>{backErrors || backWarns}</span>
          {:else if s.id === "frontend" && (frontErrors || frontWarns)}
            <span class="dbg-rail-count" class:err={frontErrors > 0} class:warn={frontErrors === 0}>{frontErrors || frontWarns}</span>
          {:else if s.id === "network" && unreachable > 0}
            <span class="dbg-rail-count warn">{unreachable}</span>
          {/if}
        </button>
      {/each}
    </nav>

    <div class="dbg-content">
      {#if active === "overview"}
        <div class="dbg-sec">
          <!-- First, because a stopped background task is the one failure the rest of this console
               cannot show: everything else keeps reporting normally while the thing that was meant
               to be doing the work is gone. -->
          {#if brokenTasks.length}
            <div class="dbg-card">
              <div class="dbg-card-h"><span>Stopped working</span></div>
              {#each brokenTasks as t (t.id)}
                {@const chip = taskChip(t)}
                <!-- What the user will see, then what it was. `.dbg-finding` is a flex row, so the
                     technical detail is its own paragraph rather than a third item that floats to
                     the far edge away from the sentence it belongs to. -->
                <p class="dbg-finding {chip.tone === 'danger' ? 'danger' : 'warn'}">
                  <span class="chip {chip.tone}">{chip.label}</span>
                  {taskConsequence(t.kind)}
                </p>
                <p class="muted small dbg-task-detail">
                  {t.kind}{t.server !== null
                    ? ` on ${servers.find((s) => s.id === t.server)?.name ?? `server ${t.server}`}`
                    : ""}{t.cause ? `: ${text(t.cause)}` : ""}
                </p>
              {/each}
            </div>
          {/if}

          <!-- Two independent axes. One switch used to mean choosing between capturing almost
               nothing and capturing the transport narrating every address this device has seen, so
               it stayed off and nobody had a log when they needed one. -->
          <div class="dbg-card">
            <div class="dbg-card-h">
              <span>Capture</span>
              <span class="dbg-card-actions">
                <button class="ghost small" onclick={() => (showSections = !showSections)}>
                  {showSections ? "Hide sections" : "Per-section detail"}
                </button>
              </span>
            </div>
            {#if capture}
              <div class="dbg-modes">
                {#each CAPTURE_MODES as m (m)}
                  <button
                    type="button"
                    class="dbg-mode"
                    class:on={capture.mode === m}
                    aria-pressed={capture.mode === m}
                    onclick={() => chooseMode(m)}
                  >{m.toUpperCase()}</button>
                {/each}
              </div>
              <p class="dbg-note">{captureModeNote(capture.mode)}</p>
              {#if capture.expires_at_restart}
                <p class="dbg-note">This mode is forgotten at the next launch, on purpose.</p>
              {/if}
              {#if pendingMode}
                <!-- Confirmed rather than applied on the first click. These are the settings that
                     start writing this device's own addresses into something a user is likely to
                     paste to a stranger. -->
                <p class="dbg-finding warn">
                  <span class="chip warn">CONFIRM</span>
                  {captureModeNote(pendingMode)}
                </p>
                <div class="dbg-card-actions">
                  <button class="ghost small" onclick={() => applyMode(pendingMode)}>
                    Turn on {pendingMode.toUpperCase()}
                  </button>
                  <button class="ghost small" onclick={() => (pendingMode = "")}>Cancel</button>
                </div>
              {/if}
              {#if captureError}
                <p class="dbg-finding danger"><span class="chip danger">REFUSED</span>{captureError}</p>
              {/if}
              {#if showSections}
                <div class="dbg-table-wrap dbg-capture-sections">
                  <table class="dbg-table">
                    <thead><tr><th>Section</th><th>Shows in</th><th>Captured at</th></tr></thead>
                    <tbody>
                      {#each capture.sections as s (s.id)}
                        <tr>
                          <td class="name-cell">{s.id}</td>
                          <td>{s.view}</td>
                          <td>
                            <select
                              value={s.level ?? "off"}
                              onchange={(e) => setSectionLevel(s.id, e.currentTarget.value)}
                            >
                              <option value="off">off</option>
                              {#each CAPTURE_LEVELS as l (l)}
                                <option value={l}>{l}</option>
                              {/each}
                            </select>
                          </td>
                        </tr>
                      {/each}
                    </tbody>
                  </table>
                </div>
              {/if}
              <p class="muted small">
                Session {stats.session_id || "(none)"}. {stats.filtered.toLocaleString()} event(s)
                not captured by these settings, {bridged.toLocaleString()} of the
                {shown.length.toLocaleString()} shown still from un-migrated call sites.
              </p>
            {:else}
              <div class="dbg-empty">Capture settings unavailable.</div>
            {/if}
          </div>

          <div class="dbg-card">
            <div class="dbg-card-h"><span>Servers</span></div>
            {#if servers.length}
              <div class="dbg-table-wrap">
                <table class="dbg-table">
                  <thead><tr><th>Server</th><th class="num">Claimed peers connected</th><th class="num">Roster</th><th>State</th></tr></thead>
                  <tbody>
                    {#each servers as s (s.id)}
                      {@const rows = routes[s.id] ?? []}
                      {@const unavailable = routeUnavailable.has(s.id)}
                      {@const counts = routeOverviewCounts(rows, unavailable)}
                      {@const state = unavailable ? "unavailable" : routeGroupState(rows)}
                      <tr>
                        <td class="name-cell">{s.name}</td>
                        <td class="num">{counts.connected}</td>
                        <td class="num">{counts.roster}</td>
                        <td>
                          {#if state === "unavailable"}
                            <span class="chip warn">UNAVAILABLE</span>
                          {:else if state === "alone"}
                            <span class="chip faint">ALONE</span>
                          {:else if state === "all-connected"}
                            <span class="chip ok">ALL CLAIMED PEERS CONNECTED HERE</span>
                          {:else if state === "none-connected"}
                            <span class="chip warn">NO CLAIMED PEER CONNECTED HERE</span>
                          {:else if state === "partial"}
                            <span class="chip warn">SOME CLAIMED PEERS CONNECTED HERE</span>
                          {:else}
                            <span class="chip warn">UNKNOWN</span>
                          {/if}
                        </td>
                      </tr>
                    {/each}
                  </tbody>
                </table>
              </div>
              <p class="muted small">These are local connections to self-asserted transport peers from signed member records. They do not prove the person is online or reachable from another network.</p>
            {:else}
              <div class="dbg-empty">No servers joined yet. Reachability below still describes this device.</div>
            {/if}
          </div>

          <div class="dbg-card">
            <div class="dbg-card-h">
              <span>This device</span>
              <span class="dbg-card-actions">
                <button class="ghost small" onclick={() => copy("device", deviceLines(device, aliases, redact).join("\n"))}>
                  {copied === "device" ? "Copied" : "Copy"}
                </button>
              </span>
            </div>
            {#if device}
              <div class="dbg-kv">
                <span class="k">Public IPv4</span>
                <span class="v"><span class="dbg-pii">{device.public_ipv4.map(text).join(", ") || "none observed"}</span></span>
                <span class="k">Public IPv6</span>
                <span class="v"><span class="dbg-pii">{device.public_ipv6.map(text).join(", ") || "none observed"}</span></span>
                <span class="k">Directly reachable</span>
                <span class="v">
                  {#if device.public_direct}<span class="chip ok">YES</span>{:else}<span class="chip warn">NO</span>{/if}
                </span>
                <span class="k">Router maps ports</span>
                <span class="v">
                  {#if device.router_maps}<span class="chip ok">YES</span>{:else}<span class="chip warn">NO</span>{/if}
                </span>
              </div>
              <p class="dbg-note">Public IPv6 observations describe inbound candidates, not a test of this device's outbound IPv6 route.</p>
              <p class="dbg-note">{device.advice}</p>
            {:else}
              <div class="dbg-empty">No reachability report yet. Open a server, or use Settings, Connection, Diagnostics.</div>
            {/if}
          </div>

          <!-- What the log file is doing, as opposed to what was asked of it. Shown here because
               the retention notice down in the feeds points at it: telling somebody the file "may
               hold more" is only useful next to somewhere they can check whether it exists. Every
               row is the sink's own reported state, never the preference. -->
          <div class="dbg-card">
            <div class="dbg-card-h">
              <span>Debug log file</span>
              <span class="dbg-card-actions">
                <button class="ghost small" onclick={() => copy("sink", sinkLines(logSink).join("\n"))}>
                  {copied === "sink" ? "Copied" : "Copy"}
                </button>
              </span>
            </div>
            <p class="dbg-note dbg-tone-{sinkSummary(logSink).tone}">{sinkSummary(logSink).text}</p>
            {#if logSink}
              <div class="dbg-kv">
                {#each sinkLines(logSink) as row (row)}
                  <span class="k">{row.slice(0, row.indexOf(":"))}</span>
                  <span class="v">{row.slice(row.indexOf(":") + 1).trim()}</span>
                {/each}
              </div>
            {/if}
          </div>

          <div class="dbg-card">
            <div class="dbg-card-h"><span>Needs attention</span></div>
            {#if attention.length}
              <div class="dbg-attn">
                {#each attention as e (e.seq)}
                  <button type="button" class="dbg-attn-item {levelClass(e.level)}" onclick={() => (active = isFrontend(e) ? "frontend" : "backend")}>
                    <span class="dbg-ts">{parts(e).ts}</span>
                    <span class="dbg-tag">{e.level}</span>
                    {#if parts(e).target}<span class="dbg-target">{parts(e).target}</span>{/if}
                    <span class="dbg-msg">{parts(e).text}</span>
                  </button>
                {/each}
              </div>
            {:else}
              <div class="dbg-empty">Nothing needs attention.</div>
            {/if}
          </div>
        </div>

      {:else if active === "network"}
        <div class="dbg-sec">
          {#each servers as s (s.id)}
            {@const rows = routes[s.id] ?? []}
            {@const unavailable = routeUnavailable.has(s.id)}
            {@const findings = unavailable ? [] : routeFindings(rows, hasPublicIpv6Observation)}
            {#if findings.length}
              <!-- Conclusions, above the table they were drawn from. Candidate families are
                   observable here; outbound route capability is not. -->
              <div class="dbg-card">
                <div class="dbg-card-h"><span>{s.name}: what is wrong</span></div>
                {#each findings as f (f.code)}
                  <p class="dbg-finding {f.severity}">
                    <span class="chip {f.severity}">{f.code}</span>
                    {f.detail}
                  </p>
                {/each}
              </div>
            {/if}
            <div class="dbg-card">
              <div class="dbg-card-h">
                <span>{s.name}</span>
                <span class="dbg-card-actions">
                  <button class="ghost small" onclick={() => copy("routes", routeLines(servers, routes, aliases, redact, hasPublicIpv6Observation, routeUnavailable).join("\n"))}>
                    {copied === "routes" ? "Copied" : "Copy"}
                  </button>
                </span>
              </div>
              {#if unavailable}
                <p class="dbg-note warn">Member-route refresh is unavailable. Any rows below are the last snapshot and are not current reachability evidence.</p>
              {/if}
              {#if rows.length}
                <div class="dbg-table-wrap">
                  <table class="dbg-table">
                    <thead><tr><th></th><th>Member</th><th>Peer</th><th>State</th><th class="num">Routes</th><th class="num">Seq</th><th class="num">Dial batches</th><th>Cooldown</th></tr></thead>
                    <tbody>
                      {#each rows as r (r.fingerprint)}
                        {@const chip = routeDisplayChip(routeState(r), unavailable)}
                        {@const key = `${s.id}:${r.fingerprint}`}
                        <tr class="dbg-row-toggle" onclick={() => (expanded = expanded === key ? "" : key)}>
                          <td>{expanded === key ? "▾" : "▸"}</td>
                          <td class="name-cell">{nameOf(r.fingerprint)}</td>
                          <td><span class="dbg-pii fp">{text(r.peer) || "none"}</span></td>
                          <td><span class="chip {chip.tone}">{chip.label}</span></td>
                          <td class="num">{r.addresses.length}</td>
                          <td class="num">{r.seq}</td>
                          <td class="num">{r.dial_attempts}</td>
                          <td>{r.next_dial_in_ms > 0 ? formatDuration(r.next_dial_in_ms) : "-"}</td>
                        </tr>
                        {#if expanded === key}
                          <tr class="dbg-row-detail">
                            <td colspan="8">
                              <div class="muted small">{unavailable ? "This retained row cannot establish the member's current connection state." : routeExplanation(r, hasPublicIpv6Observation)}</div>
                              {#if r.active_paths.length}
                                <div class="muted small">{unavailable ? "Last snapshot paths" : "Current paths"}: {r.active_paths.map(routePathLabel).join(", ")}</div>
                              {/if}
                              {#if r.last_success}
                                <div class="muted small">{unavailable ? "At the last snapshot, the last path had opened" : "Last path opened"} {formatDuration(r.last_success.age_ms)} ago: {routePathLabel(r.last_success.path)} (historical only).</div>
                              {/if}
                              {#if !unavailable && r.actions.length}
                                <ul>
                                  {#each r.actions as action}
                                    <li>{routeActionLabel(action)} <span class="muted">({routeActionScopeLabel(action)})</span></li>
                                  {/each}
                                </ul>
                              {/if}
                              {#if r.addresses.length}
                                <ul>
                                  {#each r.addresses as a (a)}
                                    <li><span class="dbg-pii">{text(a)}</span></li>
                                  {/each}
                                </ul>
                              {/if}
                            </td>
                          </tr>
                        {/if}
                      {/each}
                    </tbody>
                  </table>
                </div>
              {:else if unavailable}
                <div class="dbg-empty">No retained member-route snapshot is available.</div>
              {:else}
                <div class="dbg-empty">No other members on this server yet.</div>
              {/if}
            </div>
          {/each}
          {#if servers.length === 0}
            <div class="dbg-empty">No servers joined yet, so there is nothing to reach.</div>
          {/if}

          <div class="dbg-card dbg-feed">
            <div class="dbg-card-h"><span>Network events</span></div>
            <p class="muted small">
              Transport, reachability, discovery and join: dials and their failures, port mapping
              and relay attempts, peer records arriving, and admission. Four layers under one
              heading because from the outside they are one question. Raise the transport section
              in Capture to see dial attempts as well as failures.
            </p>
            <div class="dbg-feed-scroll h-md" role="log">
              {#if netEvents.length}
                {#each netEvents as e (e.seq)}
                  <div class="dbg-line {levelClass(e.level)}">
                    <span class="dbg-ts">{parts(e).ts}</span>
                    <span class="dbg-tag">{e.level}</span>
                    <span class="dbg-target">{parts(e).section}</span>
                    {#if parts(e).trace}<span class="dbg-trace">{parts(e).trace}</span>{/if}
                    <span class="dbg-msg">{parts(e).text}</span>
                  </div>
                {/each}
              {:else}
                <div class="dbg-empty">No network events captured yet.</div>
              {/if}
            </div>
          </div>
        </div>

      {:else if active === "voice"}
        <div class="dbg-sec">
          <div class="dbg-card">
            <div class="dbg-card-h">
              <span>Call peers</span>
              <span class="dbg-card-actions">
                <button class="ghost small" onclick={() => copy("voice", voiceLines(voicePeers, aliases, redact).join("\n"))}>
                  {copied === "voice" ? "Copied" : "Copy"}
                </button>
              </span>
            </div>
            {#if voicePeers.length}
              <div class="dbg-grid">
                {#each voicePeers as p (p.fingerprint)}
                  {@const media = mediaPathChip(p.path)}
                  <div class="dbg-card">
                    <div class="dbg-card-h"><span class="dbg-pii">{text(p.fingerprint)}</span></div>
                    <div class="dbg-kv">
                      <span class="k">Connection</span>
                      <span class="v"><span class="chip {webrtcChip(p.connection).tone}">{webrtcChip(p.connection).label}</span></span>
                      <span class="k">ICE</span>
                      <span class="v"><span class="chip {webrtcChip(p.ice).tone}">{webrtcChip(p.ice).label}</span></span>
                      <span class="k">Signalling</span>
                      <span class="v">{p.signaling}</span>
                      <span class="k">Media path</span>
                      <span class="v"><span class="chip {media.tone}">{media.label}</span></span>
                    </div>
                  </div>
                {/each}
              </div>
            {:else}
              <div class="dbg-empty">Not in a call. This section fills while a call is running.</div>
            {/if}
          </div>

          <div class="dbg-card dbg-feed">
            <div class="dbg-card-h"><span>Signalling and ICE</span></div>
            <p class="muted small">
              Everything the voice path logged this session: failed signals, rejected
              candidates, STUN and TURN errors, and router mapping refusals. Whether it ran in the
              webview or natively, because what matters is that it was about a call.
            </p>
            <div class="dbg-feed-scroll h-md" role="log">
              {#if voiceEvents.length}
                {#each voiceEvents as e (e.seq)}
                  <div class="dbg-line {levelClass(e.level)}">
                    <span class="dbg-ts">{parts(e).ts}</span>
                    <span class="dbg-tag">{e.level}</span>
                    {#if parts(e).trace}<span class="dbg-trace">{parts(e).trace}</span>{/if}
                    <span class="dbg-msg">{parts(e).text}</span>
                  </div>
                {/each}
              {:else}
                <div class="dbg-empty">No voice events this session.</div>
              {/if}
            </div>
          </div>
        </div>

      {:else if active === "backend"}
        <div class="dbg-sec">
          <div class="dbg-card dbg-feed">
            <div class="dbg-card-h"><span>Backend</span></div>
            <div class="dbg-feed-bar">
              <input class="dbg-feed-filter" bind:value={backFilter} placeholder="Filter lines" />
              <input class="dbg-feed-filter" bind:value={backTarget} placeholder="Target, e.g. catcoms_sync" />
              <span class="dbg-lvl-chips">
                {#each LEVELS as l (l)}
                  <button type="button" class="dbg-lvl {levelClass(l)} l-{l === 'ERROR' ? 'err' : l.toLowerCase()}" class:on={backLevels.includes(l)}
                    aria-pressed={backLevels.includes(l)} onclick={() => toggleLevel("back", l)}>{l}</button>
                {/each}
              </span>
              <span class="dbg-feed-count">{shownCount(backShown.length, backend.length)}</span>
              <button class="ghost small" aria-pressed={paused} onclick={togglePause}>{paused ? "Resume" : "Pause"}</button>
              <button class="ghost small" onclick={() => copy("backend", backShown.map(line).join("\n"))}>{copied === "backend" ? "Copied" : "Copy"}</button>
            </div>
            <div class="dbg-feed-scroll h-lg" role="log">
              {#if stats.dropped > 0}
                {@const retention = retentionStatus({
                  dropped: stats.dropped,
                  kept: events.length,
                  sink: logSink,
                  filtered: stats.filtered,
                  events,
                })}
                <div class="dbg-drop-note dbg-tone-{retention.tone}">
                  <div>{retention.ring}</div>
                  <div>{retention.sink}</div>
                  {#if retention.caveats.length}
                    <ul class="dbg-drop-caveats">
                      {#each retention.caveats as caveat (caveat.code)}
                        <li>{caveat.text}</li>
                      {/each}
                    </ul>
                  {/if}
                </div>
              {/if}
              {#if backShown.length}
                {#each backShown as e (e.seq)}
                  <div class="dbg-line {levelClass(e.level)}">
                    <span class="dbg-ts">{parts(e).ts}</span>
                    <span class="dbg-tag">{e.level}</span>
                    <span class="dbg-target">{parts(e).section}</span>
                    {#if parts(e).trace}<span class="dbg-trace">{parts(e).trace}</span>{/if}
                    <span class="dbg-msg">{parts(e).text}</span>
                  </div>
                {/each}
              {:else}
                <div class="dbg-empty">Nothing matches. {backend.length} event(s) captured.</div>
              {/if}
            </div>
          </div>
        </div>

      {:else if active === "frontend"}
        <div class="dbg-sec">
          <div class="dbg-card dbg-feed">
            <div class="dbg-card-h"><span>Frontend</span></div>
            <p class="muted small">
              Console errors and warnings from the webview, plus uncaught exceptions and
              unhandled promise rejections. Half this app runs in the webview.
            </p>
            <div class="dbg-feed-bar">
              <input class="dbg-feed-filter" bind:value={frontFilter} placeholder="Filter lines" />
              <span class="dbg-lvl-chips">
                {#each LEVELS as l (l)}
                  <button type="button" class="dbg-lvl l-{l === 'ERROR' ? 'err' : l.toLowerCase()}" class:on={frontLevels.includes(l)}
                    aria-pressed={frontLevels.includes(l)} onclick={() => toggleLevel("front", l)}>{l}</button>
                {/each}
              </span>
              <span class="dbg-feed-count">{shownCount(frontShown.length, frontend.length)}</span>
              <button class="ghost small" aria-pressed={paused} onclick={togglePause}>{paused ? "Resume" : "Pause"}</button>
              <button class="ghost small" onclick={() => copy("frontend", frontShown.map(line).join("\n"))}>{copied === "frontend" ? "Copied" : "Copy"}</button>
            </div>
            <div class="dbg-feed-scroll h-lg" role="log">
              {#if frontShown.length}
                {#each frontShown as e (e.seq)}
                  <div class="dbg-line {levelClass(e.level)}">
                    <span class="dbg-ts">{parts(e).ts}</span>
                    <span class="dbg-tag">{e.level}</span>
                    {#if parts(e).trace}<span class="dbg-trace">{parts(e).trace}</span>{/if}
                    <span class="dbg-msg">{parts(e).text}</span>
                  </div>
                {/each}
              {:else}
                <div class="dbg-empty">Nothing captured. Frontend errors appear here as they happen.</div>
              {/if}
            </div>
          </div>
        </div>

      {:else}
        <div class="dbg-sec">
          <div class="dbg-card">
            <div class="dbg-card-h"><span>Storage</span></div>
            {#if servers.length}
              <div class="dbg-table-wrap">
                <table class="dbg-table">
                  <thead><tr><th>Server</th><th class="num">Channels</th><th class="num">Files</th></tr></thead>
                  <tbody>
                    {#each servers as s (s.id)}
                      <tr>
                        <td class="name-cell">{s.name}</td>
                        <td class="num">{s.channels?.length ?? 0}</td>
                        <td class="num">{s.id === activeServerId ? activeFileCount : "-"}</td>
                      </tr>
                    {/each}
                  </tbody>
                </table>
              </div>
              <p class="dbg-note">
                File counts are read for the open server only. Settings, Storage has the full
                per-server figures and the repair tools.
              </p>
            {:else}
              <div class="dbg-empty">No servers, nothing stored yet.</div>
            {/if}
          </div>

          <!-- The section had a table and no feed, so an integrity failure, a partial unlock or an
               upload that was abandoned halfway had nowhere of its own to appear. -->
          <div class="dbg-card dbg-feed">
            <div class="dbg-card-h"><span>Storage events</span></div>
            <p class="muted small">
              The vault, the blob store and file transfers: lock state, integrity and repair
              outcomes, and uploads and downloads that started without finishing.
            </p>
            <div class="dbg-feed-scroll h-md" role="log">
              {#if storageEvents.length}
                {#each storageEvents as e (e.seq)}
                  <div class="dbg-line {levelClass(e.level)}">
                    <span class="dbg-ts">{parts(e).ts}</span>
                    <span class="dbg-tag">{e.level}</span>
                    <span class="dbg-target">{parts(e).section}</span>
                    {#if parts(e).trace}<span class="dbg-trace">{parts(e).trace}</span>{/if}
                    <span class="dbg-msg">{parts(e).text}</span>
                  </div>
                {/each}
              {:else}
                <div class="dbg-empty">No storage events this session.</div>
              {/if}
            </div>
          </div>
        </div>
      {/if}
    </div>
  </div>

  <div class="dbg-foot">
    <p class="muted small">
      {PRIVACY_NOTE}
      {#if saved}
        <!-- Named rather than announced and gone: the point of saving is that the file is still
             there tomorrow, so the name has to survive long enough to be written down. -->
        <span class="dbg-saved">Saved as <span class="fp">{saved}</span>, in the log folder.</span>
      {/if}
    </p>
    <label class="dbg-redact">
      <input type="checkbox" bind:checked={redact} />
      <!-- Named for everything it affects. It was "REDACT FOR SCREENSHOTS", which is where it
           started, but it also changes what Copy and Save produce, and a label that undersells
           its own scope invites someone to copy a report believing the toggle did not apply. -->
      <span>REDACT ADDRESSES AND IDS</span>
    </label>
  </div>
</div>
