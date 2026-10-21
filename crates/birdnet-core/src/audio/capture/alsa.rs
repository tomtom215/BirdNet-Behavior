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

/// One sound-card device that can record, as the kernel lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureDevice {
    /// The card's position (`card1`). Not stable across reboots.
    pub card: u32,
    /// The device on that card.
    pub device: u32,
    /// The card's id (`Device`, `PRO`): the name ALSA gives it, which survives a
    /// re-enumeration.
    pub id: String,
    /// The card's human name (`USB Audio Device`).
    pub name: String,
    /// Whether the card is a USB audio card.
    pub usb: bool,
}

impl CaptureDevice {
    /// The device string to record from: by the card's id, not its position, so
    /// it still names the same microphone after a reboot moves the index.
    #[must_use]
    pub fn spec(&self) -> String {
        format!("plughw:CARD={},DEV={}", self.id, self.device)
    }
}

/// Every device that can record, from `/proc/asound`.
///
/// Reads the kernel's own files rather than running `arecord -l`: nothing to
/// install, nothing to time out, and the same answer when `alsa-utils` is
/// missing (which is when a person most needs to be told what *is* plugged in).
#[must_use]
pub fn list_capture_devices() -> Vec<CaptureDevice> {
    list_capture_devices_in(Path::new(PROC_ASOUND))
}

/// [`list_capture_devices`] against another root, for tests.
///
/// `cards` has two lines per card, ` 1 [Device         ]: USB-Audio - USB Audio
/// Device`, and `pcm` has one per device, `01-00: USB Audio : USB Audio :
/// playback 1 : capture 1`. A card with no capture device is not listed (a
/// speaker-only HDMI output), and a malformed line is skipped, not fatal: the
/// format is the kernel's, not ours.
#[must_use]
pub fn list_capture_devices_in(root: &Path) -> Vec<CaptureDevice> {
    let Ok(cards) = std::fs::read_to_string(root.join("cards")) else {
        return Vec::new();
    };
    let Ok(pcm) = std::fs::read_to_string(root.join("pcm")) else {
        return Vec::new();
    };
    let mut by_card: Vec<(u32, String, String, bool)> = Vec::new();
    for line in cards.lines() {
        // " 1 [Device         ]: USB-Audio - USB Audio Device"
        let Some((head, tail)) = line.split_once("]:") else {
            continue;
        };
        let Some((num, id)) = head.split_once('[') else {
            continue;
        };
        let Ok(card) = num.trim().parse::<u32>() else {
            continue;
        };
        let id = id.trim().to_owned();
        let (driver, name) = tail
            .split_once(" - ")
            .map_or((tail.trim(), tail.trim()), |(d, n)| (d.trim(), n.trim()));
        by_card.push((
            card,
            id,
            name.to_owned(),
            driver.eq_ignore_ascii_case("USB-Audio"),
        ));
    }
    let mut out = Vec::new();
    for line in pcm.lines() {
        // "01-00: USB Audio : USB Audio : playback 1 : capture 1"
        let Some((addr, rest)) = line.split_once(':') else {
            continue;
        };
        if !rest.split(':').any(|f| f.trim().starts_with("capture")) {
            continue;
        }
        let Some((card, device)) = addr.trim().split_once('-') else {
            continue;
        };
        let (Ok(card), Ok(device)) = (card.parse::<u32>(), device.parse::<u32>()) else {
            continue;
        };
        let Some((_, id, name, usb)) = by_card.iter().find(|c| c.0 == card) else {
            continue;
        };
        if id.is_empty() {
            continue;
        }
        out.push(CaptureDevice {
            card,
            device,
            id: id.clone(),
            name: name.clone(),
            usb: *usb,
        });
    }
    out
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

    /// A USB microphone and a speaker-only card, as a Raspberry Pi 4 with a
    /// USB microphone lists them (shape taken from the kernel's own format).
    #[test]
    fn capture_devices_are_read_from_the_kernel_tree() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("cards"),
            " 0 [Headphones     ]: bcm2835_headphonesn - bcm2835 Headphones\n\
             \x20                     bcm2835 Headphones\n\
             \x201 [Device         ]: USB-Audio - USB Audio Device\n\
             \x20                     C-Media Electronics Inc. USB Audio Device at usb-0000:01:00.0-1.2, full speed\n",
        )
        .unwrap();
        std::fs::write(
            root.path().join("pcm"),
            "00-00: bcm2835 Headphones : bcm2835 Headphones : playback 8\n\
             01-00: USB Audio : USB Audio : playback 1 : capture 1\n",
        )
        .unwrap();
        let devices = list_capture_devices_in(root.path());
        assert_eq!(devices.len(), 1, "only the card that records: {devices:?}");
        let d = &devices[0];
        assert_eq!((d.card, d.device), (1, 0));
        assert_eq!(d.id, "Device");
        assert_eq!(d.name, "USB Audio Device");
        assert!(d.usb);
        assert_eq!(
            d.spec(),
            "plughw:CARD=Device,DEV=0",
            "by id, which survives the index moving after a reboot"
        );
        // And the string it produces is one the rest of the station understands.
        assert_eq!(
            parse_card_ref(&d.spec()),
            Some(CardRef::Id("Device".into()))
        );
    }

    /// No sound subsystem (a container, a server): an empty list, not an error.
    #[test]
    fn a_machine_without_sound_lists_nothing() {
        let root = tempfile::tempdir().unwrap();
        assert!(list_capture_devices_in(root.path()).is_empty());
        std::fs::write(root.path().join("cards"), "--- no soundcards ---\n").unwrap();
        std::fs::write(root.path().join("pcm"), "").unwrap();
        assert!(list_capture_devices_in(root.path()).is_empty());
    }
}
