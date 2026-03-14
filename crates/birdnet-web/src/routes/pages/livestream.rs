//! Livestream page: HTML page with an embedded audio player for live audio.

use axum::response::Html;
use axum::{Router, routing::get};

use crate::state::AppState;

/// Livestream page routes.
pub fn router() -> Router<AppState> {
    Router::new().route("/live", get(livestream_page))
}

/// Render the livestream page with an embedded audio player.
async fn livestream_page() -> Html<String> {
    super::render_page("Live Audio", LIVESTREAM_HTML, "live")
}

/// Embedded HTML template for the livestream page.
const LIVESTREAM_HTML: &str = r#"<div class="livestream-container">
    <h2>Live Audio Stream</h2>
    <p class="description">Listen to the live microphone feed from your BirdNet station.</p>

    <div class="audio-player">
        <audio id="live-audio" controls preload="none">
            <source src="/stream" type="audio/mpeg">
            Your browser does not support the audio element.
        </audio>
    </div>

    <div class="controls">
        <div class="volume-control">
            <label for="volume-slider">Volume:</label>
            <input type="range" id="volume-slider" min="0" max="100" value="80"
                   oninput="setVolume(this.value)">
            <span id="volume-display">80%</span>
        </div>

        <div class="stream-controls">
            <button id="play-btn" onclick="toggleStream()">Play</button>
            <span id="stream-status" class="status-indicator">Stopped</span>
        </div>
    </div>
</div>

<style>
.livestream-container {
    max-width: 600px;
    margin: 2rem auto;
    padding: 1.5rem;
    background: var(--card-bg, #1e1e2e);
    border-radius: 12px;
    border: 1px solid var(--border-color, #333);
}
.livestream-container h2 {
    margin-top: 0;
    color: var(--text-primary, #cdd6f4);
}
.description {
    color: var(--text-secondary, #a6adc8);
    margin-bottom: 1.5rem;
}
.audio-player {
    margin: 1rem 0;
}
.audio-player audio {
    width: 100%;
}
.controls {
    margin-top: 1.5rem;
}
.volume-control {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    margin-bottom: 1rem;
}
.volume-control label {
    color: var(--text-secondary, #a6adc8);
    font-size: 0.9rem;
    min-width: 55px;
}
.volume-control input[type="range"] {
    flex: 1;
    accent-color: var(--accent, #89b4fa);
}
#volume-display {
    color: var(--text-secondary, #a6adc8);
    font-size: 0.85rem;
    min-width: 35px;
    text-align: right;
}
.stream-controls {
    display: flex;
    align-items: center;
    gap: 1rem;
}
#play-btn {
    padding: 0.5rem 1.5rem;
    border: none;
    border-radius: 6px;
    background: var(--accent, #89b4fa);
    color: var(--card-bg, #1e1e2e);
    font-weight: 600;
    cursor: pointer;
    transition: opacity 0.15s;
}
#play-btn:hover {
    opacity: 0.85;
}
.status-indicator {
    font-size: 0.85rem;
    color: var(--text-secondary, #a6adc8);
}
.status-indicator.playing {
    color: var(--success, #a6e3a1);
}
</style>

<script>
const audio = document.getElementById('live-audio');
const playBtn = document.getElementById('play-btn');
const statusEl = document.getElementById('stream-status');
const volumeSlider = document.getElementById('volume-slider');
const volumeDisplay = document.getElementById('volume-display');

audio.volume = 0.8;

function setVolume(val) {
    audio.volume = val / 100;
    volumeDisplay.textContent = val + '%';
}

function toggleStream() {
    if (audio.paused) {
        audio.load();
        audio.play().then(function() {
            playBtn.textContent = 'Stop';
            statusEl.textContent = 'Playing';
            statusEl.className = 'status-indicator playing';
        }).catch(function(e) {
            statusEl.textContent = 'Error: ' + e.message;
        });
    } else {
        audio.pause();
        audio.removeAttribute('src');
        audio.load();
        var source = audio.querySelector('source');
        if (source) source.setAttribute('src', '/stream');
        playBtn.textContent = 'Play';
        statusEl.textContent = 'Stopped';
        statusEl.className = 'status-indicator';
    }
}

audio.addEventListener('error', function() {
    statusEl.textContent = 'Stream unavailable';
    statusEl.className = 'status-indicator';
    playBtn.textContent = 'Play';
});
</script>"#;
