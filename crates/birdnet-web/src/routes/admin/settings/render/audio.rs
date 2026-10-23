//! Audio capture settings section.

use std::collections::HashMap;
use std::fmt::Write as _;

use super::get_setting;

/// What the card shows in place of the microphone and network-camera addresses
/// once the audio-source list has an active row.
///
/// `summary` is the loader's one line per source, `kind<TAB>name<TAB>device`
/// (see `handler::audio_sources_summary`). The list is the single source of
/// truth for what the station captures: those addresses only seed it while it is
/// empty, and a field that is read once and then ignored is worse than no field.
/// Editing the list has an interface of its own, which does it properly — it can
/// add, remove, disable and tune one source — and which the addresses never could.
fn sources_note(summary: &str) -> String {
    use birdnet_db::audio_sources::SourceKind;
    let mut items = String::new();
    let (mut mics, mut streams) = (0_usize, 0_usize);
    for line in summary.lines() {
        let mut parts = line.splitn(3, '\t');
        let (kind, name, device) = (
            parts.next().unwrap_or(""),
            parts.next().unwrap_or(""),
            parts.next().unwrap_or(""),
        );
        let kind: Option<SourceKind> = kind.parse().ok();
        let what = match kind {
            Some(SourceKind::Rtsp) => {
                streams += 1;
                "Network stream"
            }
            Some(SourceKind::PipeWire) => {
                mics += 1;
                "PipeWire microphone"
            }
            Some(SourceKind::UsbAlsa) | None => {
                mics += 1;
                "USB microphone"
            }
        };
        let label = if name.is_empty() {
            String::new()
        } else {
            format!(" <b>{}</b> —", crate::routes::pages::escape_html(name))
        };
        let _ = write!(
            items,
            "<li>{what}:{label} <span class=\"mono\">{}</span></li>",
            crate::routes::pages::escape_html(device)
        );
    }
    let plural =
        |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    format!(
        r#"<div class="set-sources">
      <p><b>This station's microphones and streams are kept on
      <a href="/station/capture#audio">Capture → Audio &amp; Microphones</a></b>:
      {mics} and {streams}, listed here as they are saved. Add, change or remove
      them there.</p>
      <ul class="set-sources__list">{items}</ul>
      <p class="hint">The “microphone address” and “network camera address”
      settings only fill that list the first time the station starts with it
      empty. Once it has a source they are ignored, so this page does not offer
      them: there is one place to edit what the station records from.</p>
    </div>"#,
        mics = plural(mics, "microphone", "microphones"),
        streams = plural(streams, "network stream", "network streams"),
    )
}

pub(super) fn render(out: &mut String, s: &HashMap<String, String>) {
    let alsa = get_setting(s, "alsa_device");
    let rtsp = get_setting(s, "rtsp_url");
    let rtsp_urls = get_setting(s, "rtsp_urls");
    let seg = get_setting(s, "segment_duration");
    // The browser refuses to submit a form whose number input is outside its
    // own `min`/`max`, so these are `validate()`'s bounds, not a tidier range.
    let seg_min = birdnet_core::config::validate::SEGMENT_DURATION_MIN;
    let seg_max = birdnet_core::config::validate::SEGMENT_DURATION_MAX;
    let fmt = get_setting(s, "audio_format");
    let fmt_wav = if fmt == "wav" { " selected" } else { "" };
    let fmt_mp3 = if fmt == "mp3" { " selected" } else { "" };
    let fmt_flac = if fmt == "flac" { " selected" } else { "" };
    let fmt_ogg = if fmt == "ogg" { " selected" } else { "" };
    let freq_shift = get_setting(s, "freq_shift_hz");
    let address_hint = super::super::masking::ADDRESS_HINT;
    let lufs = get_setting(s, "clip_target_lufs");
    let lufs_opt = |v: &str| if lufs == v { " selected" } else { "" };
    let (lufs_off, lufs_14, lufs_18, lufs_23) = (
        lufs_opt(""),
        lufs_opt("-14"),
        lufs_opt("-18"),
        lufs_opt("-23"),
    );
    // Capture reads the audio-source table alone once it has an active row, so
    // the addresses below it are offered only while it has none.
    let capture_addresses = s.get(super::AUDIO_SOURCES_KEY).map_or_else(
        || format!(r#"    <div class="grid-2">
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
        app or manual will call an RTSP address — or an Icecast, HLS, RTMP,
        UDP or SRT stream's URL. Needs the <code>ffmpeg</code> program
        installed on the station. {address_hint}</p>
      </div>
    </div>
    <div>
      <label for="rtsp_urls">Several network cameras (separated by commas)</label>
      <input id="rtsp_urls" name="rtsp_urls" value="{rtsp_urls}" placeholder="rtsp://cam1:554/stream,rtsp://cam2:554/stream">
      <p class="hint">Each URL becomes an independent capture pipeline (RTSP_1-, RTSP_2- prefixed filenames). Overrides single RTSP URL above when set. {address_hint}</p>
    </div>
"#),
        |summary| sources_note(summary),
    );
    write!(out, r#"
  <section class="card" id="set-audio" aria-labelledby="set-audio-h">
    <h2 class="section-title" id="set-audio-h">Audio Capture</h2>
    <p class="hint flush"><a href="/recordings?view=live">▸ Listen live &amp; test your microphone →</a> — confirm the mic is picking up sound before tuning thresholds.</p>
    {capture_addresses}
    <div class="grid-2">
      <div>
        <label for="segment_duration">Segment Duration (seconds)</label>
        <input id="segment_duration" name="segment_duration" type="number" value="{seg}" min="{seg_min}" max="{seg_max}" class="bnb-w-num">
        <p class="hint">Length of each recording chunk for analysis (BirdNET-Pi: RECORDING_LENGTH)</p>
      </div>
      <div>
        <label>Audio Channels</label>
        <p class="hint standalone">Every source is recorded in mono; there is no control for this.
        On <a href="/admin/audio">Audio &amp; Microphones</a> you set a source's
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
