//! Custom audio player page with spectrogram visualization.
//!
//! Renders a detection audio clip with an interactive spectrogram display,
//! waveform overlay, and playback controls. The spectrogram is fetched from
//! the existing `/api/v2/spectrogram/{filename}` endpoint and displayed
//! alongside the audio player.
//!
//! | Path | Purpose |
//! |------|---------|
//! | GET `/player/{filename}` | Custom audio player with spectrogram |

use axum::extract::{Path, Query, State};
use axum::response::Html;
use axum::{Router, routing::get};

use super::escape_html;
use crate::state::AppState;

/// Query parameters for the player page.
#[derive(serde::Deserialize)]
struct PlayerQuery {
    species: Option<String>,
    confidence: Option<u32>,
    time: Option<String>,
    date: Option<String>,
}

pub fn router() -> Router<AppState> {
    Router::new().route("/player/{filename}", get(player_page))
}

async fn player_page(
    State(state): State<AppState>,
    Path(filename): Path<String>,
    Query(query): Query<PlayerQuery>,
) -> Html<String> {
    let species = query.species.as_deref().unwrap_or("Unknown Species");
    let confidence = query.confidence.unwrap_or(0);
    let time = query.time.as_deref().unwrap_or("");
    let date = query.date.as_deref().unwrap_or("");
    let site_name = state.site_name();

    let species_safe = escape_html(species);
    let filename_safe = escape_html(&filename);
    let time_safe = escape_html(time);
    let date_safe = escape_html(date);

    let content = format!(
        r#"<div class="player-container">
  <div class="player-header">
    <h2>{species_safe}</h2>
    <div class="player-meta">
      <span class="confidence-badge">{confidence}%</span>
      <span class="detection-time">{date_safe} {time_safe}</span>
    </div>
  </div>

  <div class="spectrogram-display">
    <img id="spectrogram-img" src="/api/v2/spectrogram/{filename_safe}?species={species_safe}&amp;confidence={confidence}&amp;time={time_safe}"
         alt="Spectrogram" loading="lazy">
    <canvas id="playhead-canvas"></canvas>
  </div>

  <div class="audio-controls">
    <audio id="detection-audio" preload="auto">
      <source src="/api/v2/recordings/{filename_safe}" type="audio/wav">
      <source src="/api/v2/recordings/{filename_safe}" type="audio/mpeg">
    </audio>

    <div class="transport">
      <button id="play-btn" onclick="togglePlayback()" aria-label="Play/Pause">
        <svg id="play-icon" viewBox="0 0 24 24" width="28" height="28">
          <polygon points="5,3 19,12 5,21" fill="currentColor"/>
        </svg>
        <svg id="pause-icon" viewBox="0 0 24 24" width="28" height="28" class="ap-hidden">
          <rect x="5" y="3" width="4" height="18" fill="currentColor"/>
          <rect x="15" y="3" width="4" height="18" fill="currentColor"/>
        </svg>
      </button>

      <div class="progress-bar" id="progress-bar" onclick="seek(event)">
        <div class="progress-fill" id="progress-fill"></div>
      </div>

      <span id="time-display" class="time-display">0:00 / 0:00</span>
    </div>

    <div class="volume-row">
      <label for="vol-slider">Vol</label>
      <input type="range" id="vol-slider" min="0" max="100" value="80"
             oninput="setVol(this.value)">
      <span id="vol-pct">80%</span>

      <label for="speed-select" class="ap-speed-label">Speed</label>
      <select id="speed-select" onchange="setSpeed(this.value)">
        <option value="0.5">0.5x</option>
        <option value="0.75">0.75x</option>
        <option value="1" selected>1x</option>
        <option value="1.5">1.5x</option>
        <option value="2">2x</option>
      </select>

      <button class="btn-small ap-download" onclick="downloadClip()">
        Download
      </button>
    </div>
  </div>
</div>"#
    );

    let html = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>{species_safe} — {site_name}</title>
  <link rel="stylesheet" href="/static/css/app.css">
  <style>{PLAYER_CSS}</style>
</head>
<body>
<div class="container ap-page">
  <nav class="ap-nav">
    <a href="/" class="ap-back">&larr; Dashboard</a>
  </nav>
  {content}
</div>
<script>{PLAYER_JS}</script>
</body>
</html>"#,
        site_name = escape_html(site_name),
    );
    Html(html)
}

