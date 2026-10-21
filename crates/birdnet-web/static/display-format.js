// Prints every clock time and temperature in the clock and unit the reader chose.
//
// The server writes canonical text (24-hour, degrees Celsius) and marks it with
// the value it was made from (`data-clk="16:31:37"`, `data-tc="21.4"`; see
// src/display.rs). This script re-prints each marked element in the choice in
// force. With scripting off the canonical text stands.
//
// THE CHOICE
//   Per browser, on the Display card (Settings > General): `bnb-time`
//   ('auto' | '24h' | '12h') and `bnb-temp` ('auto' | 'celsius' | 'fahrenheit'),
//   in localStorage (guarded: blocked storage means "not remembered").
//   'auto' follows the station's default (`time_format`, `temperature_unit`),
//   which comes from /api/v2/display and is kept in sessionStorage so a page
//   after the first one in a tab is printed right on first paint.
//
// THE FORMATS
//   The same as src/display.rs, held to the same table of cases
//   (tests/data/display_cases.json) by tools/visual-qa/display-format.mjs.
//   Rounding is half away from zero, as there; `toFixed` is not used.
(function () {
  "use strict";

  var TIME_KEY = "bnb-time";
  var TEMP_KEY = "bnb-temp";
  var DEFAULT_KEY = "bnb-display-default";

  function two(s, i) {
    var a = s.charCodeAt(i), b = s.charCodeAt(i + 1);
    if (!(a >= 48 && a <= 57 && b >= 48 && b <= 57)) return -1;
    return (a - 48) * 10 + (b - 48);
  }

  // "HH:MM" or "HH:MM:SS" -> [h, m, s|null], or null.
  function parseClock(t) {
    if (typeof t !== "string") return null;
    var h = two(t, 0), m = two(t, 3);
    if (t.charAt(2) !== ":" || h < 0 || m < 0 || h > 23 || m > 59) return null;
    if (t.length === 5) return [h, m, null];
    if (t.length === 8 && t.charAt(5) === ":") {
      var s = two(t, 6);
      return s >= 0 && s <= 59 ? [h, m, s] : null;
    }
    return null;
  }

  function pad(n) { return (n < 10 ? "0" : "") + n; }

  function formatClock(t, format) {
    var p = parseClock(t);
    if (!p) return null;
    if (format !== "12h") return t;
    var hour = p[0] % 12 === 0 ? 12 : p[0] % 12;
    var suffix = p[0] < 12 ? "AM" : "PM";
    return hour + ":" + pad(p[1]) + (p[2] === null ? "" : ":" + pad(p[2])) + " " + suffix;
  }

  function formatTemperature(celsius, decimals, unit, withUnit) {
    var fahrenheit = unit === "fahrenheit";
    var value = fahrenheit ? celsius * 9 / 5 + 32 : celsius;
    var scale = Math.pow(10, decimals);
    var rounded = Math.round(Math.abs(value) * scale) / scale;
    var sign = value < 0 && rounded !== 0 ? "-" : "";
    return sign + rounded.toFixed(decimals) + "°" + (withUnit ? (fahrenheit ? "F" : "C") : "");
  }

  // ---- the choice -----------------------------------------------------------

  function read(key, allowed) {
    try {
      var v = localStorage.getItem(key);
      return allowed.indexOf(v) >= 0 ? v : "auto";
    } catch (e) { return "auto"; }
  }

  var station = { time: "24h", temperature: "celsius" };
  try {
    var cached = JSON.parse(sessionStorage.getItem(DEFAULT_KEY) || "null");
    if (cached && (cached.time === "12h" || cached.time === "24h")) station.time = cached.time;
    if (cached && (cached.temperature === "fahrenheit" || cached.temperature === "celsius")) station.temperature = cached.temperature;
  } catch (e) { /* no cached default: the canonical one */ }

  function effective() {
    var t = read(TIME_KEY, ["auto", "24h", "12h"]);
    var u = read(TEMP_KEY, ["auto", "celsius", "fahrenheit"]);
    return {
      time: t === "auto" ? station.time : t,
      temperature: u === "auto" ? station.temperature : u
    };
  }

  // ---- printing -------------------------------------------------------------

  function printClock(el, f) {
    var shown = formatClock(el.getAttribute("data-clk"), f.time);
    if (shown !== null && el.textContent !== shown) el.textContent = shown;
  }

  function printTemperature(el, f) {
    var c = parseFloat(el.getAttribute("data-tc"));
    if (!isFinite(c)) return;
    var dp = parseInt(el.getAttribute("data-dp") || "0", 10);
    var shown = formatTemperature(c, dp >= 0 && dp <= 4 ? dp : 0, f.temperature, el.hasAttribute("data-tu"));
    if (el.textContent !== shown) el.textContent = shown;
  }

  function printAll(root) {
    var f = effective();
    var scope = root && root.querySelectorAll ? root : document;
    var clocks = scope.querySelectorAll("[data-clk]");
    for (var i = 0; i < clocks.length; i++) printClock(clocks[i], f);
    var temps = scope.querySelectorAll("[data-tc]");
    for (var j = 0; j < temps.length; j++) printTemperature(temps[j], f);
    if (scope !== document && scope.getAttribute) {
      if (scope.hasAttribute("data-clk")) printClock(scope, f);
      if (scope.hasAttribute("data-tc")) printTemperature(scope, f);
    }
  }

  function basePath() {
    return (document.body && document.body.dataset.basePath) || "";
  }

  // The station's default, once per page load: from sessionStorage at once (set
  // by an earlier page in this tab), revalidated in the background.
  function fetchStationDefault() {
    if (!window.fetch) return;
    fetch(basePath() + "/api/v2/display", { headers: { Accept: "application/json" } })
      .then(function (r) { return r.ok ? r.json() : null; })
      .then(function (d) {
        if (!d) return;
        var next = {
          time: d.time_format === "12h" ? "12h" : "24h",
          temperature: d.temperature_unit === "fahrenheit" ? "fahrenheit" : "celsius"
        };
        try { sessionStorage.setItem(DEFAULT_KEY, JSON.stringify(next)); } catch (e) { /* not kept */ }
        if (next.time !== station.time || next.temperature !== station.temperature) {
          station = next;
          printAll(document);
        }
      })
      .catch(function () { /* offline: the canonical or cached default stands */ });
  }

  // Content that arrives later (htmx swaps, charts drawn by script).
  function watch() {
    if (!window.MutationObserver) return;
    new MutationObserver(function (records) {
      for (var i = 0; i < records.length; i++) {
        var added = records[i].addedNodes;
        for (var j = 0; j < added.length; j++) {
          if (added[j].nodeType === 1) printAll(added[j]);
        }
      }
    }).observe(document.documentElement, { childList: true, subtree: true });
  }

  window.bnbDisplay = {
    formatClock: formatClock,
    formatTemperature: formatTemperature,
    refresh: function () { printAll(document); },
    effective: effective
  };

  function start() {
    printAll(document);
    watch();
    fetchStationDefault();
  }
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", start);
  else start();

  // Another tab changed the choice.
  window.addEventListener("storage", function (e) {
    if (e.key === TIME_KEY || e.key === TEMP_KEY || e.key === null) printAll(document);
  });
})();
