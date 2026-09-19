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
  <section class="card" id="set-email" aria-labelledby="set-email-h">
    <h2 class="section-title" id="set-email-h">Email Alerts</h2>
    <p class="hint">Send yourself an email when the station hears something.
    These are the same details you would type to add an address to a phone's
    mail app, and your email provider publishes them under "SMTP" or "outgoing
    mail". Leave the server blank to send no email at all.</p>
    <p class="hint">Gmail, Outlook and most others will not accept your normal
    password here — you have to create a separate <em>app password</em> in your
    email account's security settings and paste that instead.</p>
    <div class="grid-2">
      <div>
        <label for="email_smtp_host">Outgoing mail server (SMTP host)</label>
        <input id="email_smtp_host" name="email_smtp_host" value="{host}" placeholder="smtp.gmail.com">
        <p class="hint">From your email provider's help pages.</p>
      </div>
      <div>
        <label for="email_smtp_port">Port</label>
        <input id="email_smtp_port" name="email_smtp_port" type="number" value="{port}" min="1" max="65535" class="bnb-w-num">
        <p class="hint">Almost always 587. Use 465 if your provider says to,
        and set the encryption below to match.</p>
      </div>
    </div>
    <div class="grid-2">
      <div>
        <label for="email_smtp_user">Your email address</label>
        <input id="email_smtp_user" name="email_smtp_user" value="{user}" placeholder="you@gmail.com">
        <p class="hint">The account the station signs in as.</p>
      </div>
      <div>
        <label for="email_smtp_pass">App password</label>
        <input id="email_smtp_pass" name="email_smtp_pass" type="password" value="{pass}" placeholder="app-specific password">
        <p class="hint">Not your everyday password — see the note above.</p>
      </div>
    </div>
    <div class="grid-2">
      <div>
        <label for="email_from">Send from</label>
        <input id="email_from" name="email_from" value="{from}" placeholder="alerts@example.com">
        <p class="hint">Usually the same address as above.</p>
      </div>
      <div>
        <label for="email_to">Send to</label>
        <input id="email_to" name="email_to" value="{to}" placeholder="you@example.com">
        <p class="hint">Where the alerts should arrive.</p>
      </div>
    </div>
    <div class="grid-2">
      <div>
        <label for="email_from_name">Sender name</label>
        <input id="email_from_name" name="email_from_name" value="{name}" placeholder="BirdNet-Behavior">
        <p class="hint">What the alert appears to be from in your inbox.</p>
      </div>
      <div>
        <label for="email_starttls">Encryption</label>
        <select id="email_starttls" name="email_starttls" class="bnb-w-select">
          <option value="true"{tls_yes}>Usual — STARTTLS, with port 587</option>
          <option value="false"{tls_no}>Older — direct TLS, with port 465</option>
        </select>
        <p class="hint">Pick the one that matches the port above. If you are
        unsure, leave it on "Usual".</p>
      </div>
    </div>
    <div class="grid-2">
      <div>
        <label for="email_min_confidence">Only email when at least this sure (0–1)</label>
        <input id="email_min_confidence" name="email_min_confidence" type="text"
               inputmode="decimal" pattern="[0-9]*[.,]?[0-9]*" value="{econf}" placeholder="0.80">
        <p class="hint">0.80 means "at least 80% sure". Lower catches more
        birds and more mistakes.</p>
      </div>
      <div>
        <label for="email_cooldown_secs">Wait this long before emailing about the same bird again (seconds)</label>
        <input id="email_cooldown_secs" name="email_cooldown_secs" type="number" value="{ecool}" min="0" step="60">
        <p class="hint">300 is five minutes. A dawn chorus can otherwise fill your inbox.</p>
      </div>
    </div>
  </section>"#).unwrap_or_default();
}
