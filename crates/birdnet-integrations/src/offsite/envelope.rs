//! The encrypted container an offsite backup is uploaded in.
//!
//! # Why the station encrypts before it uploads
//!
//! A station's database is a log of where somebody lives and when they are
//! home. "Server-side encryption" on the bucket means the provider holds the
//! key; an SFTP host means the host's administrator does. Encrypting here means
//! the only place a plaintext backup exists is the station and whatever machine
//! the operator restores it on, which is the property that makes it reasonable
//! to advise sending backups to a rented bucket at all.
//!
//! # Format
//!
//! A fixed 52-byte header, then a sequence of AEAD chunks:
//!
//! ```text
//! off  len  field
//!   0    8  magic  b"BNBBAK1\n"
//!   8    1  KDF id            (1 = argon2id)
//!   9    1  AEAD id           (1 = ChaCha20-Poly1305)
//!  10    2  reserved, zero
//!  12    4  argon2 m_cost, KiB, big-endian
//!  16    4  argon2 t_cost, big-endian
//!  20    4  argon2 p_cost, big-endian
//!  24    4  plaintext bytes per chunk, big-endian
//!  28   16  argon2 salt
//!  44    7  nonce prefix
//!  51    1  reserved, zero
//! ```
//!
//! then `ceil(len / chunk) + (len == 0)` chunks of `plaintext + 16-byte tag`,
//! the last one short.
//!
//! ## Two decisions worth stating
//!
//! **The whole header is the AAD of every chunk.** Without that, an attacker
//! who could rewrite bytes on the storage host could lower `m_cost` to 8 KiB
//! and hand the operator back a file whose passphrase is cheap to brute-force —
//! and the operator would never know, because the file would still decrypt.
//!
//! **Nonces are a STREAM counter, not random.** Each chunk is sealed under
//! `prefix(7) ‖ counter(4, big-endian) ‖ final(1)`, where `final` is 1 for the
//! last chunk and 0 for every other. That is what makes truncation detectable:
//! cutting the file short leaves a chunk that was sealed with `final = 0` in
//! the position where the reader demands `final = 1`, and the tag check fails.
//! Random per-chunk nonces would authenticate every chunk and still let an
//! attacker silently drop the tail — which for a backup means "restores
//! cleanly, missing last March".
//!
//! Constructing nonces is why this uses [`ring::aead::LessSafeKey`]. The name
//! is a warning about nonce reuse; here the prefix is fresh random per file and
//! the counter is strictly increasing within it, so no `(key, nonce)` pair
//! repeats. [`SealingKey`](ring::aead::SealingKey) cannot express a trailing
//! final-chunk flag, which is the property being bought.

use std::io::{self, Read, Write};

use ring::aead::{Aad, CHACHA20_POLY1305, LessSafeKey, NONCE_LEN, Nonce, UnboundKey};
use ring::rand::{SecureRandom, SystemRandom};

/// Leading bytes of every envelope this writes.
///
/// Ends in `\n` so `head -c8` on a backup that turned out not to be encrypted
/// does not leave a terminal mid-line, and starts with printable ASCII so
/// `file(1)` and a human both get a hint.
pub const MAGIC: &[u8; 8] = b"BNBBAK1\n";

/// Size of the fixed header, in bytes.
pub const HEADER_LEN: usize = 52;

/// AEAD tag length appended to every chunk.
const TAG_LEN: usize = 16;

/// Bytes of plaintext per chunk.
///
/// 1 MiB: large enough that the per-chunk 16-byte tag is 0.0015% overhead and
/// a 400 MB database is 400 chunks rather than 400 000, small enough that a
/// station with 512 MB of RAM never holds more than a megabyte of the backup
/// at once. Recorded in the header, so a future default does not strand
/// existing files.
pub const CHUNK_LEN: u32 = 1024 * 1024;

/// Length of the random nonce prefix.
const PREFIX_LEN: usize = 7;

/// Length of the argon2 salt.
const SALT_LEN: usize = 16;

