//! Audio capture settings section.

use std::collections::HashMap;

use super::get_setting;

pub(super) fn render(out: &mut String, s: &HashMap<String, String>) {
    let alsa = get_setting(s, "alsa_device", "");
    let rtsp = get_setting(s, "rtsp_url", "");
    let rtsp_urls = get_setting(s, "rtsp_urls", "");
    let seg = get_setting(s, "segment_duration", "15");
    let channels = get_setting(s, "audio_channels", "1");
    let fmt = get_setting(s, "audio_format", "wav");
    let fmt_wav = if fmt == "wav" { " selected" } else { "" };
    let fmt_mp3 = if fmt == "mp3" { " selected" } else { "" };
    let fmt_flac = if fmt == "flac" { " selected" } else { "" };
    let fmt_ogg = if fmt == "ogg" { " selected" } else { "" };
    let freq_shift = get_setting(s, "freq_shift_hz", "0");
    out.push_str(&format!(r#"
  <div class="card">
    <div class="section-title">Audio Capture</div>
    <div class="grid-2">
      <div>
        <label for="alsa_device">ALSA Device</label>
        <input id="alsa_device" name="alsa_device" value="{alsa}" placeholder="e.g. plughw:1,0">
        <p class="hint">Leave blank to disable managed microphone capture. PulseAudio/PipeWire users: use "default" or leave blank and set ALSA_CARD env var.</p>
      </div>
      <div>
        <label for="rtsp_url">RTSP URL (single stream)</label>
        <input id="rtsp_url" name="rtsp_url" value="{rtsp}" placeholder="rtsp://camera.local:554/stream">
        <p class="hint">IP camera audio stream (requires ffmpeg)</p>
      </div>
    </div>
    <div>
      <label for="rtsp_urls">Multiple RTSP URLs (comma-separated)</label>
      <input id="rtsp_urls" name="rtsp_urls" value="{rtsp_urls}" placeholder="rtsp://cam1:554/stream,rtsp://cam2:554/stream">
      <p class="hint">Each URL becomes an independent capture pipeline (RTSP_1-, RTSP_2- prefixed filenames). Overrides single RTSP URL above when set.</p>
    </div>
    <div class="grid-2">
      <div>
        <label for="segment_duration">Segment Duration (seconds)</label>
        <input id="segment_duration" name="segment_duration" type="number" value="{seg}" min="5" max="60" style="max-width:120px">
        <p class="hint">Length of each recording chunk for analysis (BirdNET-Pi: RECORDING_LENGTH)</p>
      </div>
      <div>
        <label for="audio_channels">Audio Channels</label>
        <input id="audio_channels" name="audio_channels" type="number" value="{channels}" min="1" max="2" style="max-width:80px">
        <p class="hint">1 = mono (recommended), 2 = stereo (BirdNET-Pi: CHANNELS)</p>
      </div>
    </div>
    <div class="grid-2">
      <div>
        <label for="audio_format">Extracted Clip Format</label>
        <select id="audio_format" name="audio_format" style="max-width:180px">
          <option value="wav"{fmt_wav}>WAV (lossless, default)</option>
          <option value="mp3"{fmt_mp3}>MP3 (requires ffmpeg)</option>
          <option value="flac"{fmt_flac}>FLAC (lossless compressed, requires ffmpeg)</option>
          <option value="ogg"{fmt_ogg}>OGG (requires ffmpeg)</option>
        </select>
        <p class="hint">Format for saved detection audio clips (BirdNET-Pi: AUDIOFMT)</p>
      </div>
      <div>
        <label for="freq_shift_hz">Frequency Shift (Hz, 0 = disabled)</label>
        <input id="freq_shift_hz" name="freq_shift_hz" type="number" value="{freq_shift}"
               min="-12000" max="12000" step="500" style="max-width:120px">
        <p class="hint">Shift pitch of saved clips for accessibility (BirdNET-Pi: FREQ_SHIFT). Requires ffmpeg or sox. Typical: 1000–4000.</p>
      </div>
    </div>
  </div>"#));
}
