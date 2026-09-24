//! Audio capture settings section.

use std::collections::HashMap;
use std::fmt::Write as _;

use super::get_setting;

pub(super) fn render(out: &mut String, s: &HashMap<String, String>) {
    let alsa = get_setting(s, "alsa_device");
    let rtsp = get_setting(s, "rtsp_url");
    let rtsp_urls = get_setting(s, "rtsp_urls");
    let seg = get_setting(s, "segment_duration");
    let fmt = get_setting(s, "audio_format");
    let fmt_wav = if fmt == "wav" { " selected" } else { "" };
    let fmt_mp3 = if fmt == "mp3" { " selected" } else { "" };
    let fmt_flac = if fmt == "flac" { " selected" } else { "" };
    let fmt_ogg = if fmt == "ogg" { " selected" } else { "" };
    let freq_shift = get_setting(s, "freq_shift_hz");
    let lufs = get_setting(s, "clip_target_lufs");
    let lufs_opt = |v: &str| if lufs == v { " selected" } else { "" };
    let (lufs_off, lufs_14, lufs_18, lufs_23) = (
        lufs_opt(""),
        lufs_opt("-14"),
        lufs_opt("-18"),
        lufs_opt("-23"),
    );
    write!(out, r#"
  <section class="card" id="set-audio" aria-labelledby="set-audio-h">
    <h2 class="section-title" id="set-audio-h">Audio Capture</h2>
    <p class="hint flush"><a href="/recordings?view=live">▸ Listen live &amp; test your microphone →</a> — confirm the mic is picking up sound before tuning thresholds.</p>
    <div class="grid-2">
      <div>
        <label for="alsa_device">Microphone address</label>
        <input id="alsa_device" name="alsa_device" value="{alsa}" placeholder="e.g. plughw:1,0">
        <p class="hint">Most stations set microphones up on
        <a href="/admin/audio">Audio &amp; Microphones</a> instead and leave this
        blank. Blank means this station manages no microphone of its own.</p>
      </div>
      <div>
        <label for="rtsp_url">Network camera address</label>
        <input id="rtsp_url" name="rtsp_url" value="{rtsp}" placeholder="rtsp://camera.local:554/stream">
        <p class="hint">The audio stream from a network camera, which its own
        app or manual will call an RTSP address. Needs the <code>ffmpeg</code>
        program installed on the station.</p>
      </div>
    </div>
    <div>
      <label for="rtsp_urls">Several network cameras (separated by commas)</label>
      <input id="rtsp_urls" name="rtsp_urls" value="{rtsp_urls}" placeholder="rtsp://cam1:554/stream,rtsp://cam2:554/stream">
      <p class="hint">Each URL becomes an independent capture pipeline (RTSP_1-, RTSP_2- prefixed filenames). Overrides single RTSP URL above when set.</p>
    </div>
    <div class="grid-2">
      <div>
        <label for="segment_duration">Segment Duration (seconds)</label>
        <input id="segment_duration" name="segment_duration" type="number" value="{seg}" min="5" max="60" class="bnb-w-num">
        <p class="hint">Length of each recording chunk for analysis (BirdNET-Pi: RECORDING_LENGTH)</p>
      </div>
      <div>
        <label>Audio Channels</label>
        <p class="hint">Every source is recorded in mono; there is no control for this.
        On <a href="/admin/audio">Audio &amp; Microphones</a> you set a source\'s
        recording quality when you add it, and its noise filtering and quiet hours
        at any time.</p>
      </div>
    </div>
    <div class="grid-2">
      <div>
        <label for="audio_format">Extracted Clip Format</label>
        <select id="audio_format" name="audio_format" class="bnb-w-select">
          <option value="wav"{fmt_wav}>WAV (lossless, default)</option>
          <option value="mp3"{fmt_mp3}>MP3 (requires ffmpeg)</option>
          <option value="flac"{fmt_flac}>FLAC (lossless compressed, requires ffmpeg)</option>
          <option value="ogg"{fmt_ogg}>OGG (requires ffmpeg)</option>
        </select>
        <p class="hint">Format for saved detection audio clips (BirdNET-Pi: AUDIOFMT)</p>
      </div>
      <div>
        <label for="clip_target_lufs">Even out clip volume</label>
        <select id="clip_target_lufs" name="clip_target_lufs" class="bnb-w-select">
          <option value=""{lufs_off}>Off — save clips exactly as recorded</option>
          <option value="-14"{lufs_14}>Loud — like a podcast (−14 LUFS)</option>
          <option value="-18"{lufs_18}>Even — recommended (−18 LUFS)</option>
          <option value="-23"{lufs_23}>Quiet — broadcast standard (−23 LUFS)</option>
        </select>
        <p class="hint">Plays every saved clip back at about the same volume, so
        going through a morning’s recordings does not send you to the volume
        control between each one. This changes <b>only the saved clip</b>, never
        the sound the identifier listens to, so it cannot change how sure the
        station was about a bird. A very quiet clip is turned up only as far as
        it can go without distorting.</p>
        <p class="hint">The numbers are the broadcast loudness scale (LUFS,
        measured to ITU‑R BS.1770); −18 suits most gardens.</p>
      </div>
      <div>
        <label for="freq_shift_hz">Frequency Shift (Hz, 0 = disabled)</label>
        <input id="freq_shift_hz" name="freq_shift_hz" type="number" value="{freq_shift}"
               min="-12000" max="12000" step="500" class="bnb-w-num">
        <p class="hint">Shift pitch of saved clips for accessibility (BirdNET-Pi: FREQ_SHIFT). Requires ffmpeg or sox. Typical: 1000–4000.</p>
      </div>
    </div>
  </section>"#).unwrap_or_default();
}