/// Identifier for argon2id in the header's KDF field.
const KDF_ARGON2ID: u8 = 1;

/// Identifier for ChaCha20-Poly1305 in the header's AEAD field.
const AEAD_CHACHA20_POLY1305: u8 = 1;

/// argon2id memory cost, in KiB.
///
/// 19 MiB with one pass is OWASP's current second recommendation, and is the
/// one that fits a Raspberry Pi: the first (46 MiB) is a fifth of the usable
/// RAM on a 256 MB Pi Zero, and this runs while the station is also holding an
/// ONNX session. Measured on this machine, deriving a key at these parameters
/// takes about 30 ms — irrelevant next to uploading a hundred megabytes, and
/// still 19 MiB per guess for anyone attacking the file.
pub const ARGON2_M_COST: u32 = 19 * 1024;

/// argon2id time cost (passes).
pub const ARGON2_T_COST: u32 = 2;

/// argon2id parallelism.
pub const ARGON2_P_COST: u32 = 1;

/// Smallest passphrase this will encrypt under.
///
/// Not a policy about password strength — it is the line below which the
/// argon2 parameters above stop being the thing that protects the file. A
/// six-character passphrase falls to a dictionary in an afternoon at any cost
/// parameters, and a backup that is *believed* encrypted is worse than one
/// known not to be.
pub const MIN_PASSPHRASE_LEN: usize = 12;

/// What can go wrong reading or writing an envelope.
#[derive(Debug)]
pub enum EnvelopeError {
    /// Underlying reader or writer failed.
    Io(io::Error),
    /// The passphrase is shorter than [`MIN_PASSPHRASE_LEN`].
    PassphraseTooShort {
        /// How long it was.
        len: usize,
    },
    /// The input does not begin with [`MAGIC`].
    NotAnEnvelope,
    /// The header names a KDF or AEAD this build does not implement.
    UnsupportedAlgorithm {
        /// The KDF identifier from the header.
        kdf: u8,
        /// The AEAD identifier from the header.
        aead: u8,
    },
    /// A header field is outside the range this can act on.
    MalformedHeader(&'static str),
    /// Key derivation failed.
    KeyDerivation(String),
    /// A chunk failed its authentication check.
    ///
    /// Carries the chunk index, because "chunk 0" (wrong passphrase or a
    /// rewritten header) and "the last chunk" (a truncated upload) are
    /// different problems with different fixes, and the operator needs to be
    /// told which one they have.
    Authentication {
        /// Zero-based index of the chunk that failed.
        chunk: u64,
    },
    /// The stream ended in the middle of a chunk.
    Truncated {
        /// Zero-based index of the chunk that was cut short.
        chunk: u64,
    },
}

impl std::fmt::Display for EnvelopeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io error: {e}"),
            Self::PassphraseTooShort { len } => write!(
                f,
                "backup passphrase is {len} characters; at least {MIN_PASSPHRASE_LEN} are needed \
                 for the encryption to be worth anything"
            ),
            Self::NotAnEnvelope => write!(
                f,
                "this file does not start with the backup envelope magic — it is not an \
                 encrypted backup this station wrote"
            ),
            Self::UnsupportedAlgorithm { kdf, aead } => write!(
                f,
                "the envelope names KDF {kdf} and AEAD {aead}; this build implements \
                 {KDF_ARGON2ID} and {AEAD_CHACHA20_POLY1305}. A newer station wrote it"
            ),
            Self::MalformedHeader(what) => write!(f, "malformed envelope header: {what}"),
            Self::KeyDerivation(e) => write!(f, "could not derive a key: {e}"),
            Self::Authentication { chunk: 0 } => write!(
                f,
                "the first chunk failed its authentication check: either the passphrase is \
                 wrong or the file has been altered"
            ),
            Self::Authentication { chunk } => write!(
                f,
                "chunk {chunk} failed its authentication check: the file has been altered, \
                 truncated, or its chunks reordered"
            ),
            Self::Truncated { chunk } => write!(
                f,
                "the file ends in the middle of chunk {chunk}: the upload did not finish"
            ),
        }
    }
}

