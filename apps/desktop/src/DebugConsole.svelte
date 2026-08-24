<script lang="ts">
  /**
   * The in-app debug console (`docs/design-debug-console.md`).
   *
   * The app used to fail silently: a call died while the roster still showed the peer online, and a
   * node sat isolated for an hour dialling addresses it could never reach with not one dial failure
   * surfaced anywhere. This is the live view of what the diagnostics know, segmented so the failing
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
    DBG_SECTIONS,
    DBG_VIEW_CAP,
    DEFAULT_LEVELS,
    LEVELS,
    PRIVACY_NOTE,
    appendEvents,
    collectAllEvents,
    copyBundle,
    deviceLines,
    dropNote,
    eventLine,
    eventParts,
    eventText,
    filterEvents,
    formatDuration,
    isFrontend,
    latestSeq,
    levelClass,
    makeAliases,
    maybeRedact,
    mediaPathChip,
    routeChip,
    routeExplanation,
    routeFindings,
    routeLines,
    routeState,
    shownCount,
    voiceLines,
    webrtcChip,
    type DbgSection,
    type DebugDevice,
    type DebugServer,
    type DebugVoicePeer,
    type LogEvent,
    type LogStats,
    type MemberRoute,
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
  let stats = $state<LogStats>({ errors: 0, warnings: 0, dropped: 0, latest_seq: 0, capacity: 0 });
  let redact = $state(false);
  let paused = $state(false);
  let frozen = $state<LogEvent[]>([]);
  let backFilter = $state("");
  let backTarget = $state("");
  let backLevels = $state<string[]>([...DEFAULT_LEVELS]);
  let frontFilter = $state("");
  let frontLevels = $state<string[]>([...DEFAULT_LEVELS]);
  let routes = $state<Record<number, MemberRoute[]>>({});
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

  async function poll() {
    try {
      const page = await invoke<{ events: LogEvent[] } & LogStats>("get_console_log", {
        afterSeq: latestSeq(events),
        limit: 500,
      });
      events = appendEvents(events, page.events, DBG_VIEW_CAP);
      stats = page;
    } catch {
      /* the console must never be able to break the app it is observing */
    }
    // Reachability per server, for the Network section. Cheap: local state, no wire traffic.
    const next: Record<number, MemberRoute[]> = {};
    for (const s of servers) {
      try {
        next[s.id] = await invoke<MemberRoute[]>("get_member_routes", { server: s.id });
      } catch {
        next[s.id] = [];
      }
    }
    routes = next;
    voicePeers = voice();
  }

  onMount(() => {
    // This device's own reachability is the other half of Overview, and it answers "can anyone
    // reach me at all". Fetched on open rather than polled: it changes on the scale of a router
    // lease, not a second.
    onrefreshdevice();
    void poll();
    const timer = setInterval(() => {
      // Paused freezes the view, not the capture: the ring keeps filling natively and a resume
      // shows what arrived. Skipping the poll instead would lose it.
      if (!paused) void poll();
    }, 1000);
    return () => clearInterval(timer);
  });

  /** The backend half of the ring: everything Rust emitted. */
  const backend = $derived((paused ? frozen : events).filter((e) => !isFrontend(e)));
  /** The frontend half: whatever the webview logged, which arrives under one tracing target. */
  const frontend = $derived((paused ? frozen : events).filter(isFrontend));
  const backShown = $derived(
    filterEvents(backend, { levels: backLevels, target: backTarget, text: backFilter }, aliases, redact),
  );
  const frontShown = $derived(filterEvents(frontend, { levels: frontLevels, text: frontFilter }, aliases, redact));
  /** Per-section error counts for the rail badges, from the same events the feeds render. */
  const backErrors = $derived(backend.filter((e) => e.level === "ERROR").length);
  const backWarns = $derived(backend.filter((e) => e.level === "WARN").length);
  const frontErrors = $derived(frontend.filter((e) => e.level === "ERROR").length);
  const frontWarns = $derived(frontend.filter((e) => e.level === "WARN").length);
  /** Members this node cannot currently reach, across every server: the Network badge. */
  const unreachable = $derived(
    Object.values(routes)
      .flat()
      .filter((r) => routeState(r) !== "connected").length,
  );
  /** The transport's own events: dial attempts, dial failures, connection churn. */
  const netEvents = $derived(backend.filter((e) => e.target === "catcoms_net").slice(-300));
  /**
   * What the voice path logged this session.
   *
   * Matched on text rather than a dedicated target because the voice stack lives in the webview and
   * logs through the one `catcoms_ui` target. The whole rendered line is searched, not just the
   * message: a structured event carries its code in a field (`code=VOICE.SIGNAL.NO_MEMBER_ROUTE`)
   * while an old-style console line carries it in the message, and matching only the message would
   * have quietly dropped every migrated event out of this section.
   *
   * A stopgap either way. Events already state their section natively, and the console reads that
   * directly once it is rebuilt on the hub.
   */
  const voiceEvents = $derived(
    frontend.filter((e) => eventText(e).toLowerCase().includes("voice")).slice(-300),
  );
  /** The newest few loud events, so "why is the badge red" is one glance rather than a hunt. */
  const attention = $derived(
    (paused ? frozen : events)
      .filter((e) => e.level === "ERROR" || e.level === "WARN")
      .slice(-4)
      .reverse(),
  );
  /** Whether this device has any usable IPv6 route, which decides how a v6-only peer is explained. */
  const hasIpv6 = $derived((device?.public_ipv6.length ?? 0) > 0);

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
    let backend = backShown;
    let frontend = frontShown;
    if (full) {
      const every = await collectAllEvents((afterSeq, limit) =>
        invoke<{ events: LogEvent[] }>("get_console_log", { afterSeq, limit }).then((p) => p.events),
      );
      backend = every.filter((e) => !isFrontend(e));
      frontend = every.filter(isFrontend);
    }
    return copyBundle({ version, at: Date.now(), redacted: redact }, [
      { title: "this device", lines: deviceLines(device, aliases, redact) },
      { title: "network", lines: routeLines(servers, routes, aliases, redact, hasIpv6) },
      { title: "voice", lines: voiceLines(voicePeers, aliases, redact) },
      { title: "backend", lines: backend.map(line) },
      { title: "frontend", lines: frontend.map(line) },
    ]);
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
    </div>
    <div class="dbg-head-actions">
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
          {#if s.id === "backend" && (backErrors || backWarns)}
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
          <div class="dbg-card">
            <div class="dbg-card-h"><span>Servers</span></div>
            {#if servers.length}
              <div class="dbg-table-wrap">
                <table class="dbg-table">
                  <thead><tr><th>Server</th><th class="num">Reachable</th><th class="num">Roster</th><th>State</th></tr></thead>
                  <tbody>
                    {#each servers as s (s.id)}
                      {@const rows = routes[s.id] ?? []}
                      {@const live = rows.filter((r) => r.connected).length}
                      <tr>
                        <td class="name-cell">{s.name}</td>
                        <td class="num">{live} / {rows.length}</td>
                        <td class="num">{rows.length + 1}</td>
                        <td>
                          {#if rows.length === 0}
                            <span class="chip faint">ALONE</span>
                          {:else if live === rows.length}
                            <span class="chip ok">ALL REACHED</span>
                          {:else if live === 0}
                            <span class="chip danger">NONE REACHED</span>
                          {:else}
                            <span class="chip warn">PARTIAL</span>
                          {/if}
                        </td>
                      </tr>
                    {/each}
                  </tbody>
                </table>
              </div>
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
              {#if !hasIpv6}
                <!-- The failure that stranded a node for an hour: every dial went to an IPv6
                     address this machine has no route to, and failed instantly. -->
                <p class="dbg-note">
                  No usable IPv6 route on this device. A member whose record advertises only
                  IPv6 addresses cannot be dialled from here at all, and each attempt fails
                  immediately rather than timing out.
                </p>
              {/if}
              <p class="dbg-note">{device.advice}</p>
            {:else}
              <div class="dbg-empty">No reachability report yet. Open a server, or use Settings, Connection, Diagnostics.</div>
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
            {@const findings = routeFindings(rows, hasIpv6)}
            {#if findings.length}
              <!-- Conclusions, above the table they were drawn from. Working this out used to mean
                   reading multiaddrs one at a time and knowing whether this host had IPv6. -->
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
                  <button class="ghost small" onclick={() => copy("routes", routeLines(servers, routes, aliases, redact, hasIpv6).join("\n"))}>
                    {copied === "routes" ? "Copied" : "Copy"}
                  </button>
                </span>
              </div>
              {#if rows.length}
                <div class="dbg-table-wrap">
                  <table class="dbg-table">
                    <thead><tr><th></th><th>Member</th><th>Peer</th><th>State</th><th class="num">Routes</th><th class="num">Seq</th><th class="num">Fails</th><th>Next dial</th></tr></thead>
                    <tbody>
                      {#each rows as r (r.fingerprint)}
                        {@const chip = routeChip(routeState(r))}
                        {@const key = `${s.id}:${r.fingerprint}`}
                        <tr class="dbg-row-toggle" onclick={() => (expanded = expanded === key ? "" : key)}>
                          <td>{expanded === key ? "▾" : "▸"}</td>
                          <td class="name-cell">{nameOf(r.fingerprint)}</td>
                          <td><span class="dbg-pii fp">{text(r.peer) || "none"}</span></td>
                          <td><span class="chip {chip.tone}">{chip.label}</span></td>
                          <td class="num">{r.addresses.length}</td>
                          <td class="num">{r.seq}</td>
                          <td class="num" class:bad={r.dial_attempts > 0}>{r.dial_attempts}</td>
                          <td>{r.next_dial_in_ms > 0 ? formatDuration(r.next_dial_in_ms) : "-"}</td>
                        </tr>
                        {#if expanded === key}
                          <tr class="dbg-row-detail">
                            <td colspan="8">
                              <div class="muted small">{routeExplanation(r, hasIpv6)}</div>
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
              {:else}
                <div class="dbg-empty">No other members on this server yet.</div>
              {/if}
            </div>
          {/each}
          {#if servers.length === 0}
            <div class="dbg-empty">No servers joined yet, so there is nothing to reach.</div>
          {/if}

          <div class="dbg-card dbg-feed">
            <div class="dbg-card-h"><span>Transport events</span></div>
            <p class="muted small">
              Dial attempts, dial failures and connection churn, straight from the transport.
              Turn on DEBUG in Backend to see attempts as well as failures.
            </p>
            <div class="dbg-feed-scroll h-md" role="log">
              {#if netEvents.length}
                {#each netEvents as e (e.seq)}
                  <div class="dbg-line {levelClass(e.level)}">
                    <span class="dbg-ts">{parts(e).ts}</span>
                    <span class="dbg-tag">{e.level}</span>
                    <span class="dbg-msg">{parts(e).text}</span>
                  </div>
                {/each}
              {:else}
                <div class="dbg-empty">No transport events captured yet.</div>
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
              candidates, STUN and TURN errors, and router mapping refusals.
            </p>
            <div class="dbg-feed-scroll h-md" role="log">
              {#if voiceEvents.length}
                {#each voiceEvents as e (e.seq)}
                  <div class="dbg-line {levelClass(e.level)}">
                    <span class="dbg-ts">{parts(e).ts}</span>
                    <span class="dbg-tag">{e.level}</span>
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
              <input class="dbg-feed-filter" bind:value={backTarget} placeholder="Target, e.g. catcoms_net" />
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
                <div class="dbg-drop-note">{dropNote(stats.dropped, events.length)}</div>
              {/if}
              {#if backShown.length}
                {#each backShown as e (e.seq)}
                  <div class="dbg-line {levelClass(e.level)}">
                    <span class="dbg-ts">{parts(e).ts}</span>
                    <span class="dbg-tag">{e.level}</span>
                    <span class="dbg-target">{e.target}</span>
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
      <span>REDACT FOR SCREENSHOTS</span>
    </label>
  </div>
</div>
