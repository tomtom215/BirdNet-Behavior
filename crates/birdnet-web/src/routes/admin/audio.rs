//! Audio settings — microphone (USB + RTSP) management.
//!
//! The primary support surface: a source list with per-input level/uptime/last
//! detection, an expandable tuning panel (gain, sample rate, channels, bit
//! depth, pipeline toggles), an Add-RTSP wizard and researcher options. The
//! layout and controls are fully realised; live device enumeration / level
//! metering / persistence are a clearly-scoped stub (a production build wires
//! these to the audio daemon and settings store).

use std::fmt::Write as _;

use axum::Router;
use axum::extract::State;
use axum::response::Html;
use axum::routing::get;

use super::admin_shell;
use crate::routes::pages::escape_html;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/admin/audio", get(audio_page))
}

async fn audio_page(State(state): State<AppState>) -> Html<String> {
    let configured = state.audio_source().map(ToString::to_string);
    Html(admin_shell(
        "Audio",
        "audio",
        &render_body(configured.as_deref()),
    ))
}

/// A horizontal level meter (0–100) with an SNR caption.
fn level_meter(pct: u32, snr_db: f64, color: &str) -> String {
    format!(
        r#"<div style="display:flex;flex-direction:column;gap:3px;min-width:120px;">
  <div style="height:7px;border-radius:4px;background:var(--surface-2);overflow:hidden;"><span style="display:block;height:100%;width:{pct}%;background:{color};"></span></div>
  <span class="bnb-meta mono" style="font-size:10px;">{pct}% · {snr_db:.0} dB SNR</span>
</div>"#
    )
}

/// One source row in the 7-column grid.
#[allow(clippy::too_many_arguments, clippy::fn_params_excessive_bools)]
fn source_row(
    icon: &str,
    name: &str,
    path: &str,
    detail: &str,
    meter: &str,
    uptime: &str,
    last: &str,
    last_moss: bool,
    count: i64,
    expanded: bool,
) -> String {
    let last_col = if last_moss {
        "var(--moss-ink)"
    } else {
        "var(--fg-3)"
    };
    let tune = if expanded { "▾ tune" } else { "▸ tune" };
    format!(
        r#"<div style="display:grid;grid-template-columns:34px 1.6fr 1.3fr 1fr 1.4fr auto auto;gap:14px;align-items:center;padding:14px 4px;border-top:0.5px solid var(--hairline);">
  <span style="width:34px;height:34px;border-radius:8px;background:var(--surface-2);display:flex;align-items:center;justify-content:center;">{icon}</span>
  <div style="min-width:0;overflow:hidden;"><div style="font-weight:500;font-size:13.5px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;">{name}</div><div class="bnb-meta mono" style="white-space:nowrap;overflow:hidden;text-overflow:ellipsis;">{path}</div><div class="bnb-meta" style="white-space:nowrap;overflow:hidden;text-overflow:ellipsis;">{detail}</div></div>
  {meter}
  <div><div class="bnb-meta">uptime</div><div class="mono" style="font-size:12.5px;">{uptime}</div></div>
  <div><div class="bnb-meta">last detection</div><div class="mono" style="font-size:12px;color:{last_col};">{last}</div></div>
  <div style="text-align:right;"><div class="display" style="font-size:20px;">{count}</div><div class="bnb-meta">24 h</div></div>
  <button class="bnb-btn ghost" style="white-space:nowrap;">{tune}</button>
</div>"#
    )
}