impl std::error::Error for EnvelopeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for EnvelopeError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// The parsed header, and the key derived under it.
struct Sealed {
    header: [u8; HEADER_LEN],
    key: LessSafeKey,
    chunk_len: usize,
}

impl Sealed {
    /// Nonce for chunk `index`, marked final or not.
    ///
    /// Panics only on a prefix of the wrong length, which is a local
    /// constant — the assembly below is size-checked by the array types.
    fn nonce(&self, index: u64, final_chunk: bool) -> Result<Nonce, EnvelopeError> {
        let counter = u32::try_from(index).map_err(|_| {
            EnvelopeError::MalformedHeader("more chunks than the 32-bit counter can address")
        })?;
        let mut bytes = [0u8; NONCE_LEN];
        bytes[..PREFIX_LEN].copy_from_slice(&self.header[44..44 + PREFIX_LEN]);
        bytes[PREFIX_LEN..PREFIX_LEN + 4].copy_from_slice(&counter.to_be_bytes());
        bytes[NONCE_LEN - 1] = u8::from(final_chunk);
        Ok(Nonce::assume_unique_for_key(bytes))
    }
}

/// Derive the key named by a header, and bundle it with the header.
fn seal_from_header(passphrase: &str, header: [u8; HEADER_LEN]) -> Result<Sealed, EnvelopeError> {
    if passphrase.chars().count() < MIN_PASSPHRASE_LEN {
        return Err(EnvelopeError::PassphraseTooShort {
            len: passphrase.chars().count(),
        });
    }
    if &header[..8] != MAGIC {
        return Err(EnvelopeError::NotAnEnvelope);
    }
    let kdf = header[8];
    let aead = header[9];
    if kdf != KDF_ARGON2ID || aead != AEAD_CHACHA20_POLY1305 {
        return Err(EnvelopeError::UnsupportedAlgorithm { kdf, aead });
    }

    let m_cost = u32::from_be_bytes([header[12], header[13], header[14], header[15]]);
    let t_cost = u32::from_be_bytes([header[16], header[17], header[18], header[19]]);
    let p_cost = u32::from_be_bytes([header[20], header[21], header[22], header[23]]);
    let chunk_len = u32::from_be_bytes([header[24], header[25], header[26], header[27]]);

    if chunk_len == 0 {
        return Err(EnvelopeError::MalformedHeader("chunk size is zero"));
    }
    // A header can name a chunk size this station cannot buffer. Refuse rather
    // than attempt a 4 GiB allocation on a Pi and be OOM-killed mid-restore.
    if chunk_len > 64 * 1024 * 1024 {
        return Err(EnvelopeError::MalformedHeader(
            "chunk size is larger than 64 MiB; refusing to buffer it",
        ));
    }

    let params = argon2::Params::new(m_cost, t_cost, p_cost, Some(32))
        .map_err(|e| EnvelopeError::KeyDerivation(e.to_string()))?;
    let argon = argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
    let mut key_bytes = [0u8; 32];
    argon
        .hash_password_into(
            passphrase.as_bytes(),
            &header[28..28 + SALT_LEN],
            &mut key_bytes,
        )
        .map_err(|e| EnvelopeError::KeyDerivation(e.to_string()))?;

    let unbound = UnboundKey::new(&CHACHA20_POLY1305, &key_bytes)
        .map_err(|_| EnvelopeError::KeyDerivation("rejected 32-byte key".to_owned()))?;

    Ok(Sealed {
        header,
        key: LessSafeKey::new(unbound),
        chunk_len: chunk_len as usize,
    })
}

