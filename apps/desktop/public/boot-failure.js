/**
 * The one diagnostic that survives the module bundle failing to load.
 *
 * `main.ts` installs a startup capture as early as a module can, and `App.svelte` installs the real
 * frontend logger later still. Both of those live inside the bundle, so both need the bundle to
 * have loaded. When the bundle itself does not arrive, or will not parse, no application code runs
 * at all: the window is white, nothing is written, and the user cannot tell a failed load from a
 * hung launch from a crash.
 *
 * This file is the answer to that one case. It is served as a static asset from `public/`, it is a
 * classic script rather than a module, and it imports nothing, so it shares no fate with the bundle
 * it is watching. `index.html` already carries a panel that reveals itself on a timer with no script
 * involved; everything here is the enrichment on top of that, which is why a failure to load this
 * file still leaves the user with a message rather than a blank window.
 *
 * Plain JavaScript on purpose. Nothing transforms this file on its way to the browser, so anything
 * that is not browser syntax would ship literally and break the very screen that exists to explain
 * breakage.
 */
(function () {
  "use strict";

  // Looked up on use rather than held. The build hoists the module script into <head> while this
  // one stays in <body>, and a deferred module cannot run before a parser-blocking classic script
  // either way, but nothing here should depend on that: a handler that resolves its own elements
  // works wherever the tag ends up.
  function panelEl() {
    return document.getElementById("boot-failure");
  }

  /** Bounds borrowed from `startup-log.ts`: a startup failure is often a loop, and no rate limiting exists yet. */
  var MAX_NOTES = 10;
  var MAX_CHARS = 2000;

  var notes = [];

  function record(text) {
    if (notes.length >= MAX_NOTES) return;
    var bounded = text.length > MAX_CHARS ? text.slice(0, MAX_CHARS) + " [truncated]" : text;
    // The same script can fail once per retry; repeating it says nothing new.
    if (notes.indexOf(bounded) === -1) notes.push(bounded);
  }

  // --- redaction ---------------------------------------------------------------------------------
  //
  // Copied from `redactStartupText` in `startup-log.ts` rather than imported, for the reason that
  // module gives for not importing its own masking from `debug-console.ts`: the module holding the
  // shared version is one of the things that may have failed, and reaching for it would put the
  // failure inside the report about the failure.

  var LOCATION = /[a-z][a-z0-9+.-]*:\/\/[^\s'"`)\]]+|[A-Za-z]:[\\/][^\s'"`)\]]*|\/[\w.-]+(?:\/[\w.-]+)+/gi;
  var PEER_B58 = /\b12D3Koo[1-9A-HJ-NP-Za-km-z]*/g;
  var LONG_HEX = /\b[0-9a-f]{8,}\b/gi;

  function lastSegment(location) {
    var withoutQuery = location.split(/[?#]/)[0] || "";
    var parts = withoutQuery.split(/[\\/]/).filter(function (part) {
      return part && part.charAt(part.length - 1) !== ":";
    });
    var tail = parts[parts.length - 1] || "";
    return tail ? "[path]/" + tail : "[path]";
  }

  function redact(text) {
    return text
      .replace(LOCATION, function (found) {
        return lastSegment(found);
      })
      .replace(PEER_B58, "[id]")
      .replace(LONG_HEX, "[id]");
  }

  // --- the screen --------------------------------------------------------------------------------

  var actionsDrawn = false;

  function copyButton(label, read) {
    var button = document.createElement("button");
    button.type = "button";
    button.textContent = label;
    button.onclick = function () {
      if (!navigator.clipboard) {
        button.textContent = "Select the text above instead";
        return;
      }
      navigator.clipboard.writeText(read()).then(
        function () {
          button.textContent = "Copied";
        },
        function () {
          button.textContent = "Select the text above instead";
        },
      );
    };
    return button;
  }

  /**
   * Put what was seen on the screen.
   *
   * Two reports, matching the in-app failure screen: a redacted summary that is what the panel
   * offers by default, and the untouched text behind a closed disclosure that says what it
   * contains. The raw one is the useful one often enough to be worth offering, and never safe
   * enough to be worth offering silently.
   */
  function render() {
    var panel = panelEl();
    if (!panel || !notes.length) return;
    var detail = notes.join("\n");
    var summaryEl = document.getElementById("boot-failure-summary");
    var detailEl = document.getElementById("boot-failure-detail");
    if (summaryEl) summaryEl.textContent = redact(detail);
    if (detailEl) {
      detailEl.textContent = detail;
      panel.setAttribute("data-boot-detail", "");
    }
    if (actionsDrawn) return;
    actionsDrawn = true;
    var actions = document.getElementById("boot-failure-actions");
    if (actions && summaryEl) {
      actions.appendChild(
        copyButton("Copy report", function () {
          return summaryEl.textContent || "";
        }),
      );
    }
    var rawActions = document.getElementById("boot-failure-raw-actions");
    if (rawActions && detailEl) {
      rawActions.appendChild(
        copyButton("Copy raw detail", function () {
          return detailEl.textContent || "";
        }),
      );
    }
  }

  /** A failure we watched happen, as opposed to one inferred from the app never arriving. */
  function fail() {
    render();
    var panel = panelEl();
    if (panel) panel.setAttribute("data-boot-failed", "");
  }

  function describe(reason) {
    if (!reason) return "unknown";
    if (reason.stack) return String(reason.stack);
    if (reason.message) return String(reason.message);
    try {
      return String(reason);
    } catch (e) {
      return "unknown";
    }
  }

  // --- what we listen to -------------------------------------------------------------------------

  // Capture phase, because a resource that failed to arrive fires its error event on the element
  // and that event does not bubble. The same listener sees uncaught exceptions, which arrive at the
  // window itself, so a bundle that will not parse and a bundle that will not download both land
  // here.
  function onError(event) {
    var target = event.target;
    if (target && target !== window && target.nodeType === 1) {
      var tag = String(target.tagName || "resource").toLowerCase();
      record("failed to load " + tag + " " + (target.src || target.href || "(no url)"));
      // Only a script says anything about whether the application can run. An image or a font that
      // did not arrive is worth recording and is not worth a full-window alert, and if the app
      // mounts anyway the panel is hidden regardless.
      if (tag === "script") fail();
      return;
    }
    record(
      "uncaught " +
        (event.message || "error") +
        " at " +
        (event.filename || "?") +
        ":" +
        (event.lineno || 0),
    );
    fail();
  }

  function onRejection(event) {
    record("unhandled rejection: " + describe(event.reason));
    fail();
  }

  window.addEventListener("error", onError, true);
  window.addEventListener("unhandledrejection", onRejection);

  // The application arriving is the success signal, and it is the same one the stylesheet uses.
  // Removing the panel outright rather than leaving it hidden keeps a full-window overlay from
  // sitting invisibly over a window that works, and standing down entirely leaves the application's
  // own logger to handle the errors it is far better placed to handle.
  function watchForTheApp() {
    var app = document.getElementById("app");
    if (!app || typeof MutationObserver !== "function") return;
    var observer = new MutationObserver(function () {
      if (!app.firstElementChild) return;
      observer.disconnect();
      window.removeEventListener("error", onError, true);
      window.removeEventListener("unhandledrejection", onRejection);
      var panel = panelEl();
      if (panel && panel.parentNode) panel.parentNode.removeChild(panel);
    });
    observer.observe(app, { childList: true });
  }

  if (document.getElementById("app")) watchForTheApp();
  else document.addEventListener("DOMContentLoaded", watchForTheApp);
})();
