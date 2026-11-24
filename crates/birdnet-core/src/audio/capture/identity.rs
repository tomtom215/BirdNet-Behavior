//! Which physical device a configured device string names (MSC-10).
//!
//! The Audio page refuses to add a source that is already configured, and it
//! used to decide that by comparing strings. Three spellings of one USB
//! microphone — `plughw:1,0`, `hw:1,0` and `plughw:CARD=USB,DEV=0` — were three
//! "different" sources, and so were `rtsp://cam/live` and
//! `rtsp://user:pw@cam:554/live/`. Opening `hw:` twice fails with `EBUSY` and
//! restart-loops; a shared device opened twice records the same audio twice
//! and doubles every detection.
//!
//! The functions here turn a device string into a **key**: two strings with
//! the same key name the same device. They are pure, so the rules can be
//! tested without hardware; the one fact that needs the machine — which card
//! index an ALSA card name sits at — is passed in as the text of
//! `/proc/asound/cards`.
//!
//! What a key cannot see it does not guess. A card name that the cards text
//! does not list stays a name, equal only to the same name; an ALSA string that
//! is not an `hw`/`plughw` device (`default`, `dsnoop:…`, a `PipeWire` node) is
//! its own key, compared exactly.

/// The key of an ALSA device string, given the text of `/proc/asound/cards`.
///
/// `hw:` and `plughw:` open the same hardware (`plughw` only adds format
/// conversion), so both reduce to the card, device and subdevice they name:
///
/// * `hw:1,0`, `plughw:1,0`, `hw:1` (device 0 is ALSA's default) and
///   `hw:CARD=1,DEV=0` all key as card 1, device 0;
/// * `plughw:CARD=PRO,DEV=0` and `hw:PRO,0` key as card 1 when `cards` lists
///   ` 1 [PRO …]`, and as the name `PRO` when it does not — so an unplugged
///   microphone is still recognised as itself, and never as some other card.
///
/// Anything else keys as the trimmed string.
#[must_use]
pub fn alsa_device_key(device: &str, cards: &str) -> String {
    let trimmed = device.trim();
    let Some((plugin, args)) = trimmed.split_once(':') else {
        return format!("alsa-raw:{trimmed}");
    };
    if plugin != "hw" && plugin != "plughw" {
        return format!("alsa-raw:{trimmed}");
    }

    let mut card: Option<&str> = None;
    let mut dev: Option<&str> = None;
    let mut subdev: Option<&str> = None;
    for (position, field) in args.split(',').enumerate() {
        let field = field.trim();
        if let Some((name, value)) = field.split_once('=') {
            let value = value.trim();
            match name.trim() {
                "CARD" => card = Some(value),
                "DEV" => dev = Some(value),
                "SUBDEV" => subdev = Some(value),
                _ => return format!("alsa-raw:{trimmed}"),
            }
        } else {
            match position {
                0 => card = Some(field),
                1 => dev = Some(field),
                2 => subdev = Some(field),
                _ => return format!("alsa-raw:{trimmed}"),
            }
        }
    }

    let Some(card) = card.filter(|c| !c.is_empty()) else {
        return format!("alsa-raw:{trimmed}");
    };
    let number = |s: Option<&str>, default: i64| -> Option<i64> {
        match s {
            None | Some("") => Some(default),
            Some(v) => v.parse::<i64>().ok(),
        }
    };
    // ALSA's own defaults for an omitted device and subdevice (alsa.conf's
    // `pcm.hw` declares DEV default 0, SUBDEV default -1, "any").
    let (Some(dev), Some(subdev)) = (number(dev, 0), number(subdev, -1)) else {
        return format!("alsa-raw:{trimmed}");
    };

    let card = if card.chars().all(|c| c.is_ascii_digit()) {
        card.parse::<u32>()
            .map_or_else(|_| format!("name={card}"), |i| format!("index={i}"))
    } else {
        card_index_by_id(cards, card)
            .map_or_else(|| format!("name={card}"), |i| format!("index={i}"))
    };
    format!("alsa:{card},dev={dev},subdev={subdev}")
}

/// The index of the card whose id is `id`, from the text of
/// `/proc/asound/cards` (` 1 [PRO            ]: USB-Audio - …`). Card ids are
/// compared exactly, as alsa-lib compares them.
#[must_use]
pub fn card_index_by_id(cards: &str, id: &str) -> Option<u32> {
    cards.lines().find_map(|line| {
        let (head, _) = line.split_once("]:")?;
        let (num, name) = head.split_once('[')?;
        let index = num.trim().parse::<u32>().ok()?;
        (name.trim() == id).then_some(index)
    })
}

