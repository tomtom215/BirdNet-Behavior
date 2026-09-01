//! Scheme-aware parser for Apprise-syntax notification URLs.
//!
//! # Why not a URL crate
//!
//! Several Apprise schemes are not valid URLs. The Telegram form is
//! `tgram://123456789:ABCdef_ghi/12315544/` — a generic parser reads
//! `123456789:ABCdef_ghi` as `host:port` and rejects the port, or worse,
//! silently truncates the bot token. Slack puts `#channel` in the path, which
//! a generic parser takes as a fragment. So the authority is split by hand:
//! userinfo at the last `@` before the first `/`, then `/`-separated segments.
//!
//! # Secrecy
//!
//! Every URL handled here contains a credential in its path. [`ParseError`]
//! therefore carries only the scheme name and a static description of what was
//! wrong — never a fragment of the input. The gate
//! `a_parse_error_never_quotes_the_url` holds this.

use std::fmt;

/// How to authenticate to an ntfy server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NtfyAuth {
    /// `ntfy://{user}:{password}@{host}/{topic}`.
    Basic {
        /// Username.
        user: String,
        /// Password.
        password: String,
    },
    /// `ntfy://{token}@{host}/{topic}` — an ntfy access token (`tk_...`).
    Token(String),
}

/// Which Slack API a `slack://` URL addresses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlackAuth {
    /// Legacy incoming webhook: `slack://{tokenA}/{tokenB}/{tokenC}`.
    Webhook {
        /// First path token (`T...`).
        token_a: String,
        /// Second path token (`B...`).
        token_b: String,
        /// Third path token.
        token_c: String,
    },
    /// Bot OAuth token: `slack://{xoxb-...}/{channel}`.
    Bot(String),
}

/// A notification destination this crate can deliver to without Apprise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// Discord incoming webhook.
    Discord {
        /// Webhook ID (first path segment).
        webhook_id: String,
        /// Webhook token (second path segment).
        webhook_token: String,
        /// Override for the displayed author name.
        username: Option<String>,
    },
    /// Slack, via either an incoming webhook or `chat.postMessage`.
    Slack {
        /// Credential and API selection.
        auth: SlackAuth,
        /// Target channel (`#general`, `C0123`). Required for [`SlackAuth::Bot`].
        channel: Option<String>,
        /// Override for the displayed author name.
        username: Option<String>,
    },
    /// Telegram bot API.
    Telegram {
        /// `{bot_id}:{secret}` bot token.
        bot_token: String,
        /// One or more chat IDs to deliver to.
        chat_ids: Vec<String>,
    },
    /// ntfy publish endpoint.
    Ntfy {
        /// Origin to publish against, e.g. `https://ntfy.sh`.
        origin: String,
        /// One or more topics.
        topics: Vec<String>,
        /// Optional credential.
        auth: Option<NtfyAuth>,
    },
    /// Gotify server.
    Gotify {
        /// Origin including any base path, e.g. `https://gotify.example.com`.
        origin: String,
        /// Application token.
        token: String,
    },
    /// Pushover.
    Pushover {
        /// User or group key.
        user_key: String,
        /// Application API token.
        token: String,
        /// Optional device names to target.
        devices: Vec<String>,
    },
    /// Generic JSON webhook (`json://` / `jsons://`).
    Json {
        /// Fully-qualified endpoint the payload is `POST`ed to.
        endpoint: String,
        /// Optional HTTP basic credentials.
        basic: Option<(String, String)>,
    },
}

impl Target {
    /// The scheme family this target was parsed from, for logs and metrics.
    ///
    /// Safe to log: it names a service, never a credential.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Discord { .. } => "discord",
            Self::Slack { .. } => "slack",
            Self::Telegram { .. } => "telegram",
            Self::Ntfy { .. } => "ntfy",
            Self::Gotify { .. } => "gotify",
            Self::Pushover { .. } => "pushover",
            Self::Json { .. } => "json",
        }
    }
}

/// Why a notification URL could not be parsed.
///
/// No variant carries any part of the input except the scheme name, which is
/// the one component that is never a credential.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// The string was empty or had no `scheme://` prefix.
    NotAUrl,
    /// The scheme is not one this crate delivers natively.
    ///
    /// The caller falls back to Apprise for these rather than failing.
    UnsupportedScheme(String),
    /// The scheme was recognised but a required component was absent.
    Missing {
        /// Scheme name, e.g. `"tgram"`.
        scheme: &'static str,
        /// What was expected, e.g. `"a chat id"`.
        what: &'static str,
    },
    /// The scheme was recognised but a component was not usable.
    Malformed {
        /// Scheme name, e.g. `"tgram"`.
        scheme: &'static str,
        /// What was wrong, e.g. `"bot token must be {id}:{secret}"`.
        what: &'static str,
    },
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAUrl => write!(f, "not a scheme://... URL"),
            Self::UnsupportedScheme(s) => write!(f, "no native sender for scheme {s}://"),
            Self::Missing { scheme, what } => write!(f, "{scheme}:// URL is missing {what}"),
            Self::Malformed { scheme, what } => write!(f, "{scheme}:// URL has {what}"),
        }
    }
}