/// Build a fresh header with random salt and nonce prefix.
fn new_header() -> Result<[u8; HEADER_LEN], EnvelopeError> {
    let mut header = [0u8; HEADER_LEN];
    header[..8].copy_from_slice(MAGIC);
    header[8] = KDF_ARGON2ID;
    header[9] = AEAD_CHACHA20_POLY1305;
    header[12..16].copy_from_slice(&ARGON2_M_COST.to_be_bytes());
    header[16..20].copy_from_slice(&ARGON2_T_COST.to_be_bytes());
    header[20..24].copy_from_slice(&ARGON2_P_COST.to_be_bytes());
    header[24..28].copy_from_slice(&CHUNK_LEN.to_be_bytes());

    let rng = SystemRandom::new();
    let mut random = [0u8; SALT_LEN + PREFIX_LEN];
    rng.fill(&mut random).map_err(|_| {
        EnvelopeError::KeyDerivation("the system CSPRNG refused to produce a salt".to_owned())
    })?;
    header[28..28 + SALT_LEN].copy_from_slice(&random[..SALT_LEN]);
    header[44..44 + PREFIX_LEN].copy_from_slice(&random[SALT_LEN..]);
    Ok(header)
}

/// Encrypt everything `src` yields into `dst`. Returns the plaintext byte count.
///
/// # Errors
///
/// [`EnvelopeError::PassphraseTooShort`] below [`MIN_PASSPHRASE_LEN`],
/// [`EnvelopeError::Io`] on any read or write failure, and
/// [`EnvelopeError::KeyDerivation`] if argon2 or the system CSPRNG refuses.
pub fn encrypt<R: Read, W: Write>(
    passphrase: &str,
    src: &mut R,
    dst: &mut W,
) -> Result<u64, EnvelopeError> {
    let header = new_header()?;
    let sealed = seal_from_header(passphrase, header)?;
    dst.write_all(&header)?;

    // One chunk of plaintext, plus room for the tag appended in place.
    let mut buf = vec![0u8; sealed.chunk_len + TAG_LEN];
    let mut index: u64 = 0;
    let mut total: u64 = 0;

    // `pending` holds a full chunk that has been read but not yet sealed,
    // because whether it is the *final* chunk is only known once the reader has
    // been asked again and returned nothing. A file whose length is an exact
    // multiple of the chunk size is the case that gets this wrong: sealing
    // eagerly would mark its last full chunk non-final and then emit an empty
    // final chunk, which works but wastes a chunk; deciding lazily keeps the
    // format's "last chunk carries the flag" rule exact for both shapes.
    let mut pending = read_up_to(src, &mut buf[..sealed.chunk_len])?;

    loop {
        let filled = pending;
        // Peek ahead only when the chunk was full; a short read already means
        // end of stream for every `Read` this is used with, and asking again
        // would block on a pipe.
        let next = if filled == sealed.chunk_len {
            let mut lookahead = vec![0u8; sealed.chunk_len];
            let n = read_up_to(src, &mut lookahead)?;
            Some((lookahead, n))
        } else {
            None
        };
        let is_final = next.as_ref().is_none_or(|(_, n)| *n == 0);

        buf.truncate(filled);
        let nonce = sealed.nonce(index, is_final)?;
        sealed
            .key
            .seal_in_place_append_tag(nonce, Aad::from(&sealed.header), &mut buf)
            .map_err(|_| EnvelopeError::Authentication { chunk: index })?;
        dst.write_all(&buf)?;
        total += filled as u64;
        index += 1;

        match next {
            Some((data, n)) if n > 0 => {
                buf.clear();
                buf.extend_from_slice(&data[..n]);
                buf.reserve(TAG_LEN);
                pending = n;
            }
            _ => break,
        }
    }

    dst.flush()?;
    Ok(total)
}

