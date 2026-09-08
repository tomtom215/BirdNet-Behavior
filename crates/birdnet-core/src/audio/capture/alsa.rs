//! What an ALSA device string names, and which card is really behind it.
//!
//! Two forms of device string reach the station, and they age differently:
//!
//! * an **index** (`plughw:1,0`) — assigned in detection order, and not
//!   stable: the same microphone was `card 1` before a cold reboot on a
//!   Raspberry Pi 4 and `card 3` after it;
//! * an **id** (`plughw:CARD=PRO,DEV=0`) — the name ALSA gives the card, which
//!   `usb-audio-mapper` pins via a udev rule (`ATTR{id}="<name>"`).
//!
//! An index that still resolves after a re-enumeration resolves to whatever
//! now sits at it. [`card_id`] reads the id behind an index from the kernel,
//! so a start can be compared with the last one (AU-1).

use std::path::Path;

/// How a configured ALSA device string identifies its card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CardRef {
    /// A positional card index, `plughw:1,0`.
    Index(String),
    /// A card id, `plughw:CARD=PRO,DEV=0`.
    Id(String),
}

/// Parse the card out of an ALSA device string.
///
/// Accepts `plughw:1,0`, `hw:1,0`, `1,0`, `1`, and the id form
/// `plughw:CARD=PRO,DEV=0` / `hw:CARD=PRO`. Returns `None` for anything else
/// (`default`, a PipeWire node name, empty), which a caller reports as
/// unverifiable rather than as broken.
#[must_use]
pub fn parse_card_ref(device: &str) -> Option<CardRef> {
    // Strip a leading PCM plugin name ("plughw:", "hw:") if present. Split on
    // the FIRST colon: an id could in principle contain one, and everything
    // after it belongs to the argument list.
    let args = match device.split_once(':') {
        Some((_plugin, rest)) => rest,
        None => device,
    };

    // Named-argument form: CARD=<id>[,DEV=<n>][,SUBDEV=<n>]. alsa-lib declares
    // CARD as `type string` in its own alsa.conf, so the value is a name.
    for field in args.split(',') {
        if let Some(id) = field.trim().strip_prefix("CARD=") {
            let id = id.trim();
            if !id.is_empty() {
                return Some(CardRef::Id(id.to_string()));
            }
            return None;
        }
    }

    // Positional form: the card is the first argument.
    let first = args.split(',').next().unwrap_or(args).trim();
    if !first.is_empty() && first.chars().all(|c| c.is_ascii_digit()) {
        return Some(CardRef::Index(first.to_string()));
    }
    None
}

/// Where the kernel publishes each sound card.
pub const PROC_ASOUND: &str = "/proc/asound";

/// The id of the card at `index` now, from `/proc/asound/card<index>/id`;
/// `None` when no card sits at that index (or this is not Linux).
#[must_use]
pub fn card_id(index: &str) -> Option<String> {
    card_id_in(Path::new(PROC_ASOUND), index)
}

/// [`card_id`] against another root, for tests.
#[must_use]
pub fn card_id_in(root: &Path, index: &str) -> Option<String> {
    if index.is_empty() || !index.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let id = std::fs::read_to_string(root.join(format!("card{index}")).join("id")).ok()?;
    let id = id.trim();
    (!id.is_empty()).then(|| id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_forms_parse_and_the_rest_do_not() {
        assert_eq!(
            parse_card_ref("plughw:1,0"),
            Some(CardRef::Index("1".into()))
        );
        assert_eq!(parse_card_ref("hw:3"), Some(CardRef::Index("3".into())));
        assert_eq!(parse_card_ref("2,0"), Some(CardRef::Index("2".into())));
        assert_eq!(
            parse_card_ref("plughw:CARD=PRO,DEV=0"),
            Some(CardRef::Id("PRO".into()))
        );
        assert_eq!(parse_card_ref("default"), None);
        assert_eq!(parse_card_ref("plughw:CARD=,DEV=0"), None);
        assert_eq!(parse_card_ref(""), None);
    }

    /// The kernel's own record of what sits at an index, read the way the
    /// boot journal reads it.
    #[test]
    fn the_card_id_is_read_from_the_kernel_tree() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("card1")).unwrap();
        std::fs::write(root.path().join("card1/id"), "PRO\n").unwrap();
        assert_eq!(card_id_in(root.path(), "1").as_deref(), Some("PRO"));
        assert_eq!(card_id_in(root.path(), "3"), None, "nothing at index 3");
        assert_eq!(card_id_in(root.path(), "../etc"), None, "not an index");
    }
}