impl std::error::Error for ParseError {}

/// A notification URL taken apart, before any scheme-specific meaning.
#[derive(Debug)]
struct Parts<'a> {
    /// Lowercased scheme, `://` stripped.
    scheme: String,
    /// Userinfo before `:`, if an `@` appeared in the authority.
    user: Option<&'a str>,
    /// Userinfo after `:`, if it contained one.
    password: Option<&'a str>,
    /// Authority host plus path, split on `/`, empties dropped.
    segments: Vec<&'a str>,
    /// Query keys seen after `?`. Values are dropped: they may be secrets and
    /// nothing here consumes them yet.
    query_keys: Vec<String>,
}

/// Split `scheme://[user[:pass]@]seg/seg/...[?query]` into its pieces.
fn split(url: &str) -> Result<Parts<'_>, ParseError> {
    let (scheme, rest) = url.split_once("://").ok_or(ParseError::NotAUrl)?;
    if scheme.is_empty() {
        return Err(ParseError::NotAUrl);
    }

    // `#` is *not* a fragment delimiter here: Slack channels are written
    // `slack://a/b/c/#general` and the `#` is part of the path segment.
    let (rest, query) = rest.split_once('?').map_or((rest, ""), |(r, q)| (r, q));
    let query_keys = query
        .split('&')
        .filter(|kv| !kv.is_empty())
        .map(|kv| {
            kv.split_once('=')
                .map_or(kv, |(k, _)| k)
                .to_ascii_lowercase()
        })
        .collect();

    // Userinfo is delimited by the last `@` *within the authority*, so a `@`
    // later in the path (a Slack channel, an email-shaped topic) is left alone.
    let authority_end = rest.find('/').unwrap_or(rest.len());
    let (user, password, rest) =
        rest[..authority_end]
            .rfind('@')
            .map_or((None, None, rest), |at| {
                let (info, _) = rest.split_at(at);
                let after = &rest[at + 1..];
                let (u, p) = info
                    .split_once(':')
                    .map_or((info, None), |(u, p)| (u, Some(p)));
                (Some(u), p, after)
            });

    Ok(Parts {
        scheme: scheme.to_ascii_lowercase(),
        user: user.filter(|u| !u.is_empty()),
        password,
        segments: rest.split('/').filter(|s| !s.is_empty()).collect(),
        query_keys,
    })
}

/// Parse an Apprise-syntax notification URL into a native [`Target`].
///
/// Recognises `discord`, `slack`, `tgram`, `ntfy`/`ntfys`, `gotify`/`gotifys`,
/// `pover`/`pushover`, and `json`/`jsons`. Every other scheme returns
/// [`ParseError::UnsupportedScheme`], which callers treat as "hand this one to
/// Apprise" rather than as a failure.
///
/// # Errors
///
/// Returns [`ParseError`] if the string is not a URL, names a scheme with no
/// native sender, or is missing a component that scheme requires. The error
/// never quotes the input.
pub fn parse(url: &str) -> Result<Target, ParseError> {
    let p = split(url.trim())?;
    let target = match p.scheme.as_str() {
        "discord" => discord(&p)?,
        "slack" => slack(&p)?,
        "tgram" | "telegram" => telegram(&p)?,
        "ntfy" | "ntfys" => ntfy(&p)?,
        "gotify" | "gotifys" => gotify(&p)?,
        "pover" | "pushover" => pushover(&p)?,
        "json" | "jsons" => json(&p)?,
        other => return Err(ParseError::UnsupportedScheme(other.to_string())),
    };
    if !p.query_keys.is_empty() {
        // Named, not valued: an Apprise query can carry a credential
        // (`?token=`), and this line goes to the operator's log.
        tracing::debug!(
            scheme = %p.scheme,
            options = %p.query_keys.join(","),
            "ignoring URL options the native sender does not implement"
        );
    }
    Ok(target)
}

/// `discord://{id}/{token}/` or `discord://{botname}@{id}/{token}/`.
fn discord(p: &Parts<'_>) -> Result<Target, ParseError> {
    let [id, token, ..] = p.segments.as_slice() else {
        return Err(ParseError::Missing {
            scheme: "discord",
            what: "a webhook id and token",
        });
    };
    Ok(Target::Discord {
        webhook_id: (*id).to_string(),
        webhook_token: (*token).to_string(),
        username: p.user.map(ToString::to_string),
    })
}

/// `slack://{a}/{b}/{c}[/#{channel}]` or `slack://{xoxb-token}/{channel}`.
///
/// The two forms are told apart by the `xox` prefix Slack puts on every bot,
/// user and app token; a webhook's first token starts `T`.
fn slack(p: &Parts<'_>) -> Result<Target, ParseError> {
    let first = p.segments.first().ok_or(ParseError::Missing {
        scheme: "slack",
        what: "a token",
    })?;

    if first.starts_with("xox") {
        let channel = p.segments.get(1).ok_or(ParseError::Missing {
            scheme: "slack",
            what: "a channel (an OAuth token cannot post without one)",
        })?;
        return Ok(Target::Slack {
            auth: SlackAuth::Bot((*first).to_string()),
            channel: Some((*channel).to_string()),
            username: p.user.map(ToString::to_string),
        });
    }

    let [a, b, c, rest @ ..] = p.segments.as_slice() else {
        return Err(ParseError::Missing {
            scheme: "slack",
            what: "three webhook tokens",
        });
    };
    Ok(Target::Slack {
        auth: SlackAuth::Webhook {
            token_a: (*a).to_string(),
            token_b: (*b).to_string(),
            token_c: (*c).to_string(),
        },
        channel: rest.first().map(|c| (*c).to_string()),
        username: p.user.map(ToString::to_string),
    })
}

