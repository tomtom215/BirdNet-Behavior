//! What each Apprise URL form is understood to mean.
//!
//! Every URL here is in the syntax Apprise documents, with fabricated
//! credentials. The point of the file is that the *discrimination* is right:
//! for every "this parses to X" there is a counterpart showing a near-miss
//! does not, so an accept-everything parser would not pass.

use birdnet_integrations::dispatch::{NtfyAuth, ParseError, SlackAuth, Target, parse};

// ---------------------------------------------------------------------------
// Telegram — the reason this is a hand parser and not a URL crate
// ---------------------------------------------------------------------------

#[test]
fn a_telegram_bot_token_keeps_its_colon() {
    // A generic URL parser reads `123456789:AAE_secret` as host:port and either
    // rejects it or keeps only `123456789`. Losing the half after the colon
    // would produce a token that authenticates as nobody.
    let t = parse("tgram://123456789:AAE_secret/12315544/").unwrap();
    assert_eq!(
        t,
        Target::Telegram {
            bot_token: "123456789:AAE_secret".to_string(),
            chat_ids: vec!["12315544".to_string()],
        }
    );
}

#[test]
fn telegram_accepts_several_chat_ids() {
    let Target::Telegram { chat_ids, .. } = parse("tgram://1:s/aaa/bbb/ccc/").unwrap() else {
        panic!("not a telegram target");
    };
    assert_eq!(chat_ids, ["aaa", "bbb", "ccc"]);
}

#[test]
fn a_telegram_url_without_a_chat_id_is_rejected() {
    // Counterpart to the two above: the token alone cannot deliver anywhere,
    // and Telegram's own answer to that is an opaque 400 much later.
    assert_eq!(
        parse("tgram://123456789:AAE_secret/"),
        Err(ParseError::Missing {
            scheme: "tgram",
            what: "at least one chat id",
        })
    );
}

#[test]
fn a_telegram_token_missing_its_id_half_is_rejected() {
    // Pasting only the secret half of the token is the common mistake.
    assert_eq!(
        parse("tgram://AAE_secret_only/12315544/"),
        Err(ParseError::Malformed {
            scheme: "tgram",
            what: "a bot token that is not {id}:{secret}",
        })
    );
}

// ---------------------------------------------------------------------------
// Discord
// ---------------------------------------------------------------------------

#[test]
fn discord_reads_the_webhook_id_and_token_in_order() {
    assert_eq!(
        parse("discord://1234567890/abcdefTOKEN/").unwrap(),
        Target::Discord {
            webhook_id: "1234567890".to_string(),
            webhook_token: "abcdefTOKEN".to_string(),
            username: None,
        }
    );
}

#[test]
fn discord_takes_a_bot_name_from_the_userinfo() {
    let Target::Discord { username, .. } = parse("discord://Nest@123/tok/").unwrap() else {
        panic!("not a discord target");
    };
    assert_eq!(username.as_deref(), Some("Nest"));
}

#[test]
fn a_discord_url_with_only_an_id_is_rejected() {
    assert!(matches!(
        parse("discord://1234567890/"),
        Err(ParseError::Missing {
            scheme: "discord",
            ..
        })
    ));
}

// ---------------------------------------------------------------------------
// Slack — two credential shapes behind one scheme
// ---------------------------------------------------------------------------

#[test]
fn slack_three_tokens_mean_a_legacy_incoming_webhook() {
    assert_eq!(
        parse("slack://T00000000/B00000000/XXXXXXXXXXXXXXXXXXXXXXXX").unwrap(),
        Target::Slack {
            auth: SlackAuth::Webhook {
                token_a: "T00000000".to_string(),
                token_b: "B00000000".to_string(),
                token_c: "XXXXXXXXXXXXXXXXXXXXXXXX".to_string(),
            },
            channel: None,
            username: None,
        }
    );
}

#[test]
fn a_slack_webhook_can_name_a_channel() {
    let Target::Slack { channel, .. } = parse("slack://T00000000/B00000000/XXXX/#birds").unwrap()
    else {
        panic!("not a slack target");
    };
    // `#` must not be treated as a URL fragment: the channel lives in the path.
    assert_eq!(channel.as_deref(), Some("#birds"));
}

#[test]
fn an_xox_token_means_the_web_api_instead() {
    // Counterpart to the webhook case: same scheme, different credential
    // shape, and it must not be mistaken for the first of three tokens.
    assert_eq!(
        parse("slack://xoxb-1111-2222-abcdef/#birds").unwrap(),
        Target::Slack {
            auth: SlackAuth::Bot("xoxb-1111-2222-abcdef".to_string()),
            channel: Some("#birds".to_string()),
            username: None,
        }
    );
}

