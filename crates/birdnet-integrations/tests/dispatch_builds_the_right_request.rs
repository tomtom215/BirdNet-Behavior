//! What request each parsed target turns into.
//!
//! Five of the seven services live at a fixed host, so these assertions are on
//! the request *plan* rather than on traffic. What they pin is the part that
//! silently breaks: which URL the credential is spliced into, which key each
//! service expects the message under, and which header carries the token.

use birdnet_integrations::dispatch::{Auth, Body, Expect, Message, Severity, parse, plans};

/// A message with an image, so the attachment handling is exercised too.
fn msg() -> Message {
    Message {
        title: "Bird Detection: Tawny Owl".to_string(),
        body: "Strix aluco (91% confidence) at 03:14".to_string(),
        severity: Severity::Info,
        image_url: Some("https://example.org/owl.jpg".to_string()),
    }
}

/// The JSON body of the single plan a target produces.
fn only_json(url: &str) -> (String, serde_json::Value) {
    let target = parse(url).expect("parses");
    let mut p = plans(&target, &msg());
    assert_eq!(p.len(), 1, "expected exactly one request for {url}");
    let plan = p.remove(0);
    match plan.body {
        Body::Json(v) => (plan.url, v),
        Body::Form(_) => panic!("expected a JSON body"),
    }
}

#[test]
fn discord_posts_to_the_webhook_path_that_carries_its_credential() {
    let (url, body) = only_json("discord://1234567890/abcdefTOKEN/");
    assert_eq!(
        url,
        "https://discord.com/api/webhooks/1234567890/abcdefTOKEN"
    );
    assert_eq!(
        body["content"],
        "**Bird Detection: Tawny Owl**\nStrix aluco (91% confidence) at 03:14"
    );
    // A bare URL in `content` renders as a link; an embed renders the picture.
    assert_eq!(
        body["embeds"][0]["image"]["url"],
        "https://example.org/owl.jpg"
    );
}

#[test]
fn a_slack_webhook_posts_to_hooks_slack_com_with_no_authorization_header() {
    let target = parse("slack://T00000000/B00000000/XXXX/#birds").unwrap();
    let plan = plans(&target, &msg()).remove(0);
    assert_eq!(
        plan.url,
        "https://hooks.slack.com/services/T00000000/B00000000/XXXX"
    );
    assert_eq!(plan.auth, None, "a webhook authenticates by its URL alone");
    let Body::Json(body) = plan.body else {
        panic!("expected JSON")
    };
    assert_eq!(body["channel"], "#birds");
    assert!(body["text"].as_str().unwrap().contains("Strix aluco"));
}

#[test]
fn a_slack_bot_token_posts_to_the_web_api_as_a_bearer_credential() {
    // Counterpart to the webhook case: same scheme, entirely different
    // endpoint, and the token moves from the URL into a header.
    let target = parse("slack://xoxb-1111-2222-abcdef/#birds").unwrap();
    let plan = plans(&target, &msg()).remove(0);
    assert_eq!(plan.url, "https://slack.com/api/chat.postMessage");
    assert_eq!(
        plan.auth,
        Some(Auth::Bearer("xoxb-1111-2222-abcdef".to_string()))
    );
    assert!(
        !plan.url.contains("xoxb"),
        "the token must not be in the URL"
    );
}

#[test]
fn slack_is_judged_on_its_ok_field_not_its_status() {
    // Slack and Telegram answer `200 OK` with `{"ok": false, "error": ...}`
    // when they refuse a message, so a status-only check reports every such
    // failure as a successful delivery.
    for url in [
        "slack://T0/B0/XXXX",
        "slack://xoxb-1/#c",
        "tgram://1:s/12315544",
    ] {
        let target = parse(url).unwrap();
        assert_eq!(plans(&target, &msg())[0].expect, Expect::OkField, "{url}");
    }
    // ...and the services that do report failure by status are not.
    for url in [
        "discord://1/t",
        "ntfy://topic",
        "gotify://h/t",
        "json://h/p",
    ] {
        let target = parse(url).unwrap();
        assert_eq!(plans(&target, &msg())[0].expect, Expect::Status, "{url}");
    }
}

#[test]
fn telegram_makes_one_request_per_chat_against_the_bot_path() {
    let target = parse("tgram://123456789:AAE_secret/aaa/bbb/").unwrap();
    let plans = plans(&target, &msg());
    assert_eq!(plans.len(), 2, "one request per chat id");
    for p in &plans {
        assert_eq!(
            p.url,
            "https://api.telegram.org/bot123456789:AAE_secret/sendMessage"
        );
    }
    let chat_of = |i: usize| match &plans[i].body {
        Body::Json(v) => v["chat_id"].as_str().unwrap().to_string(),
        Body::Form(_) => panic!("expected JSON"),
    };
    assert_eq!(chat_of(0), "aaa");
    assert_eq!(chat_of(1), "bbb");
}

