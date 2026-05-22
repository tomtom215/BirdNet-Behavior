//! First-run onboarding wizard.
//!
//! A full-bleed, no-chrome five-step setup flow (Welcome → Location →
//! Microphone → Notifications → Done) served at `/onboarding`. The steps are
//! fully styled and client-navigable; persistence and device detection are
//! intentionally out of scope here (a clearly-scoped stub) — a production
//! build would POST each step to the settings/audio endpoints.

use axum::Router;
use axum::response::Html;
use axum::routing::get;

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/onboarding", get(onboarding_page))
}

async fn onboarding_page() -> Html<String> {
    Html(ONBOARDING_HTML.to_string())
}

const ONBOARDING_HTML: &str = r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>BirdNet-Behavior · Set up your station</title>
<link rel="stylesheet" href="/static/css/app.css">
<script src="/static/theme-guard.js"></script>
<style>
  body { margin:0; background:var(--bg); color:var(--fg); min-height:100vh; }
  .ob-root { max-width:980px; margin:0 auto; min-height:100vh; display:flex; flex-direction:column; padding:0 24px; }
  .ob-stepper { display:flex; align-items:center; gap:8px; padding:22px 0 8px; position:sticky; top:0; background:color-mix(in oklch, var(--bg) 92%, transparent); backdrop-filter:saturate(1.4) blur(10px); z-index:5; }
  .ob-pip { display:flex; align-items:center; gap:8px; }
  .ob-pip .dot { width:22px; height:22px; border-radius:999px; border:0.5px solid var(--border-2); display:flex; align-items:center; justify-content:center; font-size:11px; font-family:var(--font-mono); color:var(--fg-3); background:var(--surface); }
  .ob-pip .nm { font-size:11px; letter-spacing:0.06em; text-transform:uppercase; color:var(--fg-3); }
  .ob-pip .bar { width:28px; height:1.5px; background:var(--hairline); }
  .ob-pip.done .dot, .ob-pip.active .dot { background:var(--moss); color:var(--bg); border-color:transparent; }
  .ob-pip.active .nm { color:var(--fg); font-weight:500; }
  .ob-stage { flex:1; display:flex; align-items:center; padding:24px 0; }
  .ob-step { display:none; width:100%; animation:ob-fade .2s ease; }
  .ob-step.active { display:block; }
  @keyframes ob-fade { from { opacity:0; transform:translateY(6px); } to { opacity:1; transform:none; } }
  .ob-two { display:grid; grid-template-columns:1fr 1fr; gap:32px; align-items:center; }
  .ob-eyebrow { font-size:10.5px; letter-spacing:0.10em; text-transform:uppercase; color:var(--fg-3); font-weight:500; }
  .ob-h { font-family:var(--font-display); font-size:46px; line-height:1.08; letter-spacing:-0.02em; margin:8px 0 12px; }
  .ob-h em { font-style:italic; color:var(--moss-ink); }
  .ob-p { color:var(--fg-2); font-size:15px; max-width:42ch; }
  .ob-bullets { list-style:none; padding:0; margin:18px 0 0; display:flex; flex-direction:column; gap:10px; }
  .ob-bullets li { display:flex; gap:10px; align-items:center; font-size:14px; }
  .ob-bullets .tick { width:18px; height:18px; border-radius:999px; background:var(--moss-soft); color:var(--moss-ink); display:inline-flex; align-items:center; justify-content:center; font-size:11px; }
  .ob-nav { display:flex; align-items:center; justify-content:space-between; gap:16px; padding:18px 0 28px; border-top:0.5px solid var(--hairline); }
  .ob-field { display:flex; flex-direction:column; gap:6px; margin-bottom:14px; }
  .ob-field label { font-size:12.5px; font-weight:500; }
  .ob-field input { padding:9px 12px; border-radius:var(--r-sm); border:0.5px solid var(--border-2); background:var(--surface); color:var(--fg); font:inherit; }
  .ob-cards { display:grid; gap:12px; }
  .ob-card { display:flex; gap:14px; align-items:center; padding:14px; border-radius:var(--r-md); border:0.5px solid var(--border); background:var(--surface); cursor:pointer; transition:border-color .12s, background .12s; }
  .ob-card.sel { border-color:var(--moss); background:var(--moss-soft); }
  .ob-card .ic { width:34px; height:34px; flex-shrink:0; border-radius:8px; background:var(--surface-2); display:flex; align-items:center; justify-content:center; color:var(--fg-2); }
  .ob-card .t { font-weight:500; font-size:14px; }
  .ob-card .s { font-size:12px; color:var(--fg-3); }
  .vu { display:flex; align-items:flex-end; gap:2px; height:26px; margin-left:auto; }
  .vu i { width:3px; background:var(--moss); border-radius:1px; animation:vu 1.1s ease-in-out infinite; }
  @keyframes vu { 0%,100% { height:20%; } 50% { height:95%; } }
  .chips { display:flex; flex-wrap:wrap; gap:8px; margin-top:12px; }
  .summary-row { display:flex; justify-content:space-between; gap:16px; padding:11px 0; border-top:0.5px solid var(--hairline); font-size:14px; }
  .summary-row:first-child { border-top:0; }
  .summary-row .k { color:var(--fg-3); }
  .calib { display:flex; align-items:flex-end; gap:2px; height:46px; }
  .calib i { flex:1; background:var(--moss); opacity:.5; border-radius:1px; animation:vu 1.6s ease-in-out infinite; }
  @media (prefers-reduced-motion: reduce) {
    .ob-step, .vu i, .calib i, .sonar * { animation:none !important; }
  }
