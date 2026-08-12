//! In-process live-audio taps: a bounded, lossy ring buffer per capture source.
//!
//! # Why this exists
//!
//! A USB microphone opened through ALSA's `plughw:` is an **exclusive** device.
//! While capture holds it, a second opener gets `EBUSY` — confirmed on a
//! Raspberry Pi 4, where `ffmpeg -f alsa -i plughw:CARD=PRO,DEV=0` fails with
//! `Device or resource busy` for as long as `arecord` is recording. The live
//! `/stream` endpoint used to do exactly that second open, so on a
//! single-microphone station — the overwhelmingly common build — live audio
//! could never work at all.
//!
//! The fix is to open the device once and split the stream in-process: the
//! capture reader pushes every PCM chunk it writes to disk into a [`LiveTap`],
//! and `/stream` subscribes to that instead of touching the device.
//!
//! # Recording always wins
//!
//! A tap is a **bounded** ring that is **lossy on overflow**: [`LiveTap::push`]
//! never blocks and never fails, so a listener that stops reading (a stalled
//! browser, a wedged TCP connection, a paused `curl`) can never apply
//! backpressure to capture. When a subscriber falls further behind than the ring
//! is deep, its cursor is fast-forwarded to the oldest byte still held and the
//! skipped bytes are counted. Losing a second of *live monitoring* audio is a
//! click in someone's headphones; losing a second of *recorded* audio is a
//! detection that never happens.

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, RwLock};
use std::time::{Duration, Instant};

/// Layout of the raw PCM carried by a [`LiveTap`].
///
/// Always signed 16-bit little-endian interleaved samples — the format capture
/// asks `arecord` for (`-f S16_LE`) and the format `/stream` hands to ffmpeg
/// (`-f s16le`). The bit depth is therefore not a field: it is the one thing
/// both ends already agree on, and making it configurable would create a way
/// for them to disagree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PcmSpec {
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// Interleaved channel count.
    pub channels: u16,
}

/// Bytes per sample in the S16 format a tap carries.
pub(crate) const BYTES_PER_SAMPLE: usize = 2;

impl PcmSpec {
    /// Bytes one second of this stream occupies.
    ///
    /// Saturates rather than overflowing on absurd configuration; a spec that
    /// large is rejected by the ring-capacity clamp anyway.
    #[must_use]
    pub const fn bytes_per_second(self) -> usize {
        (self.sample_rate as usize)
            .saturating_mul(self.channels as usize)
            .saturating_mul(BYTES_PER_SAMPLE)
    }

    /// Bytes one frame (one sample on every channel) occupies.
    #[must_use]
    pub const fn bytes_per_frame(self) -> usize {
        (self.channels as usize).saturating_mul(BYTES_PER_SAMPLE)
    }
}

/// How much audio a tap holds. Purely a jitter cushion: subscribers start at the
/// live edge, so ring depth is never added to stream latency — it is only how
/// far behind a consumer may fall before it starts losing bytes. Four seconds is
/// orders of magnitude more slack than an ffmpeg pipe on a local socket needs,
/// and still under half a megabyte for the 48 kHz mono capture every supported
/// microphone source uses.
const RING_SECONDS: usize = 4;

/// Floor on ring capacity, so a nonsensically small spec still buffers
/// something useful.
const RING_MIN_BYTES: usize = 64 * 1024;

/// Ceiling on ring capacity. A Raspberry Pi's RAM is the scarce resource on the
/// target hardware and this memory is resident for the life of the process, so
/// a sample rate read wrongly must not be able to reserve an arbitrary slab.
const RING_MAX_BYTES: usize = 4 * 1024 * 1024;

/// Ring capacity for `spec`, clamped to a sane band.
fn ring_capacity(spec: PcmSpec) -> usize {
    spec.bytes_per_second()
        .saturating_mul(RING_SECONDS)
        .clamp(RING_MIN_BYTES, RING_MAX_BYTES)
}

/// The mutable half of a tap: the wrapping byte buffer plus the absolute
/// write position that gives every byte ever produced a stable sequence number.
struct Ring {
    /// Fixed-capacity wrapping storage. Never resized after construction.
    buf: Vec<u8>,
    /// Total bytes ever pushed. The byte at absolute position `p` lives at
    /// `buf[p % buf.len()]` while `p >= write_pos - buf.len()`.
    write_pos: u64,
    /// When the last push landed, for [`LiveTap::silent_for`].
    last_push: Option<Instant>,
}

