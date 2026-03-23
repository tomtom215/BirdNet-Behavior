//! SMTP email alert settings section.

use std::collections::HashMap;
use std::fmt::Write as _;

use super::get_setting;

pub(super) fn render(out: &mut String, s: &HashMap<String, String>) {
    let host = get_setting(s, "email_smtp_host", "");
    let port = get_setting(s, "email_smtp_port", "587");
    let user = get_setting(s, "email_smtp_user", "");
    let pass = get_setting(s, "email_smtp_pass", "");
    let from = get_setting(s, "email_from", "");
    let to = get_setting(s, "email_to", "");
    let name = get_setting(s, "email_from_name", "BirdNet-Behavior");
    let tls = get_setting(s, "email_starttls", "true");
    let tls_yes = if tls == "false" { "" } else { " selected" };
    let tls_no = if tls == "false" { " selected" } else { "" };
    let econf = get_setting(s, "email_min_confidence", "0.80");
    let ecool = get_setting(s, "email_cooldown_secs", "300");
    write!(out, r#"
  <div class="card">
    <div class="section-title">Email Alerts (SMTP)</div>
    <p class="hint" style="margin-bottom:1rem;">Leave SMTP host blank to disable email alerts.</p>
    <div class="grid-2">
      <div>
        <label for="email_smtp_host">SMTP Host</label>
        <input id="email_smtp_host" name="email_smtp_host" value="{host}" placeholder="smtp.gmail.com">
      </div>
      <div>
        <label for="email_smtp_port">SMTP Port</label>
        <input id="email_smtp_port" name="email_smtp_port" type="number" value="{port}" min="1" max="65535" style="max-width:120px">
      </div>
    </div>
    <div class="grid-2">
      <div>
        <label for="email_smtp_user">SMTP Username</label>
        <input id="email_smtp_user" name="email_smtp_user" value="{user}" placeholder="you@gmail.com">
      </div>
      <div>
        <label for="email_smtp_pass">SMTP Password / App Password</label>
        <input id="email_smtp_pass" name="email_smtp_pass" type="password" value="{pass}" placeholder="app-specific password">
      </div>
    </div>
    <div class="grid-2">
      <div>
        <label for="email_from">From Address</label>
        <input id="email_from" name="email_from" value="{from}" placeholder="alerts@example.com">
      </div>
      <div>
        <label for="email_to">To Address</label>
        <input id="email_to" name="email_to" value="{to}" placeholder="you@example.com">
      </div>
    </div>
    <div class="grid-2">
      <div>
        <label for="email_from_name">From Name</label>
        <input id="email_from_name" name="email_from_name" value="{name}" placeholder="BirdNet-Behavior">
      </div>
      <div>
        <label for="email_starttls">Use STARTTLS</label>
        <select id="email_starttls" name="email_starttls" style="max-width:180px">
          <option value="true"{tls_yes}>Yes (port 587)</option>
          <option value="false"{tls_no}>No — implicit TLS (port 465)</option>
        </select>
      </div>
    </div>
    <div class="grid-2">
      <div>
        <label for="email_min_confidence">Alert Min Confidence</label>
        <input id="email_min_confidence" name="email_min_confidence" type="number" value="{econf}" min="0" max="1" step="0.05">
        <p class="hint">Only email for detections above this threshold</p>
      </div>
      <div>
        <label for="email_cooldown_secs">Alert Cooldown (seconds)</label>
        <input id="email_cooldown_secs" name="email_cooldown_secs" type="number" value="{ecool}" min="0" step="60">
        <p class="hint">Min time between emails per species</p>
      </div>
    </div>
  </div>"#).unwrap_or_default();
}