const PLAYER_CSS: &str = r#"
body { background:var(--bg); color:var(--fg); font-family:system-ui,sans-serif; }
.player-container { background:var(--surface); border:1px solid var(--border); border-radius:12px; overflow:hidden; }
.player-header { padding:1.25rem 1.5rem 0.75rem; }
.player-header h2 { margin:0 0 0.5rem; font-size:1.3rem; color:var(--fg); }
.player-meta { display:flex; align-items:center; gap:0.75rem; font-size:0.85rem; color:var(--fg-3); }
.confidence-badge { background:var(--moss); color:#fff; padding:2px 8px; border-radius:4px; font-weight:600; font-size:0.8rem; }
.spectrogram-display { position:relative; background:#000; }
.spectrogram-display img { width:100%; display:block; image-rendering:pixelated; min-height:128px; }
#playhead-canvas { position:absolute; top:0; left:0; width:100%; height:100%; pointer-events:none; }
.audio-controls { padding:1rem 1.5rem 1.25rem; }
.transport { display:flex; align-items:center; gap:0.75rem; }
#play-btn { background:none; border:none; color:var(--fg); cursor:pointer; padding:4px; }
#play-btn:hover { color:var(--moss-ink); }
.progress-bar { flex:1; height:6px; background:var(--border); border-radius:3px; cursor:pointer; position:relative; }
.progress-fill { height:100%; background:var(--moss); border-radius:3px; width:0; transition:width 0.1s linear; }
.time-display { font-size:0.8rem; color:var(--fg-3); min-width:80px; text-align:right; font-variant-numeric:tabular-nums; }
.volume-row { display:flex; align-items:center; gap:0.5rem; margin-top:0.75rem; font-size:0.8rem; color:var(--fg-3); }
.volume-row input[type="range"] { width:80px; accent-color:var(--moss); }
.volume-row select { background:var(--bg); border:1px solid var(--border); color:var(--fg); border-radius:4px; padding:2px 4px; font-size:0.8rem; }
.btn-small { background:var(--border); border:none; color:var(--fg); padding:4px 12px; border-radius:4px; cursor:pointer; font-size:0.8rem; }
.btn-small:hover { background:var(--border-2); }
"#;

const PLAYER_JS: &str = r"
const audio = document.getElementById('detection-audio');
const playBtn = document.getElementById('play-btn');
const playIcon = document.getElementById('play-icon');
const pauseIcon = document.getElementById('pause-icon');
const progressFill = document.getElementById('progress-fill');
const timeDisplay = document.getElementById('time-display');
const canvas = document.getElementById('playhead-canvas');
const ctx = canvas.getContext('2d');

audio.volume = 0.8;
let animFrame;

function fmt(s) {
  const m = Math.floor(s / 60);
  const sec = Math.floor(s % 60);
  return m + ':' + (sec < 10 ? '0' : '') + sec;
}

function togglePlayback() {
  if (audio.paused) {
    audio.play();
    playIcon.classList.add('ap-hidden');
    pauseIcon.classList.remove('ap-hidden');
    animate();
  } else {
    audio.pause();
    playIcon.classList.remove('ap-hidden');
    pauseIcon.classList.add('ap-hidden');
    cancelAnimationFrame(animFrame);
  }
}

function animate() {
  if (!audio.paused) {
    updateProgress();
    drawPlayhead();
    animFrame = requestAnimationFrame(animate);
  }
}

function updateProgress() {
  const pct = audio.duration ? (audio.currentTime / audio.duration * 100) : 0;
  progressFill.style.width = pct + '%';
  timeDisplay.textContent = fmt(audio.currentTime) + ' / ' + fmt(audio.duration || 0);
}

function drawPlayhead() {
  const img = document.getElementById('spectrogram-img');
  canvas.width = img.clientWidth;
  canvas.height = img.clientHeight;
  ctx.clearRect(0, 0, canvas.width, canvas.height);
  if (audio.duration) {
    const x = (audio.currentTime / audio.duration) * canvas.width;
    ctx.strokeStyle = 'rgba(255,255,255,0.8)';
    ctx.lineWidth = 2;
    ctx.beginPath();
    ctx.moveTo(x, 0);
    ctx.lineTo(x, canvas.height);
    ctx.stroke();
  }
}

function seek(e) {
  const bar = document.getElementById('progress-bar');
  const pct = (e.clientX - bar.getBoundingClientRect().left) / bar.clientWidth;
  if (audio.duration) audio.currentTime = pct * audio.duration;
  updateProgress();
  drawPlayhead();
}

function setVol(v) {
  audio.volume = v / 100;
  document.getElementById('vol-pct').textContent = v + '%';
}

function setSpeed(v) { audio.playbackRate = parseFloat(v); }

function downloadClip() {
  const src = audio.querySelector('source').src;
  const a = document.createElement('a');
  a.href = src;
  a.download = '';
  a.click();
}

audio.addEventListener('ended', function() {
  playIcon.classList.remove('ap-hidden');
  pauseIcon.classList.add('ap-hidden');
  cancelAnimationFrame(animFrame);
  updateProgress();
});

audio.addEventListener('loadedmetadata', updateProgress);
";

#[cfg(test)]
mod tests {
    #[test]
    fn player_css_not_empty() {
        assert!(!super::PLAYER_CSS.is_empty());
    }

    #[test]
    fn player_js_not_empty() {
        assert!(!super::PLAYER_JS.is_empty());
        assert!(super::PLAYER_JS.contains("togglePlayback"));
    }
}