#[test]
fn ntfy_publishes_json_with_the_topic_in_the_body() {
    // The alternative — `POST {origin}/{topic}` with a `Title:` header — cannot
    // carry a non-ASCII title, and this station can label species in 36
    // languages. Posting to the origin with a JSON `topic` avoids that.
    let (url, body) = only_json("ntfy://ntfy.lan:8080/garden");
    assert_eq!(url, "http://ntfy.lan:8080");
    assert_eq!(body["topic"], "garden");
    assert_eq!(body["title"], "Bird Detection: Tawny Owl");
    assert_eq!(body["attach"], "https://example.org/owl.jpg");
}

#[test]
fn an_ntfy_title_survives_being_non_ascii() {
    // The concrete reason for the JSON publish form: HTTP header values are
    // Latin-1 at best, and "Grünspecht"/"Œdicnème criard" are ordinary
    // common names in the languages this station ships.
    let target = parse("ntfy://garden").unwrap();
    let m = Message {
        title: "Œdicnème criard".to_string(),
        body: "Burhinus oedicnemus".to_string(),
        severity: Severity::Info,
        image_url: None,
    };
    let Body::Json(body) = plans(&target, &m).remove(0).body else {
        panic!("expected JSON")
    };
    assert_eq!(body["title"], "Œdicnème criard");
}

#[test]
fn ntfy_credentials_become_the_matching_authorization_header() {
    let bearer = plans(&parse("ntfy://tk_abc@ntfy.lan/g").unwrap(), &msg()).remove(0);
    assert_eq!(bearer.auth, Some(Auth::Bearer("tk_abc".to_string())));

    let basic = plans(&parse("ntfy://ada:hunter2@ntfy.lan/g").unwrap(), &msg()).remove(0);
    assert_eq!(
        basic.auth,
        Some(Auth::Basic {
            user: "ada".to_string(),
            password: Some("hunter2".to_string()),
        })
    );
}

#[test]
fn gotify_carries_its_token_in_a_header_not_the_query_string() {
    // Gotify accepts `?token=`, but a query string is the part of a URL a
    // reverse proxy is most likely to write to its access log.
    let target = parse("gotifys://example.com:8443/gotify/AbCdEf12345").unwrap();
    let plan = plans(&target, &msg()).remove(0);
    assert_eq!(plan.url, "https://example.com:8443/gotify/message");
    assert!(
        !plan.url.contains("AbCdEf12345"),
        "token leaked into the URL"
    );
    assert_eq!(
        plan.headers,
        vec![("X-Gotify-Key", "AbCdEf12345".to_string())]
    );
}

#[test]
fn pushover_sends_form_parameters_with_both_keys() {
    let target = parse("pover://ukeyAAA@atokenBBB/phone/tablet").unwrap();
    let plan = plans(&target, &msg()).remove(0);
    assert_eq!(plan.url, "https://api.pushover.net/1/messages.json");
    let Body::Form(form) = plan.body else {
        panic!("Pushover takes url-encoded parameters, not JSON")
    };
    let get = |k: &str| {
        form.iter()
            .find(|(name, _)| *name == k)
            .map(|(_, v)| v.clone())
    };
    // `user` and `token` are different things and swapping them silently
    // fails: the application token goes in `token`, the recipient in `user`.
    assert_eq!(get("token").as_deref(), Some("atokenBBB"));
    assert_eq!(get("user").as_deref(), Some("ukeyAAA"));
    assert_eq!(get("device").as_deref(), Some("phone,tablet"));
    assert!(get("message").unwrap().contains("Strix aluco"));
}

#[test]
fn the_generic_json_webhook_reports_the_severity() {
    let target = parse("jsons://hooks.example.com/bird").unwrap();
    for (severity, expected) in [
        (Severity::Info, "info"),
        (Severity::Warning, "warning"),
        (Severity::Success, "success"),
    ] {
        let m = Message { severity, ..msg() };
        let Body::Json(body) = plans(&target, &m).remove(0).body else {
            panic!("expected JSON")
        };
        assert_eq!(body["type"], expected);
    }
}

#[test]
fn a_missing_image_is_never_rendered_as_the_word_none() {
    // `Option<String>` interpolated into a format string produces "None",
    // which would be posted to every channel as if it were a URL.
    let target_list = [
        "discord://1/t",
        "slack://T0/B0/X",
        "tgram://1:s/c",
        "ntfy://topic",
        "gotify://h/t",
        "pover://u@t",
        "json://h/p",
    ];
    let m = Message {
        image_url: None,
        ..msg()
    };
    for url in target_list {
        let target = parse(url).unwrap();
        for plan in plans(&target, &m) {
            let rendered = match &plan.body {
                Body::Json(v) => v.to_string(),
                Body::Form(kv) => format!("{kv:?}"),
            };
            assert!(
                !rendered.contains("None"),
                "{url} rendered an absent image: {rendered}"
            );
        }
    }
}
