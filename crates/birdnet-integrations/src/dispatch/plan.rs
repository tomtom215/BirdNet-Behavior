//! What request each service needs, worked out without making it.
//!
//! Five of the seven services live at a fixed host (`discord.com`,
//! `api.telegram.org`, `hooks.slack.com`, `slack.com`, `api.pushover.net`), so
//! a test cannot point them at a local stub. Building the request as data
//! first makes the part that actually matters — the URL the credential is
//! spliced into, the payload shape, which header carries the token — assertable
//! without a network, and leaves [`super::send()`] as a thin executor.
//!
//! (`send` is both a module and the function re-exported from it, so the link
//! needs the `()` to say which.)

use serde_json::{Value, json};

use super::parse::{NtfyAuth, SlackAuth, Target};

/// Credential to attach to a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Auth {
    /// `Authorization: Bearer <token>`.
    Bearer(String),
    /// `Authorization: Basic <base64(user:pass)>`.
    Basic {
        /// Username.
        user: String,
        /// Password, if any.
        password: Option<String>,
    },
}

/// Request body encoding.
// Not `Eq`: `serde_json::Value` holds `f64`, which is only `PartialEq`.
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Debug, Clone, PartialEq)]
pub enum Body {
    /// `application/json`.
    Json(Value),
    /// `application/x-www-form-urlencoded`.
    Form(Vec<(&'static str, String)>),
}

/// How to tell success from failure in the response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expect {
    /// A 2xx status is enough.
    Status,
    /// A 2xx status *and* a truthy `ok` in the body — Slack's Web API and
    /// Telegram both answer `200 OK` with `{"ok": false, "error": ...}` when
    /// they refuse a message.
    OkField,
}

/// One HTTP request to make.
// Not `Eq`: see [`Body`].
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    /// Service name, for errors and metrics. Never a credential.
    pub kind: &'static str,
    /// Fully-qualified URL. For Discord, Slack webhooks and Telegram this
    /// contains the credential, so it must never be logged.
    pub url: String,
    /// Payload.
    pub body: Body,
    /// Optional `Authorization` header.
    pub auth: Option<Auth>,
    /// Extra headers, e.g. Gotify's `X-Gotify-Key`.
    pub headers: Vec<(&'static str, String)>,
    /// Success criterion.
    pub expect: Expect,
}

/// Severity, mapped onto whatever each service calls it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// Routine notice — a detection.
    Info,
    /// Something needs attention.
    Warning,
    /// A recovery or a completed job.
    Success,
}

impl Severity {
    /// Lowercase name, as the generic JSON webhook and Apprise both spell it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Success => "success",
        }
    }
}

/// What to deliver.
#[derive(Debug, Clone)]
pub struct Message {
    /// Short headline.
    pub title: String,
    /// Message body.
    pub body: String,
    /// Severity.
    pub severity: Severity,
    /// Optional image to attach or link.
    pub image_url: Option<String>,
}

impl Message {
    /// A message with no image, at [`Severity::Info`].
    #[must_use]
    pub fn new(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
            severity: Severity::Info,
            image_url: None,
        }
    }

    /// Title and body as one string, for services with no title field.
    fn flattened(&self) -> String {
        let mut s = if self.title.is_empty() {
            self.body.clone()
        } else {
            format!("{}\n{}", self.title, self.body)
        };
        if let Some(img) = &self.image_url {
            s.push('\n');
            s.push_str(img);
        }
        s
    }

    /// Body with any image URL appended, for services that have their own
    /// title field but no attachment field.
    fn body_with_image(&self) -> String {
        self.image_url
            .as_ref()
            .map_or_else(|| self.body.clone(), |img| format!("{}\n{img}", self.body))
    }
}

/// Every request needed to deliver `msg` to `target`.
///
/// More than one when the URL named several topics or chats; the caller
/// attempts all of them so a single stale chat id does not silence the rest.
#[must_use]
pub fn plans(target: &Target, msg: &Message) -> Vec<Plan> {
    match target {
        Target::Discord {
            webhook_id,
            webhook_token,
            username,
        } => vec![discord(webhook_id, webhook_token, username.as_deref(), msg)],
        Target::Slack {
            auth,
            channel,
            username,
        } => vec![slack(auth, channel.as_deref(), username.as_deref(), msg)],
        Target::Telegram {
            bot_token,
            chat_ids,
        } => chat_ids
            .iter()
            .map(|chat| telegram(bot_token, chat, msg))
            .collect(),
        Target::Ntfy {
            origin,
            topics,
            auth,
        } => topics
            .iter()
            .map(|topic| ntfy(origin, topic, auth.as_ref(), msg))
            .collect(),
        Target::Gotify { origin, token } => vec![gotify(origin, token, msg)],
        Target::Pushover {
            user_key,
            token,
            devices,
        } => vec![pushover(user_key, token, devices, msg)],
        Target::Json { endpoint, basic } => vec![json_webhook(endpoint, basic.as_ref(), msg)],
    }
}

/// `POST https://discord.com/api/webhooks/{id}/{token}`.
fn discord(id: &str, token: &str, username: Option<&str>, msg: &Message) -> Plan {
    let content = if msg.title.is_empty() {
        msg.body.clone()
    } else {
        format!("**{}**\n{}", msg.title, msg.body)
    };
    let mut payload = json!({ "content": content });
    if let Some(name) = username {
        payload["username"] = json!(name);
    }
    if let Some(img) = &msg.image_url {
        // Discord renders a bare URL in `content` as a link, but an embed with
        // an `image.url` renders the picture inline.
        payload["embeds"] = json!([{ "image": { "url": img } }]);
    }
    Plan {
        kind: "discord",
        url: format!("https://discord.com/api/webhooks/{id}/{token}"),
        body: Body::Json(payload),
        auth: None,
        headers: Vec::new(),
        expect: Expect::Status,
    }
}