</style>
</head>
<body>
<div class="ob-root">
  <div class="ob-stepper" id="ob-stepper">
    <div class="ob-pip" data-pip="1"><span class="dot">1</span><span class="nm">Welcome</span></div>
    <span class="bar"></span>
    <div class="ob-pip" data-pip="2"><span class="dot">2</span><span class="nm">Location</span></div>
    <span class="bar"></span>
    <div class="ob-pip" data-pip="3"><span class="dot">3</span><span class="nm">Microphone</span></div>
    <span class="bar"></span>
    <div class="ob-pip" data-pip="4"><span class="dot">4</span><span class="nm">Alerts</span></div>
    <span class="bar"></span>
    <div class="ob-pip" data-pip="5"><span class="dot">5</span><span class="nm">Done</span></div>
  </div>

  <div class="ob-stage">
    <!-- Step 1 — Welcome -->
    <section class="ob-step active" data-step="1">
      <div class="ob-two">
        <div>
          <div class="ob-eyebrow">Welcome</div>
          <h1 class="ob-h">Let's teach the yard<br>to <em>listen</em>.</h1>
          <p class="ob-p">Ninety seconds, five steps. Your Raspberry Pi will start identifying every bird it hears — no accounts, no cloud, all yours.</p>
          <ul class="ob-bullets">
            <li><span class="tick">✓</span> No accounts — runs entirely on your Pi</li>
            <li><span class="tick">✓</span> Set once — sensible defaults the whole way</li>
            <li><span class="tick">✓</span> Always tweakable — change anything later in Settings</li>
          </ul>
        </div>
        <div style="display:flex;align-items:center;justify-content:center;">
          <svg class="sonar" width="240" height="240" viewBox="0 0 240 240" aria-hidden="true">
            <g fill="none" stroke="var(--moss)" stroke-width="1">
              <circle cx="120" cy="120" r="30"><animate attributeName="r" values="30;110" dur="5s" repeatCount="indefinite"/><animate attributeName="stroke-opacity" values="0.7;0" dur="5s" repeatCount="indefinite"/></circle>
              <circle cx="120" cy="120" r="30"><animate attributeName="r" values="30;110" dur="5s" begin="1.6s" repeatCount="indefinite"/><animate attributeName="stroke-opacity" values="0.7;0" dur="5s" begin="1.6s" repeatCount="indefinite"/></circle>
              <circle cx="120" cy="120" r="30"><animate attributeName="r" values="30;110" dur="5s" begin="3.2s" repeatCount="indefinite"/><animate attributeName="stroke-opacity" values="0.7;0" dur="5s" begin="3.2s" repeatCount="indefinite"/></circle>
            </g>
            <rect x="96" y="96" width="48" height="48" rx="8" fill="var(--surface)" stroke="var(--border-2)" stroke-width="0.5"/>
            <g stroke="var(--moss)" stroke-width="2" stroke-linecap="round">
              <line x1="110" y1="120" x2="110" y2="120"/><line x1="116" y1="112" x2="116" y2="128"/>
              <line x1="122" y1="106" x2="122" y2="134"/><line x1="128" y1="113" x2="128" y2="127"/>
            </g>
          </svg>
        </div>
      </div>
    </section>

    <!-- Step 2 — Location -->
    <section class="ob-step" data-step="2">
      <div class="ob-two">
        <div>
          <div class="ob-eyebrow">Where</div>
          <h1 class="ob-h">Where is the station?</h1>
          <p class="ob-p">Your coordinates let BirdNET weight species by what's actually likely in your area, and compute sunrise / sunset for the dawn-chorus window.</p>
          <div style="margin-top:18px;">
            <button class="bnb-btn" type="button">⌖ Auto-detect (ipapi.co)</button>
          </div>
          <div style="display:grid;grid-template-columns:1fr 1fr;gap:12px;margin-top:16px;">
            <div class="ob-field"><label>Latitude</label><input type="text" value="42.3601" inputmode="decimal"></div>
            <div class="ob-field"><label>Longitude</label><input type="text" value="-71.0589" inputmode="decimal"></div>
          </div>
          <div class="bnb-pill moss" style="margin-top:6px;">✓ Boston, MA · 247 species expected · sunrise 5:21 AM</div>
        </div>
        <div style="display:flex;align-items:center;justify-content:center;">
          <svg width="280" height="220" viewBox="0 0 280 220" aria-hidden="true" style="background:var(--surface-2);border-radius:var(--r-lg);border:0.5px solid var(--border);">
            <g fill="none" stroke="var(--border-2)" stroke-width="0.75" opacity="0.7">
              <ellipse cx="140" cy="110" rx="40" ry="28"/><ellipse cx="140" cy="110" rx="70" ry="50"/>
              <ellipse cx="140" cy="110" rx="100" ry="72"/><ellipse cx="140" cy="110" rx="128" ry="94"/>
            </g>
            <circle cx="140" cy="110" r="86" fill="none" stroke="var(--moss)" stroke-width="1" stroke-dasharray="4 5"/>
            <circle cx="140" cy="110" r="9" fill="none" stroke="var(--moss)" stroke-width="1.5"/>
            <circle cx="140" cy="110" r="3" fill="var(--moss)"/>
            <text x="140" y="206" text-anchor="middle" font-size="9" class="mono" fill="var(--fg-4)">~100 km radius</text>
          </svg>
        </div>
      </div>
    </section>

    <!-- Step 3 — Microphone -->
    <section class="ob-step" data-step="3">
      <div class="ob-eyebrow">How it hears</div>
      <h1 class="ob-h">Pick a microphone.</h1>
      <p class="ob-p" style="margin-bottom:18px;">We found a USB mic already. You can also add a network (RTSP) camera or watch a folder of recordings.</p>
      <div class="ob-cards" id="mic-cards">
        <div class="ob-card sel" data-radio="mic"><span class="ic">🎤</span><div style="flex:1;"><div class="t">UMC202HD · USB audio <span class="bnb-pill moss" style="margin-left:6px;">recommended</span></div><div class="s">card 1 · 48 kHz · detected automatically</div></div><span class="vu"><i style="animation-delay:0s"></i><i style="animation-delay:.1s"></i><i style="animation-delay:.2s"></i><i style="animation-delay:.05s"></i><i style="animation-delay:.25s"></i><i style="animation-delay:.15s"></i><i style="animation-delay:.3s"></i><i style="animation-delay:.08s"></i></span></div>
        <div class="ob-card" data-radio="mic"><span class="ic">🎤</span><div style="flex:1;"><div class="t">Built-in microphone</div><div class="s">card 0 · 44.1 kHz</div></div></div>
        <div class="ob-card" data-radio="mic"><span class="ic">📡</span><div style="flex:1;"><div class="t">Add an RTSP camera</div><div class="s">rtsp://… — bird-box or feeder cam audio</div></div></div>
        <div class="ob-card" data-radio="mic"><span class="ic">📁</span><div style="flex:1;"><div class="t">Watch a folder</div><div class="s">classify existing recordings on disk</div></div></div>
      </div>
    </section>

    <!-- Step 4 — Notifications -->
    <section class="ob-step" data-step="4">
      <div class="ob-eyebrow">Who gets told</div>
      <h1 class="ob-h">When should we ping you?</h1>
      <p class="ob-p" style="margin-bottom:18px;">Start simple — you can wire up channels (Telegram, email, MQTT…) any time.</p>
      <div class="ob-cards" style="grid-template-columns:repeat(2,1fr);">
        <div class="ob-card" data-radio="notify"><div style="flex:1;"><div class="t">Quiet</div><div class="s">Never notify — just log everything</div></div></div>
        <div class="ob-card sel" data-radio="notify"><div style="flex:1;"><div class="t">Rare only <span class="bnb-pill moss" style="margin-left:6px;">recommended</span></div><div class="s">Only first-of-station / unusual birds</div></div></div>
        <div class="ob-card" data-radio="notify"><div style="flex:1;"><div class="t">Daily digest</div><div class="s">One summary each evening</div></div></div>
        <div class="ob-card" data-radio="notify"><div style="flex:1;"><div class="t">Everything</div><div class="s">Every detection (chatty!)</div></div></div>
      </div>
      <details style="margin-top:16px;">
        <summary class="bnb-meta" style="cursor:pointer;">Pick channels now <span class="bnb-pill">optional</span></summary>
        <div class="chips">
          <span class="bnb-pill">Telegram</span><span class="bnb-pill">Email</span><span class="bnb-pill">MQTT</span>
          <span class="bnb-pill">Webhook</span><span class="bnb-pill">Slack</span><span class="bnb-pill">Discord</span>
          <span class="bnb-pill">Pushover</span><span class="bnb-pill">ntfy</span><span class="bnb-pill">Apprise</span>
          <span class="bnb-pill">BirdWeather</span><span class="bnb-pill">Home Assistant</span><span class="bnb-pill">SMS</span>
        </div>
      </details>
    </section>

    <!-- Step 5 — Done -->
    <section class="ob-step" data-step="5">
      <div class="ob-two">
        <div>
          <div class="ob-eyebrow">All set</div>
          <h1 class="ob-h">You're <em>listening</em>.</h1>
          <p class="ob-p">The pipeline is warming up. Within a minute or two you'll see the first detections roll in.</p>
          <div class="bnb-card pad" style="margin-top:16px;">
            <div class="summary-row"><span class="k">Location</span><span>Boston, MA · 42.36, −71.06</span></div>
            <div class="summary-row"><span class="k">Microphone</span><span>UMC202HD · USB · 48 kHz</span></div>
            <div class="summary-row"><span class="k">Alerts</span><span>Rare birds only</span></div>
            <div class="summary-row"><span class="k">Dashboard</span><span class="mono">http://birdnet.local/</span></div>
          </div>
        </div>
        <div>
          <div class="bnb-card pad">
            <div class="ob-eyebrow">Warming up</div>
            <div class="calib" style="margin:14px 0;"><i style="animation-delay:0s"></i><i style="animation-delay:.1s"></i><i style="animation-delay:.2s"></i><i style="animation-delay:.3s"></i><i style="animation-delay:.15s"></i><i style="animation-delay:.25s"></i><i style="animation-delay:.05s"></i><i style="animation-delay:.35s"></i><i style="animation-delay:.12s"></i><i style="animation-delay:.22s"></i><i style="animation-delay:.32s"></i><i style="animation-delay:.18s"></i></div>
            <div class="bnb-meta">Calibrating noise floor… <span class="bnb-pill moss" style="margin-left:4px;">BirdNET+ V3.0</span></div>
          </div>
        </div>
      </div>
    </section>
  </div>

  <div class="ob-nav">
    <button class="bnb-btn ghost" id="ob-back" type="button" style="visibility:hidden;">← Back</button>
    <div class="bnb-meta">Step <span id="ob-cur">1</span> of 5</div>
    <a class="bnb-btn primary" id="ob-next" href="#" role="button">Continue →</a>
  </div>
