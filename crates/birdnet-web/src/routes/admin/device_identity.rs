//! When two audio-source addresses name the same device (MSC-10).
//!
//! The Audio page refused a second source only when its address matched an
//! existing one character for character. One USB microphone is `plughw:1,0`,
//! `hw:1,0` and `plughw:CARD=USB,DEV=0`; one camera is `rtsp://cam/live` and
//! `rtsp://user:pw@CAM:554/live/`. Added twice, an exclusive ALSA device fails
//! to open and restarts for ever, and a shared one or a camera records every
//! bird twice. Each address is reduced to an identity here and identities are
//! compared instead.
//!
//! PipeWire names are compared as written: whether a PipeWire source and an
//! ALSA row are one microphone would need PipeWire's own view of the cards,
//! which this does not ask for.

use birdnet_db::audio_sources::{AudioSource, SourceKind};

/// The text of `/proc/asound/cards`, or empty where it cannot be read (not
/// Linux, or no sound cards): a card named by `CARD=` then compares by name.
#[must_use]
pub fn sound_cards() -> String {
    std::fs::read_to_string("/proc/asound/cards").unwrap_or_default()
}

/// A comparable identity for `device_id` of `kind`. `cards` is the text of
/// `/proc/asound/cards`, used to turn `CARD=<name>` into the card's number.
#[must_use]
pub fn identity(kind: SourceKind, device_id: &str, cards: &str) -> String {
    let device_id = device_id.trim();
    match kind {
        SourceKind::UsbAlsa => alsa_identity(device_id, cards),
        SourceKind::Rtsp => url_identity(device_id),
        SourceKind::PipeWire => format!("pipewire:{device_id}"),
    }
}

/// The source in `rows`, other than `except`, that names the same device as
/// `device_id` of `kind`.
#[must_use]
pub fn duplicate_of<'a>(
    rows: &'a [AudioSource],
    kind: SourceKind,
    device_id: &str,
    cards: &str,
    except: Option<&str>,
) -> Option<&'a AudioSource> {
    let wanted = identity(kind, device_id, cards);
    rows.iter().find(|row| {
        Some(row.id.as_str()) != except
            && row.kind == kind
            && identity(row.kind, &row.device_id, cards) == wanted
    })
}

/// `hw:1,0`, `plughw:1,0`, `plughw:CARD=USB,DEV=0`, `hw:1` and the other PCM
/// plugins over one card and device are the same microphone: `alsa:<card>:<dev>`.
/// An address with no `:` (`default`) is kept as written.
fn alsa_identity(device_id: &str, cards: &str) -> String {
    let Some((_plugin, params)) = device_id.split_once(':') else {
        return format!("alsa:{}", device_id.to_ascii_lowercase());
    };
    let mut card: Option<String> = None;
    let mut dev: Option<String> = None;
    for (i, part) in params.split(',').map(str::trim).enumerate() {
        if let Some((key, value)) = part.split_once('=') {
            match key.trim().to_ascii_uppercase().as_str() {
                "CARD" => card = Some(value.trim().to_owned()),
                "DEV" => dev = Some(value.trim().to_owned()),
                _ => {}
            }
        } else if i == 0 {
            card = Some(part.to_owned());
        } else if i == 1 {
            dev = Some(part.to_owned());
        }
    }
    let card = card.map_or_else(String::new, |c| card_number(&c, cards));
    let dev = dev
        .filter(|d| !d.is_empty())
        .unwrap_or_else(|| "0".to_owned());
    format!("alsa:{card}:{dev}")
}

/// A card given by number stays a number; one given by name becomes its
/// number from `/proc/asound/cards` (lines like ` 1 [USB            ]: …`), or
/// stays the lower-cased name when the listing does not have it.
fn card_number(card: &str, cards: &str) -> String {
    if card.bytes().all(|b| b.is_ascii_digit()) && !card.is_empty() {
        let n = card.trim_start_matches('0');
        return if n.is_empty() { "0" } else { n }.to_owned();
    }
    for line in cards.lines() {
        let line = line.trim_start();
        let Some((number, rest)) = line.split_once(' ') else {
            continue;
        };
        if !number.bytes().all(|b| b.is_ascii_digit()) || number.is_empty() {
            continue;
        }
        let Some(name) = rest
            .trim_start()
            .strip_prefix('[')
            .and_then(|r| r.split_once(']'))
            .map(|(n, _)| n.trim())
        else {
            continue;
        };
        if name.eq_ignore_ascii_case(card) {
            return number.to_owned();
        }
    }
    card.to_ascii_lowercase()
}

/// A stream address without its credentials, with the scheme and host
/// lower-cased, the scheme's default port written out and a trailing `/`
/// removed from the path. The path and query keep their case: a camera's path
/// is the camera's to interpret.
fn url_identity(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return url.to_owned();
    };
    let scheme = scheme.to_ascii_lowercase();
    let (authority, path) = rest
        .find(['/', '?'])
        .map_or((rest, ""), |i| (&rest[..i], &rest[i..]));
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host_port = host_port.to_ascii_lowercase();
    let has_port = host_port
        .rsplit_once(':')
        .is_some_and(|(_, p)| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
        && !host_port.ends_with(']');
    let default_port = match scheme.as_str() {
        "rtsp" => Some("554"),
        "rtsps" => Some("322"),
        "http" => Some("80"),
        "https" => Some("443"),
        _ => None,
    };
    let host_port = match (has_port, default_port) {
        (false, Some(port)) => format!("{host_port}:{port}"),
        _ => host_port,
    };
    let (path, query) = path
        .split_once('?')
        .map_or((path, None), |(p, q)| (p, Some(q)));
    let path = path.trim_end_matches('/');
    query.map_or_else(
        || format!("{scheme}://{host_port}{path}"),
        |q| format!("{scheme}://{host_port}{path}?{q}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const CARDS: &str = " 0 [vc4hdmi        ]: vc4-hdmi - vc4-hdmi\n                      vc4-hdmi\n 1 [USB            ]: USB-Audio - USB Audio Device\n                      C-Media USB Audio Device at usb-xhci-hcd.0-1\n";

    #[test]
    fn one_alsa_microphone_has_one_identity_however_it_is_written() {
        let id = |d| identity(SourceKind::UsbAlsa, d, CARDS);
        for spelling in [
            "plughw:1,0",
            "hw:1,0",
            "hw:1",
            "plughw:CARD=USB,DEV=0",
            "dsnoop:CARD=usb",
        ] {
            assert_eq!(id(spelling), "alsa:1:0", "{spelling}");
        }
        // Counterparts: another device or card is another identity.
        assert_ne!(id("hw:1,1"), id("hw:1,0"));
        assert_ne!(id("hw:0,0"), id("hw:1,0"));
        assert_ne!(id("plughw:CARD=Other,DEV=0"), id("hw:1,0"));
    }

    #[test]
    fn one_camera_has_one_identity_however_it_is_written() {
        let id = |u| identity(SourceKind::Rtsp, u, "");
        let base = id("rtsp://cam.local/live");
        for spelling in [
            "rtsp://user:pw@cam.local/live",
            "RTSP://CAM.local:554/live/",
            "rtsp://cam.local:554/live",
        ] {
            assert_eq!(id(spelling), base, "{spelling}");
        }
        // Counterparts: another port, path or host is another camera, and a
        // path keeps its case.
        assert_ne!(id("rtsp://cam.local:8554/live"), base);
        assert_ne!(id("rtsp://cam.local/live2"), base);
        assert_ne!(id("rtsp://cam.local/Live"), base);
        assert_ne!(id("rtsp://cam2.local/live"), base);
    }
}