#[allow(clippy::too_many_lines)]
fn render_body(configured: Option<&str>) -> String {
    let primary_path = configured.map_or_else(|| "hw:1,0".to_string(), escape_html);

    let sections = [
        ("Detection", "/admin/settings", false),
        ("Audio", "/admin/audio", true),
        ("Location", "/admin/settings", false),
        ("Notifications", "/admin/notifications", false),
        ("Species", "/admin/species", false),
        ("System", "/admin/system", false),
        ("Backups", "/admin/backups", false),
    ];
    let mut side = String::from(
        r#"<aside class="bnb-card pad" style="position:sticky;top:16px;"><div class="bnb-eyebrow" style="margin-bottom:8px;">Settings</div>"#,
    );
    for (label, href, active) in sections {
        let st = if active {
            "background:var(--moss-soft);color:var(--moss-ink);font-weight:500;"
        } else {
            "color:var(--fg-2);"
        };
        let _ = write!(
            side,
            r#"<a href="{href}" style="display:block;padding:7px 10px;border-radius:8px;text-decoration:none;font-size:13px;margin-bottom:2px;{st}">{label}</a>"#
        );
    }
    side.push_str("</aside>");

    let usb = source_row(
        "🎤",
        "UMC202HD · USB audio",
        &primary_path,
        "USB · 48 kHz · 24-bit · auto-detected",
        &level_meter(64, 42.0, "var(--moss)"),
        "14 d 02 h",
        "8 s ago · Northern Cardinal",
        true,
        1284,
        true,
    );
    let rtsp = source_row(
        "📡",
        "Feeder cam (RTSP)",
        "rtsp://192.168.1.42/audio",
        "RTSP · 16 kHz · tcp · keepalive on",
        &level_meter(38, 27.0, "var(--dawn)"),
        "6 d 11 h · stable",
        "2 m ago · Blue Jay",
        false,
        417,
        false,
    );

    // Expanded tune panel for the USB source.
    let tune_panel = r#"<div class="bnb-card" style="margin:0 4px 8px;padding:18px;background:var(--surface-2);">
  <div class="bnb-eyebrow" style="margin-bottom:14px;">Tuning · UMC202HD</div>
  <div style="display:grid;grid-template-columns:1fr 1fr;gap:18px 28px;">
    <div>
      <label class="bnb-meta">Input gain <span class="mono" style="color:var(--fg);">+6 dB</span></label>
      <div style="position:relative;height:26px;display:flex;align-items:center;">
        <input type="range" min="-12" max="24" value="6" style="width:100%;accent-color:var(--moss);">
      </div>
      <div style="display:flex;justify-content:space-between;" class="bnb-meta mono"><span>−12</span><span>0</span><span>+24</span></div>
    </div>
    <div>
      <label class="bnb-meta">Sample rate</label>
      <select style="width:100%;padding:8px;border-radius:6px;border:0.5px solid var(--border-2);background:var(--surface);color:var(--fg);"><option>8 kHz</option><option>16 kHz</option><option>22.05 kHz</option><option>44.1 kHz</option><option selected>48 kHz</option></select>
    </div>
    <div>
      <label class="bnb-meta">Channels</label>
      <select style="width:100%;padding:8px;border-radius:6px;border:0.5px solid var(--border-2);background:var(--surface);color:var(--fg);"><option>Mono (mix)</option><option selected>Left</option><option>Right</option><option>Stereo</option></select>
    </div>
    <div>
      <label class="bnb-meta">Bit depth</label>
      <select style="width:100%;padding:8px;border-radius:6px;border:0.5px solid var(--border-2);background:var(--surface);color:var(--fg);"><option>16-bit PCM</option><option selected>24-bit PCM</option></select>
    </div>
  </div>
  <div class="bnb-eyebrow" style="margin:18px 0 8px;">Pipeline</div>
  <div style="display:flex;flex-wrap:wrap;gap:8px;">
    <span class="bnb-pill moss">✓ High-pass filter</span>
    <span class="bnb-pill moss">✓ DC offset removal</span>
    <span class="bnb-pill">Auto-gain control</span>
    <span class="bnb-pill moss">✓ RTSP keepalive</span>
  </div>
  <div style="display:flex;gap:10px;justify-content:flex-end;margin-top:16px;">
    <button class="bnb-btn ghost">Discard</button>
    <button class="bnb-btn primary">Apply</button>
  </div>
</div>"#;

    let add_rtsp = r#"<div class="bnb-card pad" style="margin-top:16px;">
  <div class="section-header"><div><div class="bnb-eyebrow">Add a source</div><h3>Network camera (RTSP)</h3></div></div>
  <div style="display:grid;grid-template-columns:1.4fr 1fr 1fr;gap:16px;margin-top:8px;">
    <div>
      <div class="bnb-meta" style="margin-bottom:5px;">1 · Stream URL</div>
      <div style="display:flex;align-items:stretch;border:0.5px solid var(--border-2);border-radius:6px;overflow:hidden;">
        <span class="mono" style="background:var(--surface-2);padding:8px 8px;color:var(--fg-3);font-size:12px;">rtsp://</span>
        <input type="text" value="192.168.1.42/audio" style="border:0;flex:1;padding:8px;background:var(--surface);color:var(--fg);font:inherit;">
      </div>
      <div style="margin-top:8px;"><span class="bnb-pill moss">● reachable · 16 kHz mono AAC</span></div>
    </div>
    <div>
      <div class="bnb-meta" style="margin-bottom:5px;">2 · Auth (optional)</div>
      <input type="text" placeholder="username" style="width:100%;padding:8px;border-radius:6px;border:0.5px solid var(--border-2);background:var(--surface);color:var(--fg);margin-bottom:6px;">
      <input type="password" placeholder="password" style="width:100%;padding:8px;border-radius:6px;border:0.5px solid var(--border-2);background:var(--surface);color:var(--fg);">
    </div>
    <div>
      <div class="bnb-meta" style="margin-bottom:5px;">3 · Label</div>
      <input type="text" placeholder="Feeder cam" style="width:100%;padding:8px;border-radius:6px;border:0.5px solid var(--border-2);background:var(--surface);color:var(--fg);">
    </div>
  </div>
  <div style="display:flex;align-items:center;gap:12px;margin-top:14px;padding-top:14px;border-top:0.5px solid var(--hairline);">
    <span class="waveform" aria-hidden="true" style="flex:1;height:34px;display:flex;align-items:center;gap:2px;">PREVIEW</span>
    <button class="bnb-btn ghost">▶ Listen for 10 s</button>
    <button class="bnb-btn primary">Add to sources</button>
  </div>
</div>"#;

    let researcher = r#"<details class="bnb-card pad" style="margin-top:16px;">
  <summary class="bnb-meta" style="cursor:pointer;">Researcher options <span class="bnb-pill">adv</span></summary>
  <div style="display:grid;grid-template-columns:repeat(2,1fr);gap:14px 24px;margin-top:14px;">
    <div><label class="bnb-meta">RTSP transport</label><select style="width:100%;padding:7px;border-radius:6px;border:0.5px solid var(--border-2);background:var(--surface);color:var(--fg);"><option>auto</option><option selected>tcp</option><option>udp</option></select></div>
    <div><label class="bnb-meta">Reconnect backoff</label><input type="text" value="2s → 32s exponential" style="width:100%;padding:7px;border-radius:6px;border:0.5px solid var(--border-2);background:var(--surface);color:var(--fg);"></div>
    <div><label class="bnb-meta">Multi-source mode</label><select style="width:100%;padding:7px;border-radius:6px;border:0.5px solid var(--border-2);background:var(--surface);color:var(--fg);"><option selected>parallel</option><option>round-robin</option></select></div>
    <div><label class="bnb-meta">Clock-drift correction</label><span class="bnb-pill moss" style="margin-top:6px;display:inline-flex;">✓ enabled</span></div>
  </div>
</details>"#;

    let main = format!(
        r#"<div>
  <div class="bnb-card pad">
    <div class="section-header"><div><div class="bnb-eyebrow">Inputs</div><h3>Microphone sources</h3></div><span class="bnb-pill">2 active</span></div>
    {usb}
    {tune_panel}
    {rtsp}
  </div>
  {add_rtsp}
  {researcher}
</div>"#
    );

    let rail = r#"<aside style="display:flex;flex-direction:column;gap:16px;position:sticky;top:16px;">
  <div class="bnb-card pad">
    <div class="bnb-eyebrow">Combined input</div>
    <div class="display" style="font-size:34px;margin-top:4px;">2 mics</div>
    <div class="bnb-meta">1,701 detections in the last 24 h</div>
    <div style="margin-top:12px;display:flex;flex-direction:column;gap:8px;">
      <div><div class="bnb-meta">UMC202HD</div><div style="height:6px;border-radius:3px;background:var(--surface-2);"><span style="display:block;height:100%;width:64%;background:var(--moss);border-radius:3px;"></span></div></div>
      <div><div class="bnb-meta">Feeder cam</div><div style="height:6px;border-radius:3px;background:var(--surface-2);"><span style="display:block;height:100%;width:38%;background:var(--dawn);border-radius:3px;"></span></div></div>
    </div>
  </div>
  <div class="bnb-card pad">
    <div class="bnb-eyebrow" style="margin-bottom:8px;">Common pitfalls</div>
    <ul style="margin:0;padding-left:1.1rem;font-size:12.5px;color:var(--fg-2);display:flex;flex-direction:column;gap:8px;">
      <li>Gain too high clips the loudest calls — aim for peaks near −6 dB.</li>
      <li>USB hubs can drop audio under load; prefer a direct port on the Pi.</li>
      <li>RTSP over UDP drops packets on busy Wi-Fi — use tcp for stability.</li>
    </ul>
  </div>
</aside>"#;

    format!(
        r#"<div>
  <div class="bnb-eyebrow">Audio · primary support surface</div>
  <h1 class="display" style="font-size:34px;margin:6px 0 4px;">Microphone setup</h1>
  <p class="bnb-meta" style="margin-bottom:20px;">Every input the station listens on — levels, uptime and last detection at a glance. Expand a source to tune it.</p>
  <div style="display:grid;grid-template-columns:200px minmax(0,1fr) 280px;gap:24px;align-items:start;">
    {side}
    {main}
    {rail}
  </div>
</div>"#
    )
}