#[test]
fn an_oauth_token_without_a_channel_is_rejected() {
    // `chat.postMessage` has nowhere to post without one.
    assert!(matches!(
        parse("slack://xoxb-1111-2222-abcdef/"),
        Err(ParseError::Missing {
            scheme: "slack",
            ..
        })
    ));
}

#[test]
fn two_webhook_tokens_are_not_enough() {
    assert!(matches!(
        parse("slack://T00000000/B00000000"),
        Err(ParseError::Missing {
            scheme: "slack",
            ..
        })
    ));
}

// ---------------------------------------------------------------------------
// ntfy — one segment is a cloud topic, two or more is a self-hosted server
// ---------------------------------------------------------------------------

#[test]
fn a_lone_ntfy_segment_is_a_topic_on_the_public_service() {
    assert_eq!(
        parse("ntfy://mybirdtopic").unwrap(),
        Target::Ntfy {
            origin: "https://ntfy.sh".to_string(),
            topics: vec!["mybirdtopic".to_string()],
            auth: None,
        }
    );
}

#[test]
fn two_ntfy_segments_are_a_host_and_a_topic() {
    // Counterpart to the above: the same scheme, one more segment, and the
    // first segment changes meaning entirely.
    assert_eq!(
        parse("ntfy://ntfy.lan:8080/garden").unwrap(),
        Target::Ntfy {
            origin: "http://ntfy.lan:8080".to_string(),
            topics: vec!["garden".to_string()],
            auth: None,
        }
    );
}

#[test]
fn ntfys_upgrades_a_self_hosted_server_to_https() {
    let Target::Ntfy { origin, .. } = parse("ntfys://ntfy.example.org/garden").unwrap() else {
        panic!("not an ntfy target");
    };
    assert_eq!(origin, "https://ntfy.example.org");
}

#[test]
fn ntfy_userinfo_with_a_password_is_basic_auth() {
    let Target::Ntfy { auth, .. } = parse("ntfy://ada:hunter2@ntfy.lan/garden").unwrap() else {
        panic!("not an ntfy target");
    };
    assert_eq!(
        auth,
        Some(NtfyAuth::Basic {
            user: "ada".to_string(),
            password: "hunter2".to_string(),
        })
    );
}

#[test]
fn ntfy_userinfo_without_a_password_is_a_token() {
    // Counterpart: the presence of the colon is the whole discrimination.
    let Target::Ntfy { auth, .. } = parse("ntfy://tk_abc123@ntfy.lan/garden").unwrap() else {
        panic!("not an ntfy target");
    };
    assert_eq!(auth, Some(NtfyAuth::Token("tk_abc123".to_string())));
}

#[test]
fn ntfy_accepts_several_topics() {
    let Target::Ntfy { topics, .. } = parse("ntfy://ntfy.lan/garden/rare/owls").unwrap() else {
        panic!("not an ntfy target");
    };
    assert_eq!(topics, ["garden", "rare", "owls"]);
}

// ---------------------------------------------------------------------------
// Gotify
// ---------------------------------------------------------------------------

#[test]
fn gotify_takes_the_last_segment_as_the_token() {
    assert_eq!(
        parse("gotify://gotify.lan/AbCdEf12345").unwrap(),
        Target::Gotify {
            origin: "http://gotify.lan".to_string(),
            token: "AbCdEf12345".to_string(),
        }
    );
}

#[test]
fn gotify_keeps_a_base_path_out_of_the_token() {
    // Behind a reverse proxy Gotify often lives under a sub-path. The token is
    // still the last segment; everything between host and token is the path.
    assert_eq!(
        parse("gotifys://example.com:8443/gotify/AbCdEf12345").unwrap(),
        Target::Gotify {
            origin: "https://example.com:8443/gotify".to_string(),
            token: "AbCdEf12345".to_string(),
        }
    );
}

#[test]
fn a_gotify_url_with_only_a_host_is_rejected() {
    assert!(matches!(
        parse("gotify://gotify.lan"),
        Err(ParseError::Missing {
            scheme: "gotify",
            ..
        })
    ));
}

// ---------------------------------------------------------------------------
// Pushover
// ---------------------------------------------------------------------------

#[test]
fn pushover_reads_the_user_key_before_the_at_and_the_token_after() {
    assert_eq!(
        parse("pover://uQiRzpo4DXghDmr9QzzfQu27cmVRsG@azGDORePK8gMaC0QOYAMyEEuzJnyUi").unwrap(),
        Target::Pushover {
            user_key: "uQiRzpo4DXghDmr9QzzfQu27cmVRsG".to_string(),
            token: "azGDORePK8gMaC0QOYAMyEEuzJnyUi".to_string(),
            devices: Vec::new(),
        }
    );
}

