//! Toast notifications attached to htmx responses as OOB swaps.
//! See O-18 DIFF.md.
//!
//! Usage:
//!
//! ```ignore
//! use crate::routes::pages::toast;
//!
//! // Attach to any existing partial response.
//! let body = render_my_partial();
//! return toast::with(Html(body), toast::Toast::success("Settings saved."));
//!
//! // Stand-alone (no partial — pure notification).
//! return toast::oob_only(toast::Toast::warn("Restart required.")
//!     .with_action("/admin/system/restart", "Restart now"));
//! ```

use axum::response::Html;

use super::escape_html;

/// Toast tone.
#[derive(Clone, Copy)]
pub enum Kind {
    Success,
    Warn,
    Error,
    Info,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Kind::Success => "success",
            Kind::Warn => "warn",
            Kind::Error => "error",
            Kind::Info => "info",
        }
    }
}

/// One toast message.
pub struct Toast {
    kind: Kind,
    message: String,
    sticky: bool,
    timeout_ms: Option<u32>,
    action: Option<(String, String)>, // (href, label)
}

impl Toast {
    #[must_use]
    pub fn success(message: impl Into<String>) -> Self {
        Self::new(Kind::Success, message)
    }
    #[must_use]
    pub fn warn(message: impl Into<String>) -> Self {
        Self::new(Kind::Warn, message)
    }
    #[must_use]
    pub fn error(message: impl Into<String>) -> Self {
        // Errors are sticky by default so they're not missed.
        let mut t = Self::new(Kind::Error, message);
        t.sticky = true;
        t
    }
    #[must_use]
    pub fn info(message: impl Into<String>) -> Self {
        Self::new(Kind::Info, message)
    }

    fn new(kind: Kind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            sticky: false,
            timeout_ms: None,
            action: None,
        }
    }

    /// Disable the auto-dismiss timer.
    #[must_use]
    pub fn sticky(mut self) -> Self {
        self.sticky = true;
        self
    }

    /// Override the default per-kind auto-dismiss timeout (ms). 0 = sticky.
    #[must_use]
    pub fn with_timeout(mut self, ms: u32) -> Self {
        self.timeout_ms = Some(ms);
        self
    }

    /// Add a single inline action link (e.g. "Restart now").
    #[must_use]
    pub fn with_action(mut self, href: impl Into<String>, label: impl Into<String>) -> Self {
        self.action = Some((href.into(), label.into()));
        self
    }

    /// Render just the toast `<div>` (no OOB wrapper). Useful inside the region
    /// on initial page load.
    #[must_use]
    pub fn render(&self) -> String {
        let kind = self.kind.as_str();
        let sticky = if self.sticky { r#" data-sticky="1""# } else { "" };
        let timeout = self
            .timeout_ms
            .map(|m| format!(r#" data-timeout-ms="{m}""#))
            .unwrap_or_default();
        let role = match self.kind {
            Kind::Error => "alert",
            _ => "status",
        };
        let action = self
            .action
            .as_ref()
            .map(|(href, label)| {
                format!(
                    r#"<a class="bnb-toast__action" href="{}">{}</a>"#,
                    escape_html(href),
                    escape_html(label),
                )
            })
            .unwrap_or_default();
        format!(
            r#"<div class="bnb-toast bnb-rise" role="{role}" data-kind="{kind}"{sticky}{timeout}>
  <span class="bnb-toast__dot" aria-hidden="true"></span>
  <div class="bnb-toast__body">{msg}{action_html}</div>
  <button type="button" class="bnb-toast__close" data-toast-close aria-label="Dismiss">&times;</button>
</div>"#,
            msg = escape_html(&self.message),
            action_html = if action.is_empty() {
                String::new()
            } else {
                format!("<div>{action}</div>")
            },
        )
    }

    /// Render wrapped in an OOB swap targeting `#bnb-toasts` (append).
    #[must_use]
    pub fn render_oob(&self) -> String {
        format!(
            r#"<div id="bnb-toasts" hx-swap-oob="beforeend">{}</div>"#,
            self.render()
        )
    }
}

/// Attach a toast OOB fragment to an existing `Html<String>` body.
#[must_use]
pub fn with(mut html: Html<String>, toast: Toast) -> Html<String> {
    html.0.push_str(&toast.render_oob());
    html
}

/// Toast-only response — when the action doesn't return any visible partial.
#[must_use]
pub fn oob_only(toast: Toast) -> Html<String> {
    Html(toast.render_oob())
}

/// Plain `&str` helper, in case a handler is composing HTML by hand.
#[must_use]
pub fn success(msg: &str) -> String {
    Toast::success(msg.to_string()).render_oob()
}
#[must_use]
pub fn warn(msg: &str) -> String {
    Toast::warn(msg.to_string()).render_oob()
}
#[must_use]
pub fn error(msg: &str) -> String {
    Toast::error(msg.to_string()).render_oob()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_oob_wraps_target() {
        let html = success("Saved.");
        assert!(html.contains(r#"id="bnb-toasts""#));
        assert!(html.contains(r#"hx-swap-oob="beforeend""#));
        assert!(html.contains(r#"data-kind="success""#));
        assert!(html.contains("Saved."));
    }

    #[test]
    fn error_is_sticky_by_default() {
        let html = Toast::error("boom").render();
        assert!(html.contains(r#"data-sticky="1""#));
        assert!(html.contains(r#"role="alert""#));
    }

    #[test]
    fn warn_is_not_sticky_unless_called() {
        assert!(!Toast::warn("x").render().contains("data-sticky"));
        assert!(Toast::warn("x").sticky().render().contains("data-sticky"));
    }

    #[test]
    fn html_escapes_message_and_action() {
        let html = Toast::success("<x>")
            .with_action("/a?b=\"c\"", "click & here")
            .render();
        assert!(html.contains("&lt;x&gt;"));
        assert!(html.contains("click &amp; here"));
        assert!(html.contains("/a?b=&quot;c&quot;"));
    }

    #[test]
    fn with_extends_existing_body() {
        let base = Html("<p>body</p>".to_string());
        let res = with(base, Toast::success("ok"));
        assert!(res.0.contains("<p>body</p>"));
        assert!(res.0.contains("ok"));
        assert!(res.0.contains(r#"hx-swap-oob="beforeend""#));
    }
}
