//! Themed confirmation-modal helper. See O-17 DIFF.md.
//!
//! Emits a button that triggers an htmx request after a user confirms in the
//! `<dialog>`-based modal mounted by `_partial_confirm_modal.html` in
//! `layout.html`. Both an `hx-confirm` (native fallback) and the matching
//! `data-confirm-*` attributes are emitted — if JS is off or `<dialog>` is
//! missing, the request still gates on the native dialog.

use super::escape_html;

/// Which HTTP verb + endpoint the confirm triggers.
#[derive(Clone, Copy)]
// Complete verb surface; only Post/Delete have confirm sites today.
#[allow(dead_code)]
pub enum Action<'a> {
    Get(&'a str),
    Post(&'a str),
    Delete(&'a str),
    Put(&'a str),
    Patch(&'a str),
}

impl<'a> Action<'a> {
    const fn hx_attr(self) -> (&'static str, &'static str) {
        match self {
            Self::Get(_) => ("hx-get", "GET"),
            Self::Post(_) => ("hx-post", "POST"),
            Self::Delete(_) => ("hx-delete", "DELETE"),
            Self::Put(_) => ("hx-put", "PUT"),
            Self::Patch(_) => ("hx-patch", "PATCH"),
        }
    }
    const fn url(self) -> &'a str {
        match self {
            Self::Get(u) | Self::Post(u) | Self::Delete(u) | Self::Put(u) | Self::Patch(u) => u,
        }
    }
}

/// Tone of the action — paints the primary button + dialog frame.
#[derive(Clone, Copy, Default)]
// `Warn` tone is part of the API but unused by current confirm sites.
#[allow(dead_code)]
pub enum Style {
    #[default]
    Moss,
    Danger,
    Warn,
}

impl Style {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Moss => "moss",
            Self::Danger => "danger",
            Self::Warn => "warn",
        }
    }
    /// Class on the trigger button itself (the page button, not the OK button).
    const fn trigger_btn_class(self) -> &'static str {
        match self {
            Self::Danger => "bnb-btn danger",
            Self::Warn | Self::Moss => "bnb-btn",
        }
    }
}

/// Fully-specified confirm trigger.
#[allow(clippy::struct_field_names)]
pub struct Confirm<'a> {
    pub label: &'a str,
    pub action: Action<'a>,
    pub title: &'a str,
    pub body: &'a str,
    pub confirm_label: &'a str,
    pub style: Style,
    /// Optional htmx target/swap directives forwarded through to the underlying request.
    pub target: Option<&'a str>,
    pub swap: Option<&'a str>,
}

/// Render the trigger button. Place it anywhere in a page; the modal partial
/// in `layout.html` does the rest.
#[must_use]
#[allow(clippy::needless_pass_by_value)]
pub fn confirm_button(c: Confirm<'_>) -> String {
    let (hx_attr, _) = c.action.hx_attr();
    let url = c.action.url();
    let target = c
        .target
        .map(|t| format!(r#" hx-target="{}""#, escape_html(t)))
        .unwrap_or_default();
    let swap = c
        .swap
        .map(|s| format!(r#" hx-swap="{}""#, escape_html(s)))
        .unwrap_or_default();

    format!(
        r#"<button type="button" class="{cls}"
  {hx}="{url}"
  hx-confirm="{title}"
  {target}{swap}
  data-confirm-action="{hx}"
  data-confirm-url="{url}"
  data-confirm-title="{title}"
  data-confirm-body="{body}"
  data-confirm-confirm-label="{confirm}"
  data-confirm-style="{style}">{label}</button>"#,
        cls = c.style.trigger_btn_class(),
        hx = hx_attr,
        url = escape_html(url),
        title = escape_html(c.title),
        body = escape_html(c.body),
        confirm = escape_html(c.confirm_label),
        style = c.style.as_str(),
        label = escape_html(c.label),
        target = target,
        swap = swap,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_both_hx_and_data_attrs() {
        let html = confirm_button(Confirm {
            label: "Wipe",
            action: Action::Post("/admin/wipe"),
            title: "Wipe recordings",
            body: "Deletes every clip.",
            confirm_label: "Wipe",
            style: Style::Danger,
            target: None,
            swap: None,
        });
        assert!(html.contains(r#"hx-post="/admin/wipe""#));
        assert!(html.contains(r#"data-confirm-action="hx-post""#));
        assert!(html.contains(r#"data-confirm-style="danger""#));
        // Native fallback gates on hx-confirm too.
        assert!(html.contains(r#"hx-confirm="Wipe recordings""#));
        assert!(html.contains("bnb-btn danger"));
    }

    #[test]
    fn html_escapes_attributes() {
        let html = confirm_button(Confirm {
            label: "X",
            action: Action::Delete("/x?a=\"b\""),
            title: "<title>",
            body: "A & B",
            confirm_label: "ok",
            style: Style::Moss,
            target: None,
            swap: None,
        });
        assert!(html.contains("&lt;title&gt;"));
        assert!(html.contains("A &amp; B"));
        assert!(html.contains("a=&quot;b&quot;"));
    }
}
