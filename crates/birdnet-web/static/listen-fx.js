// Listening aids for every player on every page: a gain boost, a high-pass
// filter and a compressor, applied in this browser with the Web Audio API.
//
// WHAT IT IS
//   Quiet birds on a noisy station are the common case: a call recorded at
//   -40 dBFS under wind and road rumble. The three controls are the ones the
//   upstream birdnet-go player offers (frontend/src/lib/utils/audioNodes.ts,
//   createAudioNodeChain):  source -> high-pass -> gain -> [compressor] -> out,
//   with its gain ceiling of +24 dB and its compressor settings. One thing is
//   added at the end: a soft limiter. Upstream's chain can exceed full scale
//   (+24 dB on a loud clip is sixteen times too loud) and the browser then
//   clips it; here nothing leaves the chain above CEILING.
//
// WHAT IT NEVER DOES
//   It never touches the stored recording or the download: it only changes the
//   sound this browser plays. The settings live in this browser's localStorage
//   (guarded: a private window or blocked storage just means "not remembered").
//
// HOW IT REACHES EVERY PLAYER
//   There are several players (the shared quick-play button, the Clips dock,
//   the live listen, native <audio controls> on detail pages), some of them
//   detached `new Audio()` objects no page script could be asked to register.
//   `HTMLMediaElement.prototype.play` is therefore wrapped once: the first time
//   an element is played, while the click that asked for it is still on the
//   stack (so the AudioContext may start), it is routed through the chain.
//   When the settings are neutral (the default) nothing is routed at all and
//   the element plays through the browser's own path, exactly as before.
//
// SAFETY OF THE ROUTING
//   `createMediaElementSource` silences an element whose audio is cross-origin
//   and permanently reroutes its output, so: only same-origin sources are ever
//   routed; a context that is not running is never used (on iOS the element
//   would play silently while its clock advanced); and any failure while
//   building the chain connects the source straight to the output, so the
//   worst case is "plays unprocessed", never "plays nothing".
//
// `[data-listen-fx]` marks an element the controls are built into; the script
// is loaded by every full page (layout.html) and by the standalone player.
(function () {
  "use strict";

  var KEY = "bnb-listen";
  var GAIN_MAX = 24;       // dB, upstream's MAX_GAIN_DB
  var HP_OFF = 20;         // Hz, upstream's DEFAULT_HIGH_PASS_FREQ (below hearing)
  var HP_MAX = 8000;       // Hz; birdsong lives above it, rumble and wind below
  // Upstream's compressor, constant for constant.
  var COMP = { threshold: -24, knee: 30, ratio: 12, attack: 0.003, release: 0.25 };
  // The limiter is a static curve: identity up to KNEE, then a smooth tanh
  // shoulder that approaches CEILING and never reaches it. It sees the signal
  // through a x1/HEADROOM gain so excursions up to HEADROOM times full scale
  // are shaped rather than flattened by the node's own input clamp.
  var CEILING = 0.98;
  var KNEE = 0.8;
  var HEADROOM = 4;
  var CURVE_POINTS = 8193;

  var DEFAULTS = { gain: 0, hp: HP_OFF, comp: false };

  function clamp(v, lo, hi) { return Math.min(hi, Math.max(lo, v)); }

  // A stored or submitted value, made safe: numbers only, clamped, rounded.
  function clean(o) {
    o = o && typeof o === "object" ? o : {};
    var gain = Number(o.gain), hp = Number(o.hp);
    return {
      gain: isFinite(gain) ? Math.round(clamp(gain, 0, GAIN_MAX)) : DEFAULTS.gain,
      hp: isFinite(hp) ? Math.round(clamp(hp, HP_OFF, HP_MAX)) : DEFAULTS.hp,
      comp: o.comp === true
    };
  }

  function isNeutral(s) { return s.gain === 0 && s.hp <= HP_OFF && !s.comp; }

  function dbToGain(db) { return Math.pow(10, db / 20); }

  // "Off" is a corner of 0 Hz, which the Web Audio spec defines as a filter
  // that passes everything. A 20 Hz corner is not off: measured on BirdNET-Pi's
  // magpie recording it removed 10.7 dB of the clip's RMS (a large DC offset and
  // infrasonic rumble), so a "neutral" chain was not neutral.
  function cornerHz(hp) { return hp <= HP_OFF ? 0 : hp; }

  // The limiter's transfer curve over [-1, 1], which stands for [-HEADROOM, HEADROOM].
  function limiterCurve() {
    var curve = new Float32Array(CURVE_POINTS);
    var span = CEILING - KNEE;
    for (var i = 0; i < CURVE_POINTS; i++) {
      var x = ((i / (CURVE_POINTS - 1)) * 2 - 1) * HEADROOM;
      var a = Math.abs(x);
      var y = a <= KNEE ? a : KNEE + span * Math.tanh((a - KNEE) / span);
      curve[i] = x < 0 ? -y : y;
    }
    return curve;
  }

  // The chain, on any BaseAudioContext and any source node, so the page and the
  // offline measurement in the test suite build exactly the same graph:
  //   source -> high-pass -> gain -> [compressor] -> x1/HEADROOM -> limiter -> destination
  function buildChain(ctx, source, destination, settings, options) {
    var s = clean(settings);
    var withLimiter = !(options && options.limiter === false);
    var hp = ctx.createBiquadFilter();
    hp.type = "highpass";
    // Q = 1/sqrt(2) is the flat (Butterworth) response; upstream's Q of 1 puts
    // a 1.25 dB bump at the corner.
    hp.Q.value = Math.SQRT1_2;
    hp.frequency.value = cornerHz(s.hp);
    var gain = ctx.createGain();
    gain.gain.value = dbToGain(s.gain);
    var comp = ctx.createDynamicsCompressor();
    comp.threshold.value = COMP.threshold;
    comp.knee.value = COMP.knee;
    comp.ratio.value = COMP.ratio;
    comp.attack.value = COMP.attack;
    comp.release.value = COMP.release;
    var pre = ctx.createGain();
    pre.gain.value = 1 / HEADROOM;
    var shaper = ctx.createWaveShaper();
    shaper.curve = limiterCurve();
    // Not oversampled: the oversampler's reconstruction filter rings on a
    // flat-topped signal and was measured to put the output 1.4 dB OVER full
    // scale on a boosted real clip (peak 1.14 with "2x"). With none, every
    // output sample is a point on the curve, so it cannot pass CEILING.
    shaper.oversample = "none";

    var chain = {
      hp: hp, gain: gain, comp: comp, limiter: shaper, ctx: ctx,
      compOn: false,
      // The node that feeds the limiter, so the compressor can be switched in.
      tail: function () { return withLimiter ? pre : destination; },
      apply: function (next) {
        var n = clean(next);
        var t = ctx.currentTime;
        // Short ramps: a stepped gain or corner frequency clicks.
        hp.frequency.setTargetAtTime(cornerHz(n.hp), t, 0.015);
        gain.gain.setTargetAtTime(dbToGain(n.gain), t, 0.015);
        if (n.comp !== chain.compOn) {
          gain.disconnect();
          if (n.comp) { gain.connect(comp); comp.connect(chain.tail()); }
          else { try { comp.disconnect(); } catch (e) { /* not connected */ } gain.connect(chain.tail()); }
          chain.compOn = n.comp;
        }
      }
    };

    source.connect(hp);
    hp.connect(gain);
    if (s.comp) { gain.connect(comp); comp.connect(chain.tail()); chain.compOn = true; }
    else { gain.connect(chain.tail()); }
    if (withLimiter) { pre.connect(shaper); shaper.connect(destination); }
    return chain;
  }

  // ---- preferences ---------------------------------------------------------

  var current = clean(DEFAULTS);
  var listeners = [];

  function load() {
    try {
      var raw = localStorage.getItem(KEY);
      return clean(raw ? JSON.parse(raw) : DEFAULTS);
    } catch (e) { return clean(DEFAULTS); }
  }

  function save(s) {
    try { localStorage.setItem(KEY, JSON.stringify(s)); } catch (e) { /* not remembered */ }
  }

  function set(next, persist) {
    current = clean(next);
    if (persist !== false) save(current);
    chains.forEach(function (c) { try { c.apply(current); } catch (e) { /* node gone */ } });
    // A clip already playing natively picks the settings up now, not at its next play.
    if (!isNeutral(current)) seen.forEach(function (el) { if (!el.paused) attach(el); });
    listeners.forEach(function (fn) { try { fn(current); } catch (e) { /* a listener must not break the rest */ } });
  }

  // ---- routing of media elements -------------------------------------------

  var ctx = null;
  var routed = typeof WeakMap === "function" ? new WeakMap() : null;
  var chains = [];
  var seen = [];

  function contextClass() { return window.AudioContext || window.webkitAudioContext || null; }

  function getContext() {
    if (ctx) return ctx;
    var C = contextClass();
    if (!C) return null;
    try { ctx = new C({ latencyHint: "playback" }); }
    catch (e) {
      try { ctx = new C(); } catch (e2) { ctx = null; }
    }
    return ctx;
  }

  function sameOrigin(el) {
    var src = el.currentSrc || el.src;
    if (!src) {
      var child = el.querySelector && el.querySelector("source[src]");
      src = child ? child.src : "";
    }
    if (!src) return false;
    try { return new URL(src, location.href).origin === location.origin; } catch (e) { return false; }
  }

  function attach(el) {
    if (!el || !routed || routed.has(el) || isNeutral(current)) return;
    if (!sameOrigin(el)) return;
    var c = getContext();
    if (!c) return;
    if (c.state !== "running") {
      // Started from the click that asked for playback, so this normally
      // succeeds; the element plays natively until it does.
      if (c.resume) { c.resume().then(function () { attach(el); }, function () { /* stays native */ }); }
      return;
    }
    var source;
    try { source = c.createMediaElementSource(el); }
    catch (e) { return; }   // already captured by someone else: leave it be
    routed.set(el, true);
    try {
      var chain = buildChain(c, source, c.destination, current);
      chains.push(chain);
      routed.set(el, chain);
    } catch (e) {
      try { source.disconnect(); source.connect(c.destination); } catch (e2) { /* nothing more to do */ }
    }
  }

  // Wrapped once. `play()` is the one call every player makes, and it is made
  // inside the user's gesture, which is what lets the context start.
  var originalPlay = window.HTMLMediaElement && HTMLMediaElement.prototype.play;
  if (originalPlay && !originalPlay.__bnbListenFx) {
    var wrapped = function () {
      try {
        if (this instanceof HTMLAudioElement) {
          if (seen.indexOf(this) < 0) seen.push(this);
          attach(this);
        }
      } catch (e) { /* never in the way of playback */ }
      return originalPlay.apply(this, arguments);
    };
    wrapped.__bnbListenFx = true;
    HTMLMediaElement.prototype.play = wrapped;
  }

  // Autoplay and keyboard activation of native controls start playback without
  // a script calling play(); the `play` event covers them.
  document.addEventListener("play", function (e) {
    var el = e.target;
    if (el instanceof HTMLAudioElement) {
      if (seen.indexOf(el) < 0) seen.push(el);
      attach(el);
    }
  }, true);

  // The preference follows the other tabs.
  window.addEventListener("storage", function (e) {
    if (e.key === KEY || e.key === null) set(load(), false);
  });

  current = load();

  // ---- the controls ---------------------------------------------------------

  function round2(hz) {
    // Two significant figures: "1.2 kHz", "230 Hz", never "1187 Hz".
    var mag = Math.pow(10, Math.floor(Math.log(hz) / Math.LN10) - 1);
    return Math.round(hz / mag) * mag;
  }
  function hzText(hz) {
    if (hz <= HP_OFF) return "Off";
    var r = round2(hz);
    return r >= 1000 ? (r / 1000).toFixed(r % 1000 ? 1 : 0) + " kHz" : r + " Hz";
  }
  function hzSpoken(hz) {
    if (hz <= HP_OFF) return "off";
    var r = round2(hz);
    return r >= 1000 ? (r / 1000).toFixed(r % 1000 ? 1 : 0) + " kilohertz" : r + " hertz";
  }
  function dbText(db) { return db === 0 ? "Off" : "+" + db + " dB"; }
  function dbSpoken(db) { return db === 0 ? "off" : "plus " + db + " decibels"; }

  // The high-pass slider is logarithmic: a position 0..100 -> 20..HP_MAX Hz.
  function posToHz(p) { return Math.round(HP_OFF * Math.pow(HP_MAX / HP_OFF, p / 100)); }
  function hzToPos(hz) { return Math.round(100 * Math.log(Math.max(hz, HP_OFF) / HP_OFF) / Math.log(HP_MAX / HP_OFF)); }
  var mounted = 0;

  // The controls' stylesheet is fetched the first time they are built, from the
  // same prefix and version as this script, so a page without them pays nothing.
  // The block stays hidden until it has loaded: unstyled range inputs for one
  // frame are the flash this avoids.
  var cssReady = null;
  function styles() {
    if (cssReady) return cssReady;
    cssReady = new Promise(function (resolve) {
      var me = document.currentScript || document.querySelector('script[src*="listen-fx.js"]');
      var href = me && me.src ? me.src.replace(/\/static\/listen-fx\.js/, "/static/css/listen-fx.css") : "/static/css/listen-fx.css";
      var link = document.createElement("link");
      link.rel = "stylesheet";
      link.href = href;
      link.addEventListener("load", resolve);
      link.addEventListener("error", resolve);   // unstyled beats absent
      document.head.appendChild(link);
    });
    return cssReady;
  }

  function mount(root) {
    if (root.__lfx) return;
    root.__lfx = true;
    var id = "lfx" + (++mounted);
    var supported = !!contextClass() && !!routed;
    root.classList.add("lfx");
    root.hidden = true;
    styles().then(function () { root.hidden = false; });
    root.setAttribute("role", "group");
    root.setAttribute("aria-label", "Listening aids");
    root.innerHTML =
      '<div class="lfx-row">' +
        '<label for="' + id + '-gain">Boost</label>' +
        '<input type="range" id="' + id + '-gain" min="0" max="' + GAIN_MAX + '" step="1">' +
        '<output for="' + id + '-gain" class="lfx-out"></output>' +
      '</div>' +
      '<div class="lfx-row">' +
        '<label for="' + id + '-hp">Cut low rumble</label>' +
        '<input type="range" id="' + id + '-hp" min="0" max="100" step="1">' +
        '<output for="' + id + '-hp" class="lfx-out"></output>' +
      '</div>' +
      '<div class="lfx-row lfx-check">' +
        '<input type="checkbox" id="' + id + '-comp">' +
        '<label for="' + id + '-comp">Even out loud and quiet parts</label>' +
        '<button type="button" class="bnb-btn ghost lfx-reset">Reset</button>' +
      '</div>' +
      '<p class="bnb-meta lfx-note" id="' + id + '-note"></p>';
    var gain = root.querySelector("#" + id + "-gain");
    var hp = root.querySelector("#" + id + "-hp");
    var comp = root.querySelector("#" + id + "-comp");
    var reset = root.querySelector(".lfx-reset");
    var outs = root.querySelectorAll(".lfx-out");
    var note = root.querySelector(".lfx-note");

    function paint(s) {
      gain.value = String(s.gain);
      // The slider keeps the position the reader gave it; it is moved only when
      // the value came from elsewhere (another tab, Reset, storage). Snapping it
      // to the nearest position the stored Hz maps back to would, with 1 Hz of
      // resolution at the bottom, still be harmless, but a position is the one
      // thing the reader is holding.
      if (posToHz(Number(hp.value)) !== s.hp) hp.value = String(hzToPos(s.hp));
      comp.checked = s.comp;
      outs[0].textContent = dbText(s.gain);
      outs[1].textContent = hzText(s.hp);
      gain.setAttribute("aria-valuetext", dbSpoken(s.gain));
      hp.setAttribute("aria-valuetext", hzSpoken(s.hp));
      note.textContent = !supported
        ? "This browser cannot process audio, so these have no effect."
        : isNeutral(s)
          ? "Off. Changes what you hear in this browser only; recordings and downloads are never altered."
          : "On for every clip played in this browser. Recordings and downloads are never altered.";
    }
    function read() {
      return { gain: Number(gain.value), hp: posToHz(Number(hp.value)), comp: comp.checked };
    }
    [gain, hp, comp].forEach(function (el) {
      el.disabled = !supported;
      el.addEventListener("input", function () { set(read()); });
    });
    reset.disabled = !supported;
    reset.addEventListener("click", function () { set(DEFAULTS); });
    listeners.push(paint);
    paint(current);
  }

  function mountAll() {
    var roots = document.querySelectorAll("[data-listen-fx]");
    for (var i = 0; i < roots.length; i++) mount(roots[i]);
  }
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", mountAll);
  else mountAll();
  // Pages that swap their content in later (htmx) bring controls with them.
  document.addEventListener("htmx:afterSettle", mountAll);

  window.bnbListenFx = {
    KEY: KEY, CEILING: CEILING, GAIN_MAX: GAIN_MAX, HP_MAX: HP_MAX, DEFAULTS: DEFAULTS,
    clean: clean, isNeutral: isNeutral, buildChain: buildChain, limiterCurve: limiterCurve,
    get: function () { return current; }, set: set, mount: mount, attach: attach,
    // Whether `el` was routed through the chain, and the chain if it was built
    // (the tests tap its output; nothing else needs either).
    isRouted: function (el) { return !!routed && routed.has(el); },
    chainFor: function (el) { var c = routed && routed.get(el); return c && c !== true ? c : null; },
    context: function () { return ctx; }
  };
})();