/// The key of a stream URL.
///
/// Two addresses for one camera differ in ways that do not change what is
/// opened: credentials (the same camera with and without `user:pass@`), the
/// letter case of the scheme and host (RFC 3986 §3.1, §3.2.2), a default port
/// written out (`:554` for RTSP), and a trailing `/` on the path. The key drops
/// or normalises each. The path and query are otherwise kept as written — they
/// are case-sensitive, and two paths are two streams.
#[must_use]
pub fn stream_url_key(url: &str) -> String {
    let trimmed = url.trim();
    let Some((scheme, rest)) = trimmed.split_once("://") else {
        return trimmed.to_owned();
    };
    let scheme = scheme.to_ascii_lowercase();
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(authority_end);
    let host_port = authority
        .rfind('@')
        .map_or(authority, |at| &authority[at + 1..]);

    // `[v6]:port` or `host:port`; a colon inside the brackets is not a port.
    let (host, port) = if host_port.starts_with('[') {
        host_port.find(']').map_or((host_port, None), |close| {
            (
                &host_port[..=close],
                host_port[close + 1..].strip_prefix(':'),
            )
        })
    } else {
        host_port
            .rsplit_once(':')
            .map_or((host_port, None), |(h, p)| (h, Some(p)))
    };
    let host = host.to_ascii_lowercase();
    let default_port = match scheme.as_str() {
        "rtsp" => Some("554"),
        "rtsps" => Some("322"),
        "http" => Some("80"),
        "https" => Some("443"),
        "rtmp" => Some("1935"),
        _ => None,
    };
    let port = port.filter(|p| !p.is_empty() && Some(*p) != default_port);

    let (path, query) = tail
        .find(['?', '#'])
        .map_or((tail, ""), |q| tail.split_at(q));
    let path = path.trim_end_matches('/');

    port.map_or_else(
        || format!("{scheme}://{host}{path}{query}"),
        |p| format!("{scheme}://{host}:{p}{path}{query}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const CARDS: &str = " 0 [Headphones     ]: bcm2835_headphonesn - bcm2835 Headphones\n\
                         \x20                     bcm2835 Headphones\n\
                         \x201 [PRO            ]: USB-Audio - AT2020USB+\n\
                         \x20                     Audio-Technica AT2020USB+ at usb-0000:01:00.0-1.2\n";

    #[test]
    fn hw_and_plughw_on_one_card_and_device_are_one_device() {
        let k = alsa_device_key("plughw:1,0", "");
        assert_eq!(alsa_device_key("hw:1,0", ""), k);
        assert_eq!(alsa_device_key(" hw:1 ", ""), k, "device 0 is the default");
        assert_eq!(alsa_device_key("hw:CARD=1,DEV=0", ""), k);
    }

    #[test]
    fn a_card_name_resolves_through_the_cards_text() {
        let by_index = alsa_device_key("hw:1,0", CARDS);
        assert_eq!(alsa_device_key("plughw:CARD=PRO,DEV=0", CARDS), by_index);
        assert_eq!(alsa_device_key("hw:PRO,0", CARDS), by_index);
        assert_eq!(alsa_device_key("hw:CARD=PRO", CARDS), by_index);
    }

    /// An unplugged card's name is still that card, and never another one.
    #[test]
    fn an_unresolvable_name_equals_only_itself() {
        let named = alsa_device_key("plughw:CARD=PRO,DEV=0", "");
        assert_eq!(alsa_device_key("hw:CARD=PRO,DEV=0", ""), named);
        assert_ne!(alsa_device_key("hw:1,0", ""), named);
        assert_ne!(alsa_device_key("hw:CARD=USB,DEV=0", ""), named);
        assert_ne!(
            alsa_device_key("hw:CARD=pro,DEV=0", CARDS),
            alsa_device_key("hw:CARD=PRO,DEV=0", CARDS),
            "alsa-lib compares card ids exactly"
        );
    }

    /// The counterpart: different hardware keeps different keys.
    #[test]
    fn different_cards_devices_and_subdevices_stay_different() {
        let a = alsa_device_key("hw:1,0", CARDS);
        assert_ne!(alsa_device_key("hw:2,0", CARDS), a);
        assert_ne!(alsa_device_key("hw:1,1", CARDS), a);
        assert_ne!(alsa_device_key("hw:1,0,1", CARDS), a);
        assert_ne!(alsa_device_key("hw:CARD=Headphones,DEV=0", CARDS), a);
    }

    #[test]
    fn other_alsa_strings_are_compared_as_written() {
        assert_eq!(alsa_device_key("default", ""), "alsa-raw:default");
        assert_ne!(
            alsa_device_key("dsnoop:1,0", ""),
            alsa_device_key("hw:1,0", "")
        );
        assert_ne!(alsa_device_key("hw:", ""), alsa_device_key("hw:0", ""));
        assert_ne!(alsa_device_key("hw:1,x", ""), alsa_device_key("hw:1,0", ""));
    }

    #[test]
    fn credentials_case_default_port_and_trailing_slash_do_not_make_a_new_camera() {
        let k = stream_url_key("rtsp://cam.local/live");
        assert_eq!(stream_url_key("rtsp://user:pw@cam.local/live/"), k);
        assert_eq!(stream_url_key("RTSP://CAM.local:554/live"), k);
        assert_eq!(stream_url_key("  rtsp://admin@Cam.Local:554/live//  "), k);
    }

    /// The counterpart: two cameras, two ports, two paths stay apart.
    #[test]
    fn different_hosts_ports_and_paths_stay_different() {
        let k = stream_url_key("rtsp://cam.local/live");
        assert_ne!(stream_url_key("rtsp://cam2.local/live"), k);
        assert_ne!(stream_url_key("rtsp://cam.local:8554/live"), k);
        assert_ne!(
            stream_url_key("rtsp://cam.local/Live"),
            k,
            "paths are case-sensitive"
        );
        assert_ne!(stream_url_key("rtsp://cam.local/live?channel=2"), k);
        assert_ne!(stream_url_key("rtsps://cam.local/live"), k);
    }

    #[test]
    fn an_ipv6_host_keeps_its_brackets_and_its_port() {
        assert_eq!(
            stream_url_key("rtsp://[FE80::1]:554/s"),
            stream_url_key("rtsp://[fe80::1]/s")
        );
        assert_ne!(
            stream_url_key("rtsp://[fe80::1]:8554/s"),
            stream_url_key("rtsp://[fe80::1]/s")
        );
    }
}