</div>

<script>
(function () {
  var step = 1, total = 5;
  var stepsEls = document.querySelectorAll('.ob-step');
  var pips = document.querySelectorAll('.ob-pip');
  var back = document.getElementById('ob-back');
  var next = document.getElementById('ob-next');
  var cur = document.getElementById('ob-cur');

  function render() {
    stepsEls.forEach(function (s) { s.classList.toggle('active', +s.dataset.step === step); });
    pips.forEach(function (p) {
      var n = +p.dataset.pip;
      p.classList.toggle('active', n === step);
      p.classList.toggle('done', n < step);
    });
    cur.textContent = step;
    back.style.visibility = step === 1 ? 'hidden' : 'visible';
    if (step === total) { next.textContent = 'Go to dashboard →'; next.setAttribute('href', '/'); }
    else { next.textContent = 'Continue →'; next.setAttribute('href', '#'); }
  }
  back.addEventListener('click', function () { if (step > 1) { step--; render(); } });
  next.addEventListener('click', function (e) {
    if (step < total) { e.preventDefault(); step++; render(); }
    // on last step the anchor navigates to "/"
  });

  // Single-select radio cards.
  document.querySelectorAll('[data-radio]').forEach(function (card) {
    card.addEventListener('click', function () {
      document.querySelectorAll('[data-radio="' + card.dataset.radio + '"]').forEach(function (c) {
        c.classList.remove('sel');
      });
      card.classList.add('sel');
    });
  });
  render();
})();
</script>
</body>
</html>"##;