#[test]
fn pushover_can_target_named_devices() {
    let Target::Pushover { devices, .. } = parse("pover://ukey@atoken/phone/tablet").unwrap()
    else {
        panic!("not a pushover target");
    };
    assert_eq!(devices, ["phone", "tablet"]);
}

#[test]
fn a_pushover_url_without_a_user_key_is_rejected() {
    // Counterpart: the `@` is what separates the two keys, and a URL with only
    // one of them would otherwise send with an empty `user` parameter.
    assert!(matches!(
        parse("pover://azGDORePK8gMaC0QOYAMyEEuzJnyUi"),
        Err(ParseError::Missing {
            scheme: "pover",
            ..
        })
    ));
}

// ---------------------------------------------------------------------------
// Generic JSON webhook
// ---------------------------------------------------------------------------

#[test]
fn json_builds_an_http_endpoint_and_jsons_an_https_one() {
    let Target::Json { endpoint, basic } = parse("json://hooks.lan/bird").unwrap() else {
        panic!("not a json target");
    };
    assert_eq!(endpoint, "http://hooks.lan/bird");
    assert_eq!(basic, None);

    let Target::Json { endpoint, .. } = parse("jsons://hooks.example.com/bird").unwrap() else {
        panic!("not a json target");
    };
    assert_eq!(endpoint, "https://hooks.example.com/bird");
}

#[test]
fn json_userinfo_becomes_basic_auth() {
    let Target::Json { basic, .. } = parse("jsons://ada:hunter2@hooks.example.com/b").unwrap()
    else {
        panic!("not a json target");
    };
    assert_eq!(basic, Some(("ada".to_string(), "hunter2".to_string())));
}

// ---------------------------------------------------------------------------
// Fallback and rejection
// ---------------------------------------------------------------------------

#[test]
fn an_unhandled_scheme_defers_rather_than_failing() {
    // Apprise supports ~80 services. The ones without a native sender must
    // come back as `UnsupportedScheme` — the caller routes those to Apprise —
    // and not as an error that would drop the notification.
    for url in [
        "mailto://user:pass@gmail.com",
        "matrix://user:pass@matrix.org/#room",
        "twilio://sid:token@from/to",
        "rocket://user:pass@host/#channel",
    ] {
        let scheme = url.split("://").next().unwrap();
        assert_eq!(
            parse(url),
            Err(ParseError::UnsupportedScheme(scheme.to_string())),
            "{scheme} should defer to Apprise"
        );
    }
}

#[test]
fn schemes_are_case_insensitive() {
    assert_eq!(parse("NTFY://topic").unwrap().kind(), "ntfy");
}

#[test]
fn something_that_is_not_a_url_is_rejected() {
    for not_a_url in ["", "   ", "just some words", "discord:/missing-slash"] {
        assert_eq!(parse(not_a_url), Err(ParseError::NotAUrl), "{not_a_url:?}");
    }
}

// ---------------------------------------------------------------------------
// The one that matters most
// ---------------------------------------------------------------------------

#[test]
fn a_parse_error_never_quotes_the_url() {
    // Parse errors are logged. Every URL in this file carries a credential in
    // its path, so an error that echoed its input would publish a working
    // webhook to the operator's journal — and to any support bundle.
    let secrets = [
        (
            "tgram://999888777:SUPERSECRETBOTTOKEN/",
            "SUPERSECRETBOTTOKEN",
        ),
        ("discord://SUPERSECRETWEBHOOK", "SUPERSECRETWEBHOOK"),
        ("slack://T1/SUPERSECRETSLACK", "SUPERSECRETSLACK"),
        ("gotify://SUPERSECRETHOSTONLY", "SUPERSECRETHOSTONLY"),
        ("pover://SUPERSECRETAPPTOKEN", "SUPERSECRETAPPTOKEN"),
        ("tgram://SUPERSECRETNOCOLON/12315544/", "SUPERSECRETNOCOLON"),
    ];
    for (url, secret) in secrets {
        let err = parse(url).expect_err("should not parse");
        let rendered = format!("{err} {err:?}");
        assert!(
            !rendered.contains(secret),
            "error for {url:?} leaked its input: {rendered}"
        );
    }
}

#[test]
fn an_unsupported_scheme_error_names_only_the_scheme() {
    // Counterpart to the gate above: `UnsupportedScheme` is the one variant
    // that carries a piece of the input, so pin exactly which piece.
    let err = parse("matrix://ada:SUPERSECRETPASSWORD@matrix.org/#birds").expect_err("unsupported");
    let rendered = format!("{err} {err:?}");
    assert!(rendered.contains("matrix"), "{rendered}");
    assert!(!rendered.contains("SUPERSECRETPASSWORD"), "{rendered}");
    assert!(!rendered.contains("matrix.org"), "{rendered}");
}
