//! Email body templates for bird detection notifications.
//!
//! Generates both plain-text and HTML email bodies from detection data.
//! All user-supplied strings are HTML-escaped to prevent injection.

use super::types::DetectionEmail;

/// Escape HTML special characters to prevent injection.
fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Build the email subject line.
pub fn subject(detection: &DetectionEmail) -> String {
    format!(
        "🐦 Bird detected: {} ({:.0}% confidence)",
        detection.common_name,
        detection.confidence * 100.0,
    )
}

/// Build a plain-text email body.
pub fn plain_body(detection: &DetectionEmail) -> String {
    let station = detection
        .station_name
        .as_deref()
        .unwrap_or("BirdNet-Behavior");

    let url_line = detection
        .detection_url
        .as_deref()
        .map(|u| format!("\nView detection: {u}\n"))
        .unwrap_or_default();

    format!(
        "Bird Detection Alert — {station}\n\
         ═══════════════════════════════\n\
         Species:    {common}\n\
         Scientific: {sci}\n\
         Confidence: {conf:.0}%\n\
         Date:       {date}\n\
         Time:       {time}\n\
         {url_line}\n\
         ───────────────────────────────\n\
         Sent by BirdNet-Behavior · https://github.com/tomtom215/BirdNet-Behavior",
        station = station,
        common = detection.common_name,
        sci = detection.scientific_name,
        conf = detection.confidence * 100.0,
        date = detection.date,
        time = detection.time,
        url_line = url_line,
    )
}

/// Build an HTML email body with inline CSS (wide client compatibility).
pub fn html_body(detection: &DetectionEmail) -> String {
    let station = escape(
        detection
            .station_name
            .as_deref()
            .unwrap_or("BirdNet-Behavior"),
    );
    let common = escape(&detection.common_name);
    let sci = escape(&detection.scientific_name);
    let conf_pct = (detection.confidence * 100.0).round() as u32;
    let date = escape(&detection.date);
    let time = escape(&detection.time);

    let conf_color = if conf_pct >= 80 {
        "#4ade80"
    } else if conf_pct >= 60 {
        "#fbbf24"
    } else {
        "#f87171"
    };

    let url_block = detection
        .detection_url
        .as_deref()
        .map(|u| {
            format!(
                r#"<tr><td colspan="2" style="padding:12px 0 0;">
                  <a href="{u}" style="display:inline-block;padding:8px 20px;
                     background:#0ea5e9;color:#fff;border-radius:6px;
                     text-decoration:none;font-weight:600;font-size:14px;">
                    View Detection →
                  </a></td></tr>"#
            )
        })
        .unwrap_or_default();

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head><meta charset="UTF-8"><meta name="viewport" content="width=device-width,initial-scale=1"></head>
<body style="margin:0;padding:0;background:#0f172a;font-family:system-ui,sans-serif;">
<table width="100%" cellpadding="0" cellspacing="0"
       style="background:#0f172a;padding:32px 16px;">
  <tr><td align="center">
    <table width="560" cellpadding="0" cellspacing="0"
           style="background:#1e293b;border-radius:12px;overflow:hidden;
                  border:1px solid #334155;max-width:560px;width:100%;">
      <!-- Header -->
      <tr>
        <td style="background:#0c4a6e;padding:20px 24px;">
          <p style="margin:0;font-size:11px;color:#7dd3fc;text-transform:uppercase;
                    letter-spacing:1px;">{station}</p>
          <h1 style="margin:6px 0 0;font-size:22px;color:#f0f9ff;font-weight:700;">
            🐦 Bird Detected
          </h1>
        </td>
      </tr>
      <!-- Body -->
      <tr>
        <td style="padding:24px;">
          <h2 style="margin:0 0 4px;font-size:20px;color:#e2e8f0;font-weight:700;">
            {common}
          </h2>
          <p style="margin:0 0 20px;font-size:13px;color:#94a3b8;font-style:italic;">
            {sci}
          </p>
          <table width="100%" cellpadding="0" cellspacing="0">
            <tr>
              <td style="padding:8px 0;border-bottom:1px solid #334155;
                         font-size:13px;color:#94a3b8;width:40%;">Confidence</td>
              <td style="padding:8px 0;border-bottom:1px solid #334155;
                         font-size:15px;color:{conf_color};font-weight:700;">
                {conf_pct}%
              </td>
            </tr>
            <tr>
              <td style="padding:8px 0;border-bottom:1px solid #334155;
                         font-size:13px;color:#94a3b8;">Date</td>
              <td style="padding:8px 0;border-bottom:1px solid #334155;
                         font-size:14px;color:#e2e8f0;">{date}</td>
            </tr>
            <tr>
              <td style="padding:8px 0;border-bottom:1px solid #334155;
                         font-size:13px;color:#94a3b8;">Time</td>
              <td style="padding:8px 0;border-bottom:1px solid #334155;
                         font-size:14px;color:#e2e8f0;">{time}</td>
            </tr>
            {url_block}
          </table>
        </td>
      </tr>
      <!-- Footer -->
      <tr>
        <td style="padding:16px 24px;border-top:1px solid #334155;
                   font-size:11px;color:#475569;text-align:center;">
          Sent by BirdNet-Behavior · Acoustic Bird Monitoring
        </td>
      </tr>
    </table>
  </td></tr>
</table>
</body>
</html>"#,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> DetectionEmail {
        DetectionEmail {
            common_name: "European Robin".into(),
            scientific_name: "Erithacus rubecula".into(),
            confidence: 0.92,
            date: "2026-03-13".into(),
            time: "07:12:34".into(),
            station_name: Some("My Garden".into()),
            detection_url: Some("http://localhost:8080/species/detail?name=European+Robin".into()),
        }
    }

    #[test]
    fn subject_contains_species() {
        let s = subject(&sample());
        assert!(s.contains("European Robin"));
        assert!(s.contains("92%"));
    }

    #[test]
    fn plain_body_contains_all_fields() {
        let body = plain_body(&sample());
        assert!(body.contains("European Robin"));
        assert!(body.contains("Erithacus rubecula"));
        assert!(body.contains("92%"));
        assert!(body.contains("2026-03-13"));
        assert!(body.contains("07:12:34"));
        assert!(body.contains("My Garden"));
    }

    #[test]
    fn html_body_escapes_xss() {
        let mut d = sample();
        d.common_name = "<script>alert(1)</script>".into();
        let html = html_body(&d);
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;"));
    }

    #[test]
    fn html_body_contains_confidence_color() {
        let body = html_body(&sample());
        // 92% → green
        assert!(body.contains("#4ade80"));
    }

    #[test]
    fn plain_body_no_url_line_when_absent() {
        let mut d = sample();
        d.detection_url = None;
        let body = plain_body(&d);
        assert!(!body.contains("View detection:"));
    }
}
