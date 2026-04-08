//! Server-Sent Events (SSE) log streaming for the admin panel.
//!
//! Provides a live log stream at `GET /admin/system/logs` using axum's SSE
//! support.  Log messages are captured by a custom `tracing` layer that
//! broadcasts to an unbounded channel; each SSE client receives a fresh
//! receiver on connection.
//!
//! | Path | Purpose |
//! |------|---------|
//! | `GET /admin/system/logs`       | SSE stream of recent log lines    |
//! | `GET /admin/system/logs/page`  | Standalone log viewer HTML page    |

use std::collections::VecDeque;
use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::response::{Html, Sse, sse::Event};
use axum::routing::get;
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::{Stream, StreamExt};

use crate::state::AppState;

// ---------------------------------------------------------------------------
// Log broadcast infrastructure
// ---------------------------------------------------------------------------

/// Maximum number of log lines buffered in the broadcast channel.
const LOG_CHANNEL_CAPACITY: usize = 512;

/// A captured log line.
#[derive(Debug, Clone)]
pub struct LogLine {
    /// Log level (ERROR, WARN, INFO, DEBUG, TRACE).
    pub level: String,
    /// Log message.
    pub message: String,
    /// Target module.
    pub target: String,
    /// Timestamp (Unix milliseconds).
    pub timestamp_ms: u64,
}

/// Maximum number of recent log lines retained for new subscriber warm-up.
const RECENT_LOG_CAPACITY: usize = 200;

/// Shared log broadcaster — inject into tracing subscriber, clone for SSE handlers.
#[derive(Debug, Clone)]
pub struct LogBroadcaster {
    tx: broadcast::Sender<LogLine>,
    /// Last N lines retained for new subscribers (ring buffer).
    recent: Arc<Mutex<VecDeque<LogLine>>>,
}

impl LogBroadcaster {
    /// Create a new broadcaster with the given capacity.
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(LOG_CHANNEL_CAPACITY);
        Self {
            tx,
            recent: Arc::new(Mutex::new(VecDeque::with_capacity(RECENT_LOG_CAPACITY))),
        }
    }

    /// Publish a new log line.  Silently ignores send errors (no receivers).
    pub fn publish(&self, line: LogLine) {
        // Retain in history ring buffer (O(1) eviction via VecDeque).
        if let Ok(mut buf) = self.recent.lock() {
            if buf.len() >= RECENT_LOG_CAPACITY {
                buf.pop_front();
            }
            buf.push_back(line.clone());
        }
        let _ = self.tx.send(line);
    }

    /// Subscribe to future log lines.
    pub fn subscribe(&self) -> broadcast::Receiver<LogLine> {
        self.tx.subscribe()
    }

    /// Return the last N retained log lines (for new subscriber warm-up).
    pub fn recent(&self, n: usize) -> Vec<LogLine> {
        self.recent
            .lock()
            .map(|buf| buf.iter().rev().take(n).cloned().collect::<Vec<_>>())
            .unwrap_or_default()
            .into_iter()
            .rev()
            .collect()
    }
}

impl Default for LogBroadcaster {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Route mounting
// ---------------------------------------------------------------------------

/// Mount log streaming routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/system/logs", get(log_stream))
        .route("/admin/system/logs/page", get(log_page))
}

// ---------------------------------------------------------------------------
// GET /admin/system/logs — SSE stream
// ---------------------------------------------------------------------------

async fn log_stream(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let broadcaster = state.log_broadcaster();
    let recent = broadcaster.recent(50);
    let rx = broadcaster.subscribe();

    // Build the stream: first drain recent lines, then follow live
    let recent_stream =
        tokio_stream::iter(recent).map(|line| Ok::<Event, Infallible>(log_line_to_event(&line)));

    let live_stream = BroadcastStream::new(rx)
        .filter_map(std::result::Result::ok)
        .map(|line| Ok::<Event, Infallible>(log_line_to_event(&line)));

    let combined = recent_stream.chain(live_stream);

    Sse::new(combined).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    )
}

