// Live detection stream client.
//
// Connects to the server's detection WebSocket (/api/v2/ws/detections) and
// keeps the connection alive across transient drops with exponential backoff
// plus full jitter (capped), so a Wi-Fi blip or a daemon restart does not
// leave the page silently stale. Self-contained and dependency-free.
//
// Graceful degradation: every DOM touch is optional. If "#live-status" is
// absent there is simply no indicator; if "#live-detections" is absent no list
// is populated. The client also dispatches a "birdnet:detection" CustomEvent
// on document for any page that wants to react.
//
// Resource-friendly on a Pi: the socket is dropped while the tab is hidden and
// re-opened when it becomes visible again.
(function () {
  "use strict";

  var WS_PATH = "/api/v2/ws/detections";

  // The station may be served under a reverse-proxy prefix. The server stamps
  // it on <body data-base-path>; a URL built here from location.host plus a
  // literal path is the one kind the server cannot rewrite on the way out.
  function basePath() {
    return (document.body && document.body.dataset.basePath) || "";
  }
  var MIN_DELAY_MS = 1000; // first retry waits up to 1s
  var MAX_DELAY_MS = 30000; // backoff caps at 30s
  var MAX_LIVE_ROWS = 50; // bound the optional live list

  var ws = null;
  var attempt = 0;
  var reconnectTimer = null;
  var stopped = false;

  function setStatus(state, text) {
    var el = document.getElementById("live-status");
    if (!el) return;
    el.hidden = false;
    el.setAttribute("data-state", state);
    var label = el.querySelector("[data-live-label]");
    (label || el).textContent = text;
  }

  // Exponential backoff with full jitter, capped at MAX_DELAY_MS. Full jitter
  // (random in [0, base]) avoids synchronized reconnect storms.
  function backoffDelayMs() {
    var base = Math.min(MAX_DELAY_MS, MIN_DELAY_MS * Math.pow(2, attempt));
    return Math.floor(Math.random() * base);
  }

  function wsUrl() {
    var proto = window.location.protocol === "https:" ? "wss:" : "ws:";
    return proto + "//" + window.location.host + basePath() + WS_PATH;
  }

  function scheduleReconnect() {
    if (stopped || reconnectTimer !== null) return;
    setStatus("reconnecting", "Reconnecting…");
    var delay = backoffDelayMs();
    reconnectTimer = window.setTimeout(function () {
      reconnectTimer = null;
      connect();
    }, delay);
  }

  function prependDetection(d) {
    var list = document.getElementById("live-detections");
    if (!list) return;

    var row = document.createElement("li");
    row.className = "live-detection";
    if (d.is_new_today) row.setAttribute("data-new", "true");

    var name = document.createElement("span");
    name.className = "live-detection-name";
    // textContent (never innerHTML) so a crafted species name cannot inject
    // markup into the page.
    name.textContent = d.common_name || d.scientific_name || "Unknown";

    var meta = document.createElement("span");
    meta.className = "live-detection-meta";
    var conf =
      typeof d.confidence === "number"
        ? Math.round(d.confidence * 100) + "%"
        : "";
    meta.textContent = (d.time || "") + (conf ? " · " + conf : "");

    row.appendChild(name);
    row.appendChild(meta);
    list.insertBefore(row, list.firstChild);

    while (list.children.length > MAX_LIVE_ROWS) {
      list.removeChild(list.lastChild);
    }
  }

  function connect() {
    if (stopped || document.hidden) return;

    var socket;
    try {
      socket = new WebSocket(wsUrl());
    } catch (e) {
      scheduleReconnect();
      return;
    }
    ws = socket;

    socket.onopen = function () {
      attempt = 0;
      setStatus("live", "Live");
    };

    socket.onmessage = function (ev) {
      var data;
      try {
        data = JSON.parse(ev.data);
      } catch (e) {
        return;
      }
      if (!data || data.event !== "detection") return;
      document.dispatchEvent(
        new CustomEvent("birdnet:detection", { detail: data }),
      );
      prependDetection(data);
    };

    socket.onclose = function () {
      if (ws === socket) ws = null;
      // Cap the exponent so Math.pow stays bounded; MAX_DELAY_MS is the real
      // ceiling on the wait.
      attempt = Math.min(attempt + 1, 16);
      scheduleReconnect();
    };

    socket.onerror = function () {
      // An error is always followed by close; close defensively so the
      // onclose path (and its backoff) runs exactly once.
      try {
        socket.close();
      } catch (e) {
        /* ignore */
      }
    };
  }

  function start() {
    document.addEventListener("visibilitychange", function () {
      if (document.hidden) {
        if (ws) {
          try {
            ws.close();
          } catch (e) {
            /* ignore */
          }
        }
      } else if (ws === null && reconnectTimer === null) {
        // Reconnect promptly (no backoff) when the operator returns to the tab.
        attempt = 0;
        connect();
      }
    });

    window.addEventListener("beforeunload", function () {
      stopped = true;
      if (reconnectTimer !== null) {
        window.clearTimeout(reconnectTimer);
        reconnectTimer = null;
      }
      if (ws) {
        try {
          ws.close();
        } catch (e) {
          /* ignore */
        }
      }
    });

    connect();
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", start);
  } else {
    start();
  }
})();