/// `tgram://{bot_token}/{chat_id}[/{chat_id}...]`.
fn telegram(p: &Parts<'_>) -> Result<Target, ParseError> {
    let [token, chats @ ..] = p.segments.as_slice() else {
        return Err(ParseError::Missing {
            scheme: "tgram",
            what: "a bot token",
        });
    };
    // A bot token is `{numeric id}:{secret}`. Rejecting a token without the
    // colon catches the common mistake of pasting only the secret half, which
    // would otherwise fail much later as an opaque 404 from Telegram.
    if !token.contains(':') {
        return Err(ParseError::Malformed {
            scheme: "tgram",
            what: "a bot token that is not {id}:{secret}",
        });
    }
    if chats.is_empty() {
        return Err(ParseError::Missing {
            scheme: "tgram",
            what: "at least one chat id",
        });
    }
    Ok(Target::Telegram {
        bot_token: (*token).to_string(),
        chat_ids: chats.iter().map(|c| (*c).to_string()).collect(),
    })
}

/// ntfy's public host, used when the URL names only a topic.
const NTFY_CLOUD: &str = "https://ntfy.sh";

/// `ntfy://{topic}`, `ntfy://{host}/{topic}...`, with optional userinfo.
///
/// One segment means the cloud service (`ntfy://mytopic`); two or more mean
/// the first is a self-hosted host and the rest are topics. This is the
/// documented Apprise disambiguation and there is no way to have both.
fn ntfy(p: &Parts<'_>) -> Result<Target, ParseError> {
    let secure = p.scheme == "ntfys";
    let auth = match (p.user, p.password) {
        (Some(u), Some(pw)) => Some(NtfyAuth::Basic {
            user: u.to_string(),
            password: pw.to_string(),
        }),
        (Some(t), None) => Some(NtfyAuth::Token(t.to_string())),
        (None, _) => None,
    };

    let (origin, topics) = match p.segments.as_slice() {
        [] => {
            return Err(ParseError::Missing {
                scheme: "ntfy",
                what: "a topic",
            });
        }
        [topic] => (NTFY_CLOUD.to_string(), vec![(*topic).to_string()]),
        [host, rest @ ..] => (
            format!("{}://{host}", if secure { "https" } else { "http" }),
            rest.iter().map(|t| (*t).to_string()).collect(),
        ),
    };

    Ok(Target::Ntfy {
        origin,
        topics,
        auth,
    })
}

/// `gotify://{host}[:{port}][/{path}]/{token}`.
fn gotify(p: &Parts<'_>) -> Result<Target, ParseError> {
    let secure = p.scheme == "gotifys";
    let [host, rest @ ..] = p.segments.as_slice() else {
        return Err(ParseError::Missing {
            scheme: "gotify",
            what: "a hostname",
        });
    };
    let Some((token, path)) = rest.split_last() else {
        return Err(ParseError::Missing {
            scheme: "gotify",
            what: "an application token",
        });
    };
    let mut origin = format!("{}://{host}", if secure { "https" } else { "http" });
    for seg in path {
        origin.push('/');
        origin.push_str(seg);
    }
    Ok(Target::Gotify {
        origin,
        token: (*token).to_string(),
    })
}

/// `pover://{user_key}@{token}[/{device}...]`.
fn pushover(p: &Parts<'_>) -> Result<Target, ParseError> {
    let user_key = p.user.ok_or(ParseError::Missing {
        scheme: "pover",
        what: "a user key before the '@'",
    })?;
    let [token, devices @ ..] = p.segments.as_slice() else {
        return Err(ParseError::Missing {
            scheme: "pover",
            what: "an application token",
        });
    };
    Ok(Target::Pushover {
        user_key: user_key.to_string(),
        token: (*token).to_string(),
        devices: devices.iter().map(|d| (*d).to_string()).collect(),
    })
}

/// `json://{host}[/{path}]` — Apprise's generic JSON POST.
fn json(p: &Parts<'_>) -> Result<Target, ParseError> {
    if p.segments.is_empty() {
        return Err(ParseError::Missing {
            scheme: "json",
            what: "a hostname",
        });
    }
    let scheme = if p.scheme == "jsons" { "https" } else { "http" };
    let endpoint = format!("{scheme}://{}", p.segments.join("/"));
    let basic = match (p.user, p.password) {
        (Some(u), Some(pw)) => Some((u.to_string(), pw.to_string())),
        _ => None,
    };
    Ok(Target::Json { endpoint, basic })
}