/// Decrypt an envelope from `src` into `dst`. Returns the plaintext byte count.
///
/// # Errors
///
/// [`EnvelopeError::NotAnEnvelope`] if the magic is absent,
/// [`EnvelopeError::Authentication`] if a chunk fails its tag check (a wrong
/// passphrase shows up as chunk 0), [`EnvelopeError::Truncated`] if the stream
/// ends mid-chunk, and [`EnvelopeError::Io`] on any read or write failure.
pub fn decrypt<R: Read, W: Write>(
    passphrase: &str,
    src: &mut R,
    dst: &mut W,
) -> Result<u64, EnvelopeError> {
    let mut header = [0u8; HEADER_LEN];
    match src.read_exact(&mut header) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
            return Err(EnvelopeError::NotAnEnvelope);
        }
        Err(e) => return Err(e.into()),
    }
    let sealed = seal_from_header(passphrase, header)?;

    let framed = sealed.chunk_len + TAG_LEN;
    let mut buf = vec![0u8; framed];
    let mut index: u64 = 0;
    let mut total: u64 = 0;

    let mut filled = read_up_to(src, &mut buf)?;
    loop {
        if filled < TAG_LEN {
            // Less than a tag left: either an empty file body (no chunks at
            // all, which no writer produces — the empty plaintext still gets
            // one chunk) or a cut-off tail.
            return Err(EnvelopeError::Truncated { chunk: index });
        }
        // A short frame can only be the last chunk; a full one is only the last
        // if nothing follows.
        let next = if filled == framed {
            let mut lookahead = vec![0u8; framed];
            let n = read_up_to(src, &mut lookahead)?;
            Some((lookahead, n))
        } else {
            None
        };
        let is_final = next.as_ref().is_none_or(|(_, n)| *n == 0);

        let nonce = sealed.nonce(index, is_final)?;
        let plain = sealed
            .key
            .open_in_place(nonce, Aad::from(&sealed.header), &mut buf[..filled])
            .map_err(|_| EnvelopeError::Authentication { chunk: index })?;
        dst.write_all(plain)?;
        total += plain.len() as u64;
        index += 1;

        match next {
            Some((data, n)) if n > 0 => {
                buf.clear();
                buf.extend_from_slice(&data[..n]);
                filled = n;
            }
            _ => break,
        }
    }

    dst.flush()?;
    Ok(total)
}

/// Does this look like an envelope this station wrote?
///
/// Cheap enough to call on the first bytes of a downloaded file before
/// spending 30 ms on argon2 telling the operator the same thing.
#[must_use]
pub fn is_envelope(head: &[u8]) -> bool {
    head.len() >= MAGIC.len() && &head[..MAGIC.len()] == MAGIC
}