impl Ring {
    fn new(capacity: usize) -> Self {
        Self {
            buf: vec![0; capacity],
            write_pos: 0,
            last_push: None,
        }
    }

    /// Absolute position of the oldest byte still retrievable.
    const fn oldest(&self) -> u64 {
        self.write_pos.saturating_sub(self.buf.len() as u64)
    }

    // `write_pos % cap` and `available.min(out.len())` are both bounded by a
    // `usize` (`cap`, `out.len()`), so neither can lose information on a 32-bit
    // target — the bound is the whole point of the modulus.
    #[allow(clippy::cast_possible_truncation)]
    fn push(&mut self, data: &[u8], now: Instant) {
        let cap = self.buf.len();
        if cap == 0 || data.is_empty() {
            return;
        }
        // A single push larger than the ring can only leave its tail behind.
        // Account for the discarded head by advancing the write position, so
        // `write_pos` stays "bytes ever produced" and subscribers see the loss.
        let skip = data.len().saturating_sub(cap);
        self.write_pos = self.write_pos.saturating_add(skip as u64);
        let data = &data[skip..];

        let start = (self.write_pos % cap as u64) as usize;
        let first = data.len().min(cap - start);
        self.buf[start..start + first].copy_from_slice(&data[..first]);
        if first < data.len() {
            self.buf[..data.len() - first].copy_from_slice(&data[first..]);
        }
        self.write_pos = self.write_pos.saturating_add(data.len() as u64);
        self.last_push = Some(now);
    }

    /// Copy up to `out.len()` bytes from absolute position `pos`, advancing it.
    ///
    /// Returns `(bytes copied, bytes skipped because they had already been
    /// overwritten)`.
    #[allow(clippy::cast_possible_truncation)]
    fn read(&self, pos: &mut u64, out: &mut [u8]) -> (usize, u64) {
        let cap = self.buf.len();
        if cap == 0 {
            return (0, 0);
        }
        let oldest = self.oldest();
        let dropped = if *pos < oldest {
            let gap = oldest - *pos;
            *pos = oldest;
            gap
        } else {
            0
        };
        let available = self.write_pos.saturating_sub(*pos);
        let n = available.min(out.len() as u64) as usize;
        if n > 0 {
            let start = (*pos % cap as u64) as usize;
            let first = n.min(cap - start);
            out[..first].copy_from_slice(&self.buf[start..start + first]);
            if first < n {
                out[first..n].copy_from_slice(&self.buf[..n - first]);
            }
            *pos += n as u64;
        }
        (n, dropped)
    }
}

/// One capture source's live PCM, shared between the capture reader thread that
/// fills it and any number of `/stream` subscribers that drain it.
///
/// Created and looked up through a [`LiveAudioHub`]; see the module docs for why
/// it is bounded and lossy.
pub struct LiveTap {
    spec: PcmSpec,
    ring: Mutex<Ring>,
    /// Signalled after every push so blocked subscribers wake immediately
    /// instead of polling.
    data_ready: Condvar,
}

// The ring holds hundreds of kilobytes of PCM and the condvar carries nothing
// worth printing, so this deliberately summarises instead of listing fields:
// the derived impl would dump the entire buffer into a log line.
#[allow(clippy::missing_fields_in_debug)]
impl std::fmt::Debug for LiveTap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (capacity, produced) = self.with_ring(|ring| (ring.buf.len(), ring.write_pos));
        f.debug_struct("LiveTap")
            .field("spec", &self.spec)
            .field("capacity_bytes", &capacity)
            .field("bytes_produced", &produced)
            .finish()
    }
}

impl LiveTap {
    /// A tap sized for `spec`, holding no audio yet.
    #[must_use]
    pub fn new(spec: PcmSpec) -> Self {
        Self {
            spec,
            ring: Mutex::new(Ring::new(ring_capacity(spec))),
            data_ready: Condvar::new(),
        }
    }

    /// Run `f` against the ring, recovering from a poisoned lock rather than
    /// panicking.
    ///
    /// A subscriber that panics mid-read must not be able to wedge capture: the
    /// ring is plain bytes plus two counters, so the worst a poisoned lock can
    /// mean here is a partially-copied read buffer that its owner already
    /// abandoned.
    fn with_ring<T>(&self, f: impl FnOnce(&mut Ring) -> T) -> T {
        let mut guard = self
            .ring
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        f(&mut guard)
    }

