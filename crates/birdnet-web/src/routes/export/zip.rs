//! A deterministic ZIP writer for export bundles (RES-08).
//!
//! The same members in the same order give the same bytes: every entry is
//! dated 1980-01-01 00:00 (the earliest DOS date, so no clock reaches the
//! archive), carries no extra fields, no comment and no owner, and is DEFLATE
//! at a fixed level. Members are written in the order given. ZIP64 is not
//! written: a member or an archive past 4 GiB is refused, which the export's
//! row bound keeps far away.

use std::io::Write as _;

/// Why an archive could not be built.
#[derive(Debug)]
pub enum ZipError {
    /// A member, or the archive, does not fit the 32-bit ZIP format.
    TooLarge(String),
    /// Compression failed.
    Io(std::io::Error),
}

impl std::fmt::Display for ZipError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLarge(what) => write!(f, "{what} is too large for a ZIP archive"),
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ZipError {}

/// DOS time 00:00:00 and date 1980-01-01.
const DOS_TIME: u16 = 0;
const DOS_DATE: u16 = (1 << 5) | 1;

fn u32_of(n: usize, what: &str) -> Result<u32, ZipError> {
    u32::try_from(n).map_err(|_| ZipError::TooLarge(what.to_owned()))
}

/// Build an archive of `members`, `(path, bytes)`, in order.
///
/// # Errors
///
/// A member or the archive past the 32-bit format, or a compression failure.
pub fn build(members: &[(String, Vec<u8>)]) -> Result<Vec<u8>, ZipError> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    let count = u16::try_from(members.len())
        .map_err(|_| ZipError::TooLarge("the member count".to_owned()))?;
    for (path, data) in members {
        let mut enc = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::new(6));
        enc.write_all(data).map_err(ZipError::Io)?;
        let packed = enc.finish().map_err(ZipError::Io)?;
        let mut crc = flate2::Crc::new();
        crc.update(data);
        let crc = crc.sum();
        let name = path.as_bytes();
        let name_len = u16::try_from(name.len())
            .map_err(|_| ZipError::TooLarge(format!("the name {path}")))?;
        let csize = u32_of(packed.len(), path)?;
        let usize_ = u32_of(data.len(), path)?;
        let offset = u32_of(out.len(), "the archive")?;

        // Local file header.
        out.extend_from_slice(&0x0403_4b50_u32.to_le_bytes());
        out.extend_from_slice(&20_u16.to_le_bytes()); // version needed
        out.extend_from_slice(&0x0800_u16.to_le_bytes()); // UTF-8 names
        out.extend_from_slice(&8_u16.to_le_bytes()); // DEFLATE
        out.extend_from_slice(&DOS_TIME.to_le_bytes());
        out.extend_from_slice(&DOS_DATE.to_le_bytes());
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&csize.to_le_bytes());
        out.extend_from_slice(&usize_.to_le_bytes());
        out.extend_from_slice(&name_len.to_le_bytes());
        out.extend_from_slice(&0_u16.to_le_bytes()); // extra
        out.extend_from_slice(name);
        out.extend_from_slice(&packed);

        // Central directory entry.
        central.extend_from_slice(&0x0201_4b50_u32.to_le_bytes());
        central.extend_from_slice(&0x031e_u16.to_le_bytes()); // made by: Unix, 3.0
        central.extend_from_slice(&20_u16.to_le_bytes());
        central.extend_from_slice(&0x0800_u16.to_le_bytes());
        central.extend_from_slice(&8_u16.to_le_bytes());
        central.extend_from_slice(&DOS_TIME.to_le_bytes());
        central.extend_from_slice(&DOS_DATE.to_le_bytes());
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&csize.to_le_bytes());
        central.extend_from_slice(&usize_.to_le_bytes());
        central.extend_from_slice(&name_len.to_le_bytes());
        central.extend_from_slice(&0_u16.to_le_bytes()); // extra
        central.extend_from_slice(&0_u16.to_le_bytes()); // comment
        central.extend_from_slice(&0_u16.to_le_bytes()); // disk
        central.extend_from_slice(&0_u16.to_le_bytes()); // internal attrs
        central.extend_from_slice(&(0o100_644_u32 << 16).to_le_bytes()); // -rw-r--r--
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name);
    }
    let cd_offset = u32_of(out.len(), "the archive")?;
    let cd_size = u32_of(central.len(), "the central directory")?;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4b50_u32.to_le_bytes());
    out.extend_from_slice(&0_u16.to_le_bytes());
    out.extend_from_slice(&0_u16.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&cd_size.to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&0_u16.to_le_bytes());
    u32_of(out.len(), "the archive")?;
    Ok(out)
}

/// Read an archive written by [`build`] back into its members (for tests and
/// for the bundle's own round-trip check): stored names and inflated bytes,
/// each checked against its CRC.
#[cfg(test)]
pub fn read(archive: &[u8]) -> Vec<(String, Vec<u8>)> {
    use std::io::Read as _;
    let le16 = |b: &[u8], at: usize| usize::from(u16::from_le_bytes([b[at], b[at + 1]]));
    let le32 =
        |b: &[u8], at: usize| u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]) as usize;
    let mut out = Vec::new();
    let mut at = 0;
    while archive.len() >= at + 4 && le32(archive, at) == 0x0403_4b50 {
        let crc = u32::try_from(le32(archive, at + 14)).unwrap();
        let csize = le32(archive, at + 18);
        let name_len = le16(archive, at + 26);
        let extra = le16(archive, at + 28);
        let name = String::from_utf8(archive[at + 30..at + 30 + name_len].to_vec()).unwrap();
        let start = at + 30 + name_len + extra;
        let mut data = Vec::new();
        flate2::read::DeflateDecoder::new(&archive[start..start + csize])
            .read_to_end(&mut data)
            .unwrap();
        let mut c = flate2::Crc::new();
        c.update(&data);
        assert_eq!(c.sum(), crc, "CRC of {name}");
        out.push((name, data));
        at = start + csize;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_members_give_the_same_bytes_and_read_back() {
        let members = vec![
            ("a.txt".to_owned(), b"hello".to_vec()),
            ("dir/b.csv".to_owned(), b"x,y\n1,2\n".repeat(100)),
            ("empty".to_owned(), Vec::new()),
        ];
        let one = build(&members).unwrap();
        let two = build(&members).unwrap();
        assert_eq!(one, two);
        assert_eq!(read(&one), members);
        // End of central directory: three entries.
        let eocd = &one[one.len() - 22..];
        assert_eq!(&eocd[..4], &0x0605_4b50_u32.to_le_bytes());
        assert_eq!(u16::from_le_bytes([eocd[10], eocd[11]]), 3);
    }

    #[test]
    fn a_changed_member_changes_the_archive() {
        let a = build(&[("a".to_owned(), b"1".to_vec())]).unwrap();
        let b = build(&[("a".to_owned(), b"2".to_vec())]).unwrap();
        assert_ne!(a, b);
    }
}
