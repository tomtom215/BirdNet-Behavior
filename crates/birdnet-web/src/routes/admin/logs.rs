//! Server-Sent Events (SSE) log streaming for the admin panel.
//!
//! Provides a live log stream at `GET /admin/system/logs` using axum's SSE
//! support. Log messages are captured by a `tracing` layer in the binary
//! (`log_capture`), which publishes into the [`LogBroadcaster`] the
//! application hands to `AppState`; each SSE client receives a fresh receiver
//! on connection plus a replay of [`LogBroadcaster::recent`].
//!
//! This paragraph described that layer for a long time before one existed. It
//! did not, `AppState` held a channel nothing published to, and this endpoint
//! replayed an empty backlog and then emitted keep-alives for ever. The
//! channel is also **bounded** at `LOG_CHANNEL_CAPACITY`, not unbounded as
//! the same paragraph claimed: a slow client lags and is told so rather than
//! growing the buffer until the station runs out of memory.
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
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::{Stream, StreamExt as _};

use crate::routes::pages::escape_html;
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
    let mut rx = broadcaster.subscribe();
    let mut shutdown = state.subscribe_shutdown();

    // Forward log lines into a bounded channel from a task that also watches the
    // shutdown latch. When the server begins shutting down the task returns,
    // ending the SSE stream so this long-lived response stops holding axum's
    // graceful drain open until the grace backstop force-exits. (tokio-stream
    // has no `take_until`, so termination is driven explicitly here.)
    let (tx, out) = tokio::sync::mpsc::channel::<Result<Event, Infallible>>(64);
    tokio::spawn(async move {
        // Already shutting down? Don't start a long-lived stream at all.
        let already_shutting_down = *shutdown.borrow_and_update();
        if already_shutting_down {
            return;
        }
        // Replay the recent backlog first so a (re)connecting client has context.
        for line in recent {
            if tx.send(Ok(log_line_to_event(&line))).await.is_err() {
                return; // client disconnected
            }
        }
        loop {
            tokio::select! {
                // Server is shutting down — end the stream so the drain finishes.
                _ = shutdown.changed() => return,
                line = rx.recv() => match line {
                    Ok(line) => {
                        if tx.send(Ok(log_line_to_event(&line))).await.is_err() {
                            return; // client disconnected
                        }
                    }
                    // Drop lagged lines (matches the previous filter_map(ok)).
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => return,
                }
            }
        }
    });

    // The stream opens with a comment of its own. Firefox fires an
    // `EventSource`'s `onopen` on the first byte of the *body*, not on the
    // response headers (Chromium and WebKit use the headers), and on a quiet
    // log the first body byte used to be the 15-second keep-alive below. The
    // viewer said "Connecting…" for up to a quarter of a minute, in Firefox
    // only, over a stream that was already open and healthy. A comment line is
    // ignored by every `EventSource` and by the viewer's `sse-swap="log"`.
    Sse::new(
        tokio_stream::once(Ok(Event::default().comment(CONNECTED_COMMENT)))
            .chain(ReceiverStream::new(out)),
    )
    .keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    )
}

/// The comment the log stream sends the moment it opens; see [`log_stream`].
const CONNECTED_COMMENT: &str = "connected";

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
        level = escape_html(&line.level),
        target = escape_html(&line.target),
        msg = escape_html(&line.message),
    );

    Event::default().event("log").data(html)
}

// ---------------------------------------------------------------------------
// GET /admin/system/logs/page — full log viewer page
// ---------------------------------------------------------------------------

async fn log_page(_: State<AppState>) -> Html<String> {
    Html(render_log_page())
}

/// The log viewer inside the admin shell — what [`log_page`] answers, apart
/// from the state it never reads.
fn render_log_page() -> String {
    crate::routes::admin::admin_subpage_shell(
        "Live logs",
        "system",
        "Live logs",
        &crate::routes::pages::with_asset_version(LOG_PAGE_BODY.as_str()),
    )
}