    /// The PCM layout every subscriber will receive.
    #[must_use]
    pub const fn spec(&self) -> PcmSpec {
        self.spec
    }

    /// Publish a chunk of PCM. Never blocks and never fails.
    ///
    /// Called from the capture reader thread for every chunk it also writes to
    /// disk, so what a listener hears is what the detector will classify —
    /// including any capture gain, which is applied upstream of this call.
    pub fn push(&self, pcm: &[u8]) {
        if pcm.is_empty() {
            return;
        }
        let now = Instant::now();
        self.with_ring(|ring| ring.push(pcm, now));
        // Wake every waiter: several `/stream` clients may share one source.
        self.data_ready.notify_all();
    }

    /// How long since audio was last pushed, or `None` if none ever was.
    ///
    /// `/stream` uses this to answer "is this source actually recording right
    /// now?" before committing to a response — a paused or dead source
    /// otherwise yields a connection that hangs open producing silence.
    #[must_use]
    pub fn silent_for(&self) -> Option<Duration> {
        self.with_ring(|ring| ring.last_push.map(|t| t.elapsed()))
    }

    /// Total bytes this tap has ever been handed.
    #[must_use]
    pub fn bytes_produced(&self) -> u64 {
        self.with_ring(|ring| ring.write_pos)
    }
}

/// A subscriber's position in a [`LiveTap`], plus its own loss accounting.
///
/// Obtained from [`LiveAudioHub::subscribe`] or [`LiveTap::subscribe`]. Starts
/// at the **live edge**, not the oldest buffered byte, so a listener joins
/// "now" rather than replaying the ring.
#[derive(Debug)]
pub struct LiveSubscription {
    tap: Arc<LiveTap>,
    pos: u64,
    dropped: u64,
}

impl LiveTap {
    /// Subscribe at the live edge.
    #[must_use]
    pub fn subscribe(self: &Arc<Self>) -> LiveSubscription {
        let pos = self.with_ring(|ring| ring.write_pos);
        LiveSubscription {
            tap: Arc::clone(self),
            pos,
            dropped: 0,
        }
    }
}

impl LiveSubscription {
    /// Fill `out` with as much PCM as is available, waiting up to `timeout` for
    /// the first byte.
    ///
    /// Returns the number of bytes written to `out`; `0` means the timeout
    /// elapsed with the source silent (paused, dead, or simply between periods).
    /// Callers loop on this, so a `0` is a chance to notice their client has
    /// gone away rather than an error.
    pub fn read(&mut self, out: &mut [u8], timeout: Duration) -> usize {
        if out.is_empty() {
            return 0;
        }
        let deadline = Instant::now() + timeout;
        let mut guard = self
            .tap
            .ring
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        loop {
            let (n, dropped) = guard.read(&mut self.pos, out);
            self.dropped = self.dropped.saturating_add(dropped);
            if n > 0 {
                return n;
            }
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                return 0;
            };
            guard = wait_timeout(&self.tap.data_ready, guard, remaining);
        }
    }

    /// Bytes this subscriber never saw because it fell further behind than the
    /// ring is deep. Monotonic for the life of the subscription.
    #[must_use]
    pub const fn dropped_bytes(&self) -> u64 {
        self.dropped
    }

    /// The PCM layout of the bytes [`Self::read`] yields.
    #[must_use]
    pub fn spec(&self) -> PcmSpec {
        self.tap.spec
    }
}

/// `Condvar::wait_timeout` that recovers from poisoning instead of panicking,
/// matching [`LiveTap::with_ring`]'s stance.
fn wait_timeout<'a>(
    condvar: &Condvar,
    guard: MutexGuard<'a, Ring>,
    timeout: Duration,
) -> MutexGuard<'a, Ring> {
    condvar
        .wait_timeout(guard, timeout)
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .0
}

/// Process-wide registry of live taps, keyed by capture-source label.
///
/// The key is [`crate::audio::capture::CaptureSource::label`] — the same string
/// the health gauge, the published capture status and the recording filenames
/// use — so "which source am I listening to" has exactly one answer across the
/// whole station.
#[derive(Debug, Default)]
pub struct LiveAudioHub {
    taps: RwLock<HashMap<String, Arc<LiveTap>>>,
}