/// Legacy incoming webhook, or `chat.postMessage` for an `xox*` token.
fn slack(auth: &SlackAuth, channel: Option<&str>, username: Option<&str>, msg: &Message) -> Plan {
    let text = if msg.title.is_empty() {
        msg.flattened()
    } else {
        let mut t = format!("*{}*\n{}", msg.title, msg.body);
        if let Some(img) = &msg.image_url {
            t.push('\n');
            t.push_str(img);
        }
        t
    };

    match auth {
        SlackAuth::Webhook {
            token_a,
            token_b,
            token_c,
        } => {
            let mut payload = json!({ "text": text });
            if let Some(ch) = channel {
                payload["channel"] = json!(ch);
            }
            if let Some(name) = username {
                payload["username"] = json!(name);
            }
            Plan {
                kind: "slack",
                url: format!("https://hooks.slack.com/services/{token_a}/{token_b}/{token_c}"),
                body: Body::Json(payload),
                auth: None,
                headers: Vec::new(),
                expect: Expect::OkField,
            }
        }
        SlackAuth::Bot(token) => Plan {
            kind: "slack",
            url: "https://slack.com/api/chat.postMessage".to_string(),
            body: Body::Json(json!({
                "channel": channel.unwrap_or_default(),
                "text": text,
            })),
            auth: Some(Auth::Bearer(token.clone())),
            headers: Vec::new(),
            expect: Expect::OkField,
        },
    }
}

/// `POST https://api.telegram.org/bot{token}/sendMessage`.
fn telegram(bot_token: &str, chat_id: &str, msg: &Message) -> Plan {
    Plan {
        kind: "telegram",
        url: format!("https://api.telegram.org/bot{bot_token}/sendMessage"),
        body: Body::Json(json!({ "chat_id": chat_id, "text": msg.flattened() })),
        auth: None,
        headers: Vec::new(),
        expect: Expect::OkField,
    }
}

/// ntfy's JSON publish form: `POST {origin}` with the topic in the body.
///
/// The header form (`POST {origin}/{topic}` with a `Title:` header) cannot
/// carry a non-ASCII title, and bird common names have accents in most of the
/// 36 languages this station's species labels can be set to.
fn ntfy(origin: &str, topic: &str, auth: Option<&NtfyAuth>, msg: &Message) -> Plan {
    let mut payload = json!({
        "topic": topic,
        "title": msg.title,
        "message": msg.body,
    });
    if let Some(img) = &msg.image_url {
        payload["attach"] = json!(img);
    }
    Plan {
        kind: "ntfy",
        url: origin.to_string(),
        body: Body::Json(payload),
        auth: auth.map(|a| match a {
            NtfyAuth::Basic { user, password } => Auth::Basic {
                user: user.clone(),
                password: Some(password.clone()),
            },
            NtfyAuth::Token(t) => Auth::Bearer(t.clone()),
        }),
        headers: Vec::new(),
        expect: Expect::Status,
    }
}

/// `POST {origin}/message` with the token in `X-Gotify-Key`.
///
/// Gotify also accepts `?token=`, but a query string is the part of a URL most
/// likely to end up in a reverse proxy's access log.
fn gotify(origin: &str, token: &str, msg: &Message) -> Plan {
    Plan {
        kind: "gotify",
        url: format!("{origin}/message"),
        body: Body::Json(json!({
            "title": msg.title,
            "message": msg.body_with_image(),
        })),
        auth: None,
        headers: vec![("X-Gotify-Key", token.to_string())],
        expect: Expect::Status,
    }
}

/// `POST https://api.pushover.net/1/messages.json`, form-encoded.
fn pushover(user_key: &str, token: &str, devices: &[String], msg: &Message) -> Plan {
    let mut form = vec![
        ("token", token.to_string()),
        ("user", user_key.to_string()),
        ("title", msg.title.clone()),
        ("message", msg.body_with_image()),
    ];
    if !devices.is_empty() {
        form.push(("device", devices.join(",")));
    }
    Plan {
        kind: "pushover",
        url: "https://api.pushover.net/1/messages.json".to_string(),
        body: Body::Form(form),
        auth: None,
        headers: Vec::new(),
        expect: Expect::Status,
    }
}

/// Apprise's generic `json://` sink: POST the message as a JSON object.
fn json_webhook(endpoint: &str, basic: Option<&(String, String)>, msg: &Message) -> Plan {
    Plan {
        kind: "json",
        url: endpoint.to_string(),
        body: Body::Json(json!({
            "title": msg.title,
            "message": msg.body,
            "type": msg.severity.as_str(),
            "image": msg.image_url,
        })),
        auth: basic.map(|(user, password)| Auth::Basic {
            user: user.clone(),
            password: Some(password.clone()),
        }),
        headers: Vec::new(),
        expect: Expect::Status,
    }
}

impl From<crate::apprise::NotifyType> for Severity {
    fn from(t: crate::apprise::NotifyType) -> Self {
        match t {
            crate::apprise::NotifyType::Info => Self::Info,
            crate::apprise::NotifyType::Warning => Self::Warning,
            crate::apprise::NotifyType::Success => Self::Success,
        }
    }
}