/// The log viewer's body. A page inside the admin shell like every other, not a
/// document of its own: it used to carry its own `<html>`, a four-link nav that
/// said "Dashboard" and named three of the admin pages, and its own styles.
/// The SSE extension is loaded here, after the layout's `htmx.min.js`.
static LOG_PAGE_BODY: crate::seal::Sealed = crate::seal::Sealed::new(
    r#"<script src="/static/htmx-sse.js?v={{version}}"></script>
<section class="logv">
  <h1 class="logv-title">Live log stream</h1>

  <div class="logv-controls">
    <button type="button" class="bnb-btn ghost" id="clear-btn">Clear</button>
    <button type="button" class="bnb-btn ghost" id="pause-btn">Pause</button>
    <label class="logv-ctl">
      <input type="checkbox" id="scroll-lock" checked> Auto-scroll
    </label>
    <label class="logv-ctl">
      Filter:
      <select id="level-filter" class="logv-select">
        <option value="">All levels</option>
        <option value="level-error">ERROR</option>
        <option value="level-warn">WARN</option>
        <option value="level-info">INFO</option>
        <option value="level-debug">DEBUG</option>
      </select>
    </label>
    <span class="logv-status" id="conn-status" role="status">Connecting…</span>
  </div>

  <div id="log-panel"
       class="logv-panel"
       hx-ext="sse"
       sse-connect="/admin/system/logs"
       sse-swap="log">
  </div>
</section>

<script>
  let paused = false;
  const panel = document.getElementById('log-panel');
  const MAX_LINES = 1000;

  document.body.addEventListener('htmx:sseOpen', () => {
    document.getElementById('conn-status').textContent = '● Connected';
    document.getElementById('conn-status').className = 'logv-status connected';
  });
  document.body.addEventListener('htmx:sseError', () => {
    document.getElementById('conn-status').textContent = '✕ Disconnected';
    document.getElementById('conn-status').className = 'logv-status';
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

  document.getElementById('clear-btn').addEventListener('click', clearLog);
  document.getElementById('pause-btn').addEventListener('click', togglePause);
  document.getElementById('level-filter').addEventListener('change', applyFilter);
</script>"#,
);

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

    /// A log line carries whatever reached the logger — a species name from a
    /// model, an HTTP header, an MQTT payload — straight into this page's
    /// markup over SSE. The escaping is gated on `escape_html`; what this pins
    /// is that the event builder still routes through it.
    #[test]
    fn a_log_line_is_escaped_before_it_reaches_the_stream() {
        let line = LogLine {
            level: "ERROR".into(),
            target: "birdnet".into(),
            message: "<img src=x onerror='alert(1)'> & \"quoted\"".into(),
            timestamp_ms: 0,
        };
        let html = format!("{:?}", log_line_to_event(&line));
        assert!(!html.contains("<img"), "unescaped tag: {html}");
        assert!(html.contains("&lt;img"), "escaped tag missing: {html}");
        assert!(
            html.contains("&#x27;"),
            "the apostrophe must be escaped too"
        );
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

    #[test]
    fn log_page_has_no_inline_style_attributes() {
        // P3-3 (O-25): styling is classes in app.css, never a markup `style=`
        // attribute. (The JS `line.style.display` CSSOM mutation is not a CSP
        // style-src concern — only markup `style=` attributes are.)
        assert!(!LOG_PAGE_BODY.contains("style=\""));
        // ...and none of the page's own CSS is left in a `<style>` block either:
        // it lives in app.css with the rest, where the shared layout loads it.
        assert!(!LOG_PAGE_BODY.contains("<style"));
    }

    /// The viewer is a page in the admin shell, not a document of its own: it has
    /// the site header and tab bar, the admin nav with System lit, and a
    /// breadcrumb — and the stale four-link nav that said "Dashboard" is gone.
    #[test]
    fn the_log_viewer_is_rendered_through_the_shared_layout() {
        let html = render_log_page();
        assert_eq!(html.matches("<html").count(), 1, "one document, not two");
        assert!(html.contains(r#"class="topnav""#), "the site header");
        assert!(html.contains("bnb-tabbar"), "the phone tab bar");
        assert!(html.contains(r#"href="/admin/system" class="am-nav-active""#));
        assert!(html.contains("Live logs"), "the breadcrumb leaf");
        assert!(!html.contains(">Dashboard<"), "the old bespoke nav");
        assert!(
            html.contains("htmx-sse.js"),
            "the SSE extension is still loaded"
        );
        assert!(
            html.find("htmx.min.js").unwrap() < html.find("htmx-sse.js").unwrap(),
            "the extension registers against htmx, so htmx loads first"
        );
        assert!(!html.contains("{{version}}"), "asset versions are stamped");
    }
}