/// Shared handle to the process's [`LiveAudioHub`].
pub type LiveAudioHubHandle = Arc<LiveAudioHub>;

/// Create an empty [`LiveAudioHubHandle`].
#[must_use]
pub fn new_live_audio_hub() -> LiveAudioHubHandle {
    Arc::new(LiveAudioHub::default())
}

impl LiveAudioHub {
    /// Get (or create) the tap for `label`, for a capture process to push into.
    ///
    /// Idempotent across capture restarts: the supervisor respawns a source
    /// many times over a deployment, and each respawn must keep filling the
    /// **same** tap so subscribers survive a restart instead of being stranded
    /// on an orphaned ring.
    ///
    /// A `spec` that differs from the existing tap's replaces the tap, because
    /// the sample rate a subscriber's decoder was configured with no longer
    /// describes the bytes. That needs a configuration change *and* a capture
    /// restart inside one process lifetime — sources are resolved once at
    /// startup — so it is logged rather than smoothed over.
    pub fn tap(&self, label: &str, spec: PcmSpec) -> Arc<LiveTap> {
        if let Some(existing) = self.lookup(label) {
            if existing.spec == spec {
                return existing;
            }
            tracing::warn!(
                source = label,
                old_rate = existing.spec.sample_rate,
                new_rate = spec.sample_rate,
                "live audio tap replaced because the capture format changed"
            );
        }
        let tap = Arc::new(LiveTap::new(spec));
        self.with_taps_mut(|taps| {
            taps.insert(label.to_owned(), Arc::clone(&tap));
        });
        tap
    }

    /// Look up an existing tap without creating one.
    #[must_use]
    pub fn lookup(&self, label: &str) -> Option<Arc<LiveTap>> {
        self.with_taps(|taps| taps.get(label).map(Arc::clone))
    }

    /// Subscribe to `label`'s live audio at the live edge, or `None` when that
    /// source has no in-process tap (it is not a teed source, or capture has
    /// never started for it).
    #[must_use]
    pub fn subscribe(&self, label: &str) -> Option<LiveSubscription> {
        self.lookup(label).map(|tap| tap.subscribe())
    }

    /// Labels that currently have a tap, for diagnostics.
    #[must_use]
    pub fn labels(&self) -> Vec<String> {
        let mut labels = self.with_taps(|taps| taps.keys().cloned().collect::<Vec<_>>());
        labels.sort();
        labels
    }

    fn with_taps<T>(&self, f: impl FnOnce(&HashMap<String, Arc<LiveTap>>) -> T) -> T {
        let guard = self
            .taps
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        f(&guard)
    }