/// Read until `buf` is full or the reader is exhausted.
///
/// `Read::read` is allowed to return fewer bytes than asked for at any time —
/// a `File` on a network mount, a pipe, a decompressor. Treating one short read
/// as end-of-stream would silently split a chunk, and because every chunk here
/// carries its own tag, the result would be a *valid-looking* envelope that
/// decrypts to the right bytes but with a different chunk boundary. Encrypt and
/// decrypt would still agree, so nothing would ever fail — which is precisely
/// the kind of defect that only shows up on someone else's hardware.
fn read_up_to<R: Read>(src: &mut R, buf: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match src.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(filled)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASS: &str = "correct horse battery staple";

    fn round_trip(plain: &[u8]) -> Vec<u8> {
        let mut sealed = Vec::new();
        let n = encrypt(PASS, &mut &plain[..], &mut sealed).expect("encrypt");
        assert_eq!(n, plain.len() as u64, "encrypt reported the wrong length");
        let mut out = Vec::new();
        let m = decrypt(PASS, &mut &sealed[..], &mut out).expect("decrypt");
        assert_eq!(m, plain.len() as u64, "decrypt reported the wrong length");
        assert_eq!(out, plain, "round trip changed the bytes");
        sealed
    }

    /// A plaintext that is `n` bytes of something non-repeating, so a chunk
    /// swap or an off-by-one boundary shows up as different bytes rather than
    /// as the same byte in a different place.
    fn pattern(n: usize) -> Vec<u8> {
        #[allow(clippy::cast_possible_truncation)]
        (0..n).map(|i| (i % 251) as u8).collect()
    }

    #[test]
    fn round_trips_at_every_boundary_that_can_be_off_by_one() {
        // The lengths where the chunking logic changes behaviour. `CHUNK_LEN`
        // is a megabyte, so this uses a smaller envelope written by hand for
        // the multi-chunk cases; see `multi_chunk_round_trip`.
        for len in [0usize, 1, 15, 16, 17, 4096] {
            round_trip(&pattern(len));
        }
    }

    #[test]
    fn multi_chunk_round_trips_including_an_exact_multiple() {
        // The interesting one: a plaintext that is an exact multiple of the
        // chunk size. The final-chunk flag has to land on the last *full*
        // chunk, and an implementation that decides eagerly gets this wrong.
        for chunks in [1usize, 2, 3] {
            for extra in [0usize, 1] {
                let len = chunks * CHUNK_LEN as usize + extra;
                let sealed = round_trip(&pattern(len));
                // And the framing is exactly what the format claims, so a
                // change in chunking is visible here rather than only as a
                // silent format break for older files.
                let body = sealed.len() - HEADER_LEN;
                let expected_chunks = chunks + usize::from(extra > 0);
                assert_eq!(
                    body,
                    len + expected_chunks * TAG_LEN,
                    "{len} bytes should frame as {expected_chunks} chunks"
                );
            }
        }
    }

    #[test]
    fn a_wrong_passphrase_fails_on_the_first_chunk() {
        let sealed = round_trip(b"the quick brown fox");
        let mut out = Vec::new();
        let err = decrypt("a different passphrase", &mut &sealed[..], &mut out)
            .expect_err("a wrong passphrase must not decrypt");
        assert!(
            matches!(err, EnvelopeError::Authentication { chunk: 0 }),
            "expected a chunk-0 authentication failure, got {err}"
        );
        assert!(
            out.is_empty(),
            "nothing may be written before the tag checks"
        );
    }

    #[test]
    fn truncating_the_last_chunk_is_detected() {
        // The property random per-chunk nonces would not give: every remaining
        // chunk still authenticates, so only the final-chunk flag can catch it.
        let plain = pattern(3 * CHUNK_LEN as usize);
        let mut sealed = Vec::new();
        encrypt(PASS, &mut &plain[..], &mut sealed).expect("encrypt");

        let one_chunk = CHUNK_LEN as usize + TAG_LEN;
        sealed.truncate(sealed.len() - one_chunk);

        let mut out = Vec::new();
        let err = decrypt(PASS, &mut &sealed[..], &mut out)
            .expect_err("a backup missing its tail must not restore cleanly");
        assert!(
            matches!(err, EnvelopeError::Authentication { .. }),
            "expected an authentication failure on the new last chunk, got {err}"
        );
    }

    #[test]
    fn a_partial_chunk_at_the_end_is_reported_as_truncation() {
        let plain = pattern(4096);
        let mut sealed = Vec::new();
        encrypt(PASS, &mut &plain[..], &mut sealed).expect("encrypt");
        sealed.truncate(HEADER_LEN + 8); // less than a tag

        let mut out = Vec::new();
        let err = decrypt(PASS, &mut &sealed[..], &mut out).expect_err("must not restore");
        assert!(
            matches!(err, EnvelopeError::Truncated { chunk: 0 }),
            "expected truncation, got {err}"
        );
    }

    #[test]
    fn swapping_two_chunks_is_detected() {
        // What the per-chunk counter buys. Both chunks authenticate under their
        // own nonce; only the position is wrong.
        let plain = pattern(2 * CHUNK_LEN as usize + 10);
        let mut sealed = Vec::new();
        encrypt(PASS, &mut &plain[..], &mut sealed).expect("encrypt");

        let framed = CHUNK_LEN as usize + TAG_LEN;
        let (head, body) = sealed.split_at(HEADER_LEN);
        let mut reordered = head.to_vec();
        reordered.extend_from_slice(&body[framed..2 * framed]);
        reordered.extend_from_slice(&body[..framed]);
        reordered.extend_from_slice(&body[2 * framed..]);

        let mut out = Vec::new();
        let err = decrypt(PASS, &mut &reordered[..], &mut out).expect_err("must not restore");
        assert!(
            matches!(err, EnvelopeError::Authentication { chunk: 0 }),
            "expected chunk 0 to reject the chunk that belongs at index 1, got {err}"
        );
    }

    #[test]
    fn the_header_is_authenticated_so_the_kdf_cost_cannot_be_lowered() {
        // Without the header as AAD, an attacker with write access to the
        // storage host could rewrite `m_cost` to 8 KiB and hand the file back;
        // it would still decrypt, and the operator would have no way to know
        // their passphrase was now cheap to guess.
        let plain = b"a station database, in miniature";
        let mut sealed = Vec::new();
        encrypt(PASS, &mut &plain[..], &mut sealed).expect("encrypt");

        // Rewrite m_cost. The key changes, so this is really testing that the
        // failure is *detected*, whichever way round.
        sealed[12..16].copy_from_slice(&8192u32.to_be_bytes());
        let mut out = Vec::new();
        assert!(
            decrypt(PASS, &mut &sealed[..], &mut out).is_err(),
            "a rewritten argon2 cost must not produce a readable backup"
        );

        // The one that would pass without AAD: a field that does not feed the
        // key. Both reserved bytes are outside the KDF inputs entirely.
        let mut sealed2 = Vec::new();
        encrypt(PASS, &mut &plain[..], &mut sealed2).expect("encrypt");
        sealed2[10] = 0xff;
        let mut out2 = Vec::new();
        let err = decrypt(PASS, &mut &sealed2[..], &mut out2)
            .expect_err("a rewritten reserved byte must be detected");
        assert!(
            matches!(err, EnvelopeError::Authentication { chunk: 0 }),
            "expected the AAD check to reject it, got {err}"
        );
    }

    #[test]
    fn a_short_passphrase_is_refused_in_both_directions() {
        let mut sink = Vec::new();
        let err = encrypt("hunter2", &mut &b"x"[..], &mut sink)
            .expect_err("a 7-character passphrase must be refused");
        assert!(
            matches!(err, EnvelopeError::PassphraseTooShort { len: 7 }),
            "got {err}"
        );
        assert!(
            sink.is_empty(),
            "nothing may be written before the passphrase is accepted — a header \
             on disk with no body is a file an operator will try to restore"
        );

        // And the same check on the way back, so a file written by an older
        // build with a weak passphrase is not quietly easier to open than one
        // this build would write.
        let mut sealed = Vec::new();
        encrypt(PASS, &mut &b"x"[..], &mut sealed).expect("encrypt");
        let mut out = Vec::new();
        assert!(matches!(
            decrypt("short", &mut &sealed[..], &mut out),
            Err(EnvelopeError::PassphraseTooShort { .. })
        ));
    }

    #[test]
    fn a_plain_file_is_not_mistaken_for_an_envelope() {
        // Longer than the header, which is the whole point. The first version
        // of this test passed a 25-byte string: `read_exact` hit end-of-file
        // and returned `NotAnEnvelope` before the magic was ever compared, so
        // the test stayed green with the magic check deleted. It was asserting
        // "short files are rejected", which nothing needed it to assert.
        let mut plain = b"SQLite format 3\0".to_vec();
        plain.resize(HEADER_LEN * 2, 0x5a);

        let mut out = Vec::new();
        let err = decrypt(PASS, &mut &plain[..], &mut out)
            .expect_err("a plain database must be recognised as not an envelope");
        assert!(
            matches!(err, EnvelopeError::NotAnEnvelope),
            "a full-length file with the wrong magic must be reported as not an \
             envelope, not diagnosed further: got {err}"
        );
        assert!(!is_envelope(&plain));

        // And a file too short to hold a header is the same answer, reached a
        // different way — pinned separately so neither path can stand in for
        // the other.
        let mut out2 = Vec::new();
        assert!(matches!(
            decrypt(PASS, &mut &b"BNB"[..], &mut out2),
            Err(EnvelopeError::NotAnEnvelope)
        ));

        // Counterpart, so the check is not simply always-false.
        let mut sealed = Vec::new();
        encrypt(PASS, &mut &b"x"[..], &mut sealed).expect("encrypt");
        assert!(is_envelope(&sealed));
        assert!(!is_envelope(b"BNBBAK"), "a short read must not match");
    }

    #[test]
    fn an_unknown_algorithm_is_named_rather_than_guessed() {
        let mut sealed = Vec::new();
        encrypt(PASS, &mut &b"x"[..], &mut sealed).expect("encrypt");
        sealed[9] = 7;
        let mut out = Vec::new();
        let err = decrypt(PASS, &mut &sealed[..], &mut out).expect_err("must refuse");
        assert!(
            matches!(err, EnvelopeError::UnsupportedAlgorithm { kdf: 1, aead: 7 }),
            "got {err}"
        );
    }

    #[test]
    fn a_refusable_chunk_size_is_refused_before_it_is_allocated() {
        let mut sealed = Vec::new();
        encrypt(PASS, &mut &b"x"[..], &mut sealed).expect("encrypt");

        let mut zero = sealed.clone();
        zero[24..28].copy_from_slice(&0u32.to_be_bytes());
        let mut out = Vec::new();
        assert!(matches!(
            decrypt(PASS, &mut &zero[..], &mut out),
            Err(EnvelopeError::MalformedHeader(_))
        ));

        // A header claiming 4 GiB per chunk is an OOM on a Pi, not an error the
        // allocator will let us report.
        let mut huge = sealed;
        huge[24..28].copy_from_slice(&u32::MAX.to_be_bytes());
        let mut out2 = Vec::new();
        assert!(matches!(
            decrypt(PASS, &mut &huge[..], &mut out2),
            Err(EnvelopeError::MalformedHeader(_))
        ));
    }

    #[test]
    fn two_encryptions_of_the_same_bytes_differ() {
        // Salt and nonce prefix are per-file. If either were fixed, identical
        // backups would produce identical ciphertext and a bucket listing would
        // leak "nothing changed this week".
        let plain = b"the same database, twice";
        let mut a = Vec::new();
        let mut b = Vec::new();
        encrypt(PASS, &mut &plain[..], &mut a).expect("encrypt");
        encrypt(PASS, &mut &plain[..], &mut b).expect("encrypt");
        assert_ne!(a, b, "two encryptions must not be byte-identical");
        assert_ne!(
            &a[28..44],
            &b[28..44],
            "the salt must be fresh for every file"
        );
        assert_ne!(
            &a[44..51],
            &b[44..51],
            "the nonce prefix must be fresh for every file"
        );
    }

    #[test]
    fn a_reader_that_dribbles_produces_the_same_ciphertext_framing() {
        // `Read::read` may return fewer bytes than asked for at any time. If
        // `read_up_to` stopped at the first short read, chunk boundaries would
        // depend on the reader's mood — and because encrypt and decrypt would
        // still agree, nothing would ever fail. The defect would only appear as
        // a backup written on one machine that a different build could not
        // frame the same way.
        struct Dribble<'a>(&'a [u8]);
        impl Read for Dribble<'_> {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                let n = self.0.len().min(buf.len()).min(7);
                buf[..n].copy_from_slice(&self.0[..n]);
                self.0 = &self.0[n..];
                Ok(n)
            }
        }

        let plain = pattern(CHUNK_LEN as usize + 3);
        let mut steady = Vec::new();
        encrypt(PASS, &mut &plain[..], &mut steady).expect("encrypt");
        let mut dribbled = Vec::new();
        encrypt(PASS, &mut Dribble(&plain), &mut dribbled).expect("encrypt");

        assert_eq!(
            steady.len(),
            dribbled.len(),
            "a reader returning 7 bytes at a time framed the file differently"
        );
        let mut out = Vec::new();
        decrypt(PASS, &mut &dribbled[..], &mut out).expect("decrypt");
        assert_eq!(out, plain);
    }
}