fn log_line_to_event(line: &LogLine) -> Event {
    let level_class = match line.level.as_str() {
        "ERROR" => "level-error",
        "WARN" => "level-warn",
        "INFO" => "level-info",
        "DEBUG" => "level-debug",
        _ => "level-trace",
    };

    let html = format!(
        r#"<div class="log-line {level_class}">
          <span class="log-level">{level}</span>
          <span class="log-target">{target}</span>
          <span class="log-msg">{msg}</span>
        </div>"#,
        level_class = level_class,
        level = html_escape(&line.level),
        target = html_escape(&line.target),
        msg = html_escape(&line.message),
    );

    Event::default().event("log").data(html)
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

// ---------------------------------------------------------------------------
// GET /admin/system/logs/page — full log viewer page
// ---------------------------------------------------------------------------

async fn log_page(_: State<AppState>) -> Html<String> {
    Html(LOG_PAGE_HTML.to_string())
}

const LOG_PAGE_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width,initial-scale=1.0">
  <title>Live Logs — BirdNet-Behavior Admin</title>
  <script src="/static/htmx.min.js"></script>
  <script src="/static/htmx-sse.js"></script>
  <style>
    body { background:#0f172a; color:#e2e8f0; font-family:system-ui,sans-serif; margin:0; }
    .container { max-width:1100px; margin:0 auto; padding:2rem 1rem; }
    nav a { color:#94a3b8; text-decoration:none; margin-right:1.5rem; font-size:.9rem; }
    nav a:hover, nav a.active { color:#38bdf8; }
    h1 { font-size:1.5rem; font-weight:700; color:#f1f5f9; margin-bottom:1.5rem; }
    #log-panel {
      background:#0d1117; border:1px solid #21262d; border-radius:.5rem;
      padding:1rem; height:600px; overflow-y:auto; font-family:monospace;
      font-size:.8rem; display:flex; flex-direction:column; gap:2px;
    }
    .log-line { display:flex; gap:.75rem; line-height:1.4; }
    .log-level { min-width:50px; font-weight:700; }
    .log-target { color:#64748b; min-width:180px; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
    .log-msg { color:#cbd5e1; flex:1; word-break:break-all; }
    .level-error .log-level { color:#f87171; }
    .level-warn  .log-level { color:#fbbf24; }
    .level-info  .log-level { color:#4ade80; }
    .level-debug .log-level { color:#60a5fa; }
    .level-trace .log-level { color:#a78bfa; }
    .controls { display:flex; gap:1rem; margin-bottom:1rem; align-items:center; }
    .btn { padding:.4rem 1rem; border-radius:.375rem; border:1px solid #334155;
           background:#1e293b; color:#e2e8f0; cursor:pointer; font-size:.875rem; }
    .btn:hover { border-color:#38bdf8; color:#38bdf8; }
    .status { font-size:.8rem; color:#64748b; }
    .status.connected { color:#4ade80; }
  </style>
</head>
<body>
<div class="container">
  <nav style="margin-bottom:2rem;padding:1rem 0;border-bottom:1px solid #334155;">
    <a href="/">Dashboard</a>
    <a href="/admin/settings">Settings</a>
    <a href="/admin/system">System</a>
    <a href="/admin/system/logs/page" class="active">Live Logs</a>
  </nav>

  <h1>Live Log Stream</h1>

  <div class="controls">
    <button class="btn" onclick="clearLog()">Clear</button>
    <button class="btn" id="pause-btn" onclick="togglePause()">Pause</button>
    <label style="display:flex;align-items:center;gap:.5rem;font-size:.875rem;color:#94a3b8;">
      <input type="checkbox" id="scroll-lock" checked> Auto-scroll
    </label>
    <label style="display:flex;align-items:center;gap:.5rem;font-size:.875rem;color:#94a3b8;">
      Filter:
      <select id="level-filter" onchange="applyFilter()"
              style="background:#1e293b;border:1px solid #334155;color:#e2e8f0;
                     border-radius:.25rem;padding:.25rem .5rem;font-size:.8rem;">
        <option value="">All levels</option>
        <option value="level-error">ERROR</option>
        <option value="level-warn">WARN</option>
        <option value="level-info">INFO</option>
        <option value="level-debug">DEBUG</option>
      </select>
    </label>
    <span class="status" id="conn-status">Connecting…</span>
  </div>

  <div id="log-panel"
       hx-ext="sse"
       sse-connect="/admin/system/logs"
       sse-swap="log">
  </div>
</div>

<script>
  let paused = false;
  const panel = document.getElementById('log-panel');
  const MAX_LINES = 1000;

  document.body.addEventListener('htmx:sseOpen', () => {
    document.getElementById('conn-status').textContent = '● Connected';
    document.getElementById('conn-status').className = 'status connected';
  });
  document.body.addEventListener('htmx:sseError', () => {
    document.getElementById('conn-status').textContent = '✕ Disconnected';
    document.getElementById('conn-status').className = 'status';
  });
  document.body.addEventListener('htmx:afterSwap', () => {
    if (paused) return;
    // Trim old lines
    while (panel.children.length > MAX_LINES) panel.removeChild(panel.firstChild);
    if (document.getElementById('scroll-lock').checked) {
      panel.scrollTop = panel.scrollHeight;
    }
    applyFilter();
  });

  function clearLog() { panel.innerHTML = ''; }
  function togglePause() {
    paused = !paused;
    document.getElementById('pause-btn').textContent = paused ? 'Resume' : 'Pause';
  }
  function applyFilter() {
    const filter = document.getElementById('level-filter').value;
    for (const line of panel.children) {
      line.style.display = (!filter || line.classList.contains(filter)) ? '' : 'none';
    }
  }
</script>
</body>
</html>"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn broadcaster_publish_and_recent() {
        let b = LogBroadcaster::new();
        b.publish(LogLine {
            level: "INFO".into(),
            message: "hello".into(),
            target: "test".into(),
            timestamp_ms: 0,
        });
        let recent = b.recent(10);
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].message, "hello");
    }

    #[test]
    fn broadcaster_recent_capped_at_n() {
        let b = LogBroadcaster::new();
        for i in 0..10_u8 {
            b.publish(LogLine {
                level: "INFO".into(),
                message: format!("line {i}"),
                target: "t".into(),
                timestamp_ms: 0,
            });
        }
        let recent = b.recent(3);
        assert_eq!(recent.len(), 3);
        // Most recent last
        assert!(recent[2].message.contains("line 9"));
    }

    #[test]
    fn html_escape_xss() {
        assert_eq!(html_escape("<script>"), "&lt;script&gt;");
        assert_eq!(html_escape("a & b"), "a &amp; b");
    }

    #[test]
    fn log_line_to_event_level_class() {
        let line = LogLine {
            level: "ERROR".into(),
            message: "boom".into(),
            target: "db".into(),
            timestamp_ms: 0,
        };
        let event = log_line_to_event(&line);
        // Event should be named "log"
        let _ = event; // just verify no panic
    }

    #[test]
    fn broadcaster_subscribe_receives() {
        let b = LogBroadcaster::new();
        let mut rx = b.subscribe();
        b.publish(LogLine {
            level: "WARN".into(),
            message: "test".into(),
            target: "t".into(),
            timestamp_ms: 1,
        });
        let received = rx.try_recv().unwrap();
        assert_eq!(received.level, "WARN");
    }
}