    fn with_taps_mut<T>(&self, f: impl FnOnce(&mut HashMap<String, Arc<LiveTap>>) -> T) -> T {
        let mut guard = self
            .taps
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        f(&mut guard)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MONO_48K: PcmSpec = PcmSpec {
        sample_rate: 48_000,
        channels: 1,
    };

    fn tap_with_capacity(capacity: usize) -> Arc<LiveTap> {
        let tap = LiveTap {
            spec: MONO_48K,
            ring: Mutex::new(Ring::new(capacity)),
            data_ready: Condvar::new(),
        };
        Arc::new(tap)
    }

    /// Zero timeout: read whatever is already buffered and return immediately.
    fn read_now(sub: &mut LiveSubscription, out: &mut [u8]) -> usize {
        sub.read(out, Duration::ZERO)
    }

    /// Whatever is already buffered, as an owned `Vec` — keeps assertions free
    /// of the borrow gymnastics an in-place `&out[..read(&mut out)]` needs.
    fn drain_now(sub: &mut LiveSubscription) -> Vec<u8> {
        let mut buf = [0u8; 64];
        let n = read_now(sub, &mut buf);
        buf[..n].to_vec()
    }

    #[test]
    fn spec_arithmetic() {
        assert_eq!(MONO_48K.bytes_per_second(), 96_000);
        assert_eq!(MONO_48K.bytes_per_frame(), 2);
        let stereo = PcmSpec {
            sample_rate: 44_100,
            channels: 2,
        };
        assert_eq!(stereo.bytes_per_second(), 176_400);
        assert_eq!(stereo.bytes_per_frame(), 4);
    }

    #[test]
    fn ring_capacity_is_clamped_both_ways() {
        // 48 kHz mono for 4 s = 384 000 bytes, inside the band.
        assert_eq!(ring_capacity(MONO_48K), 384_000);
        // A tiny spec is floored, not left useless.
        let tiny = PcmSpec {
            sample_rate: 8_000,
            channels: 1,
        };
        assert_eq!(ring_capacity(tiny), RING_MIN_BYTES);
        // An absurd spec is capped, not honoured.
        let absurd = PcmSpec {
            sample_rate: u32::MAX,
            channels: 8,
        };
        assert_eq!(ring_capacity(absurd), RING_MAX_BYTES);
    }

    #[test]
    fn subscriber_reads_what_was_pushed_after_it_joined() {
        let tap = tap_with_capacity(1024);
        let mut sub = tap.subscribe();
        // Nothing pushed yet.
        let mut out = [0u8; 64];
        assert_eq!(read_now(&mut sub, &mut out), 0);

        tap.push(b"hello");
        let n = read_now(&mut sub, &mut out);
        assert_eq!(&out[..n], b"hello");
        // Drained.
        assert_eq!(read_now(&mut sub, &mut out), 0);
    }

    #[test]
    fn a_subscriber_joins_at_the_live_edge_not_the_ring_start() {
        let tap = tap_with_capacity(1024);
        tap.push(b"already-recorded-audio");
        // Joining after the fact must NOT replay the buffered history —
        // otherwise every listener starts several seconds in the past.
        let mut sub = tap.subscribe();
        let mut out = [0u8; 64];
        assert_eq!(read_now(&mut sub, &mut out), 0);
        tap.push(b"live");
        let n = read_now(&mut sub, &mut out);
        assert_eq!(&out[..n], b"live");
    }

    #[test]
    fn reads_wrap_around_the_ring() {
        // Capacity 8: push across the wrap boundary and read it back intact.
        let tap = tap_with_capacity(8);
        let mut sub = tap.subscribe();
        tap.push(b"abcde");
        tap.push(b"fgh");
        let mut out = [0u8; 8];
        let n = read_now(&mut sub, &mut out);
        assert_eq!(&out[..n], b"abcdefgh");
        // The next push wraps to the front of the storage.
        tap.push(b"ij");
        let n = read_now(&mut sub, &mut out);
        assert_eq!(&out[..n], b"ij");
    }

    /// The core guarantee: a subscriber that stops reading loses *its own*
    /// bytes and nothing else. The tap keeps accepting pushes at full speed.
    #[test]
    fn a_lagging_subscriber_is_fast_forwarded_and_loses_only_its_own_bytes() {
        let tap = tap_with_capacity(8);
        let mut sub = tap.subscribe();
        // Push 24 bytes into an 8-byte ring without reading: 16 bytes are gone.
        for chunk in [b"abcdefgh", b"ijklmnop", b"qrstuvwx"] {
            tap.push(chunk);
        }
        let mut out = [0u8; 16];
        let n = read_now(&mut sub, &mut out);
        assert_eq!(&out[..n], b"qrstuvwx", "only the newest ring-full survives");
        assert_eq!(sub.dropped_bytes(), 16, "the skipped bytes are accounted");
        // The subscription keeps working afterwards.
        tap.push(b"yz");
        let n = read_now(&mut sub, &mut out);
        assert_eq!(&out[..n], b"yz");
        assert_eq!(sub.dropped_bytes(), 16, "no further loss once caught up");
    }

    #[test]
    fn a_push_larger_than_the_ring_keeps_its_tail_and_counts_the_head_as_lost() {
        let tap = tap_with_capacity(4);
        let mut sub = tap.subscribe();
        tap.push(b"abcdefghij"); // 10 bytes into a 4-byte ring
        let mut out = [0u8; 16];
        let n = read_now(&mut sub, &mut out);
        assert_eq!(&out[..n], b"ghij");
        assert_eq!(sub.dropped_bytes(), 6);
        assert_eq!(tap.bytes_produced(), 10, "write position counts every byte");
    }

    #[test]
    fn two_subscribers_are_independent() {
        let tap = tap_with_capacity(1024);
        let mut fast = tap.subscribe();
        let mut slow = tap.subscribe();
        tap.push(b"one");
        assert_eq!(drain_now(&mut fast), b"one");
        tap.push(b"two");
        // `slow` has read nothing yet, so it still sees both chunks; `fast`
        // sees only what arrived after its last read.
        assert_eq!(drain_now(&mut slow), b"onetwo");
        assert_eq!(drain_now(&mut fast), b"two");
    }

    #[test]
    fn read_blocks_until_a_push_arrives_then_returns_it() {
        let tap = tap_with_capacity(1024);
        let mut sub = tap.subscribe();
        let writer = Arc::clone(&tap);
        let handle = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            writer.push(b"late");
        });
        let mut out = [0u8; 16];
        // Generous timeout so a loaded CI runner can't flake this.
        let n = sub.read(&mut out, Duration::from_secs(5));
        assert_eq!(&out[..n], b"late");
        handle.join().expect("writer thread");
    }

    #[test]
    fn read_returns_zero_when_the_source_is_silent() {
        let tap = tap_with_capacity(1024);
        let mut sub = tap.subscribe();
        let mut out = [0u8; 16];
        let start = Instant::now();
        assert_eq!(sub.read(&mut out, Duration::from_millis(30)), 0);
        assert!(
            start.elapsed() >= Duration::from_millis(20),
            "a silent source must wait for the timeout, not spin"
        );
    }

    #[test]
    fn silent_for_tracks_pushes() {
        let tap = tap_with_capacity(1024);
        assert!(tap.silent_for().is_none(), "no audio has ever arrived");
        tap.push(b"x");
        assert!(tap.silent_for().is_some_and(|d| d < Duration::from_secs(1)));
    }

    #[test]
    fn empty_pushes_and_reads_are_no_ops() {
        let tap = tap_with_capacity(16);
        let mut sub = tap.subscribe();
        tap.push(b"");
        assert_eq!(tap.bytes_produced(), 0);
        assert!(tap.silent_for().is_none());
        let mut empty: [u8; 0] = [];
        assert_eq!(sub.read(&mut empty, Duration::from_millis(1)), 0);
    }

    // ---- hub ---------------------------------------------------------------

    #[test]
    fn hub_creates_then_reuses_a_tap_per_label() {
        let hub = new_live_audio_hub();
        let first = hub.tap("src_seed_1", MONO_48K);
        let second = hub.tap("src_seed_1", MONO_48K);
        assert!(
            Arc::ptr_eq(&first, &second),
            "a capture restart must keep filling the same tap, or every \
             subscriber is stranded on an orphaned ring"
        );
        assert_eq!(hub.labels(), vec!["src_seed_1".to_string()]);
    }

    #[test]
    fn hub_replaces_a_tap_whose_format_changed() {
        let hub = new_live_audio_hub();
        let first = hub.tap("src_seed_1", MONO_48K);
        let second = hub.tap(
            "src_seed_1",
            PcmSpec {
                sample_rate: 44_100,
                channels: 1,
            },
        );
        assert!(!Arc::ptr_eq(&first, &second));
        assert_eq!(second.spec().sample_rate, 44_100);
    }

    #[test]
    fn hub_subscribe_is_none_for_an_unknown_label() {
        let hub = new_live_audio_hub();
        assert!(hub.subscribe("nope").is_none());
        assert!(hub.lookup("nope").is_none());
        hub.tap("src_seed_1", MONO_48K);
        assert!(hub.subscribe("src_seed_1").is_some());
        // Another source's label must not resolve to this tap.
        assert!(hub.subscribe("src_seed_2").is_none());
    }

    #[test]
    fn hub_keeps_sources_separate() {
        let hub = new_live_audio_hub();
        let a = hub.tap("src_a", MONO_48K);
        let b = hub.tap("src_b", MONO_48K);
        let mut sub_a = hub.subscribe("src_a").expect("tap a");
        let mut sub_b = hub.subscribe("src_b").expect("tap b");
        a.push(b"aaa");
        b.push(b"bbb");
        assert_eq!(drain_now(&mut sub_a), b"aaa");
        assert_eq!(drain_now(&mut sub_b), b"bbb");
        assert_eq!(hub.labels(), vec!["src_a".to_string(), "src_b".to_string()]);
    }

    /// Debug must stay terse — the derived impl would print the whole ring.
    #[test]
    fn debug_does_not_dump_the_buffer() {
        let tap = tap_with_capacity(4096);
        tap.push(&[7u8; 512]);
        let rendered = format!("{tap:?}");
        assert!(rendered.contains("capacity_bytes"));
        assert!(rendered.contains("bytes_produced"));
        assert!(
            rendered.len() < 200,
            "Debug must summarise, not dump: {rendered}"
        );
    }
}
