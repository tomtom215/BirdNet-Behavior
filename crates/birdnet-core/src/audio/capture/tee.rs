//! The in-process capture tee: one device open, two consumers.
//!
//! # The problem
//!
//! An ALSA `plughw:` microphone is an **exclusive** device. While `arecord`
//! holds it, a second opener is refused: on the Raspberry Pi 4 under test,
//! `ffmpeg -f alsa -i plughw:CARD=PRO,DEV=0` fails with `Device or resource
//! busy` for as long as capture is running. The live `/stream` endpoint did
//! exactly that second open, so on a single-microphone station — which is what
//! almost every build is — live audio could not work, and neither could
//! anything else that wanted to listen to the microphone.
//!
//! # The shape of the fix
//!
//! Open the device once and split the stream in-process:
//!
//! ```text
//!   arecord -t raw ──stdout──▶ [tee thread] ──▶ SegmentWriter  (rotating WAVs)
//!                                          └──▶ LiveTap        (bounded, lossy)
//! ```
//!
//! `arecord` stops segmenting for us — it just streams headerless PCM — and
//! [`super::segment::SegmentWriter`] takes over writing the files, with names
//! byte-identical to the ones `--use-strftime` produced. `/stream` subscribes to
//! the [`super::live::LiveTap`] instead of touching the device.
//!
//! # Recording is the priority, always
//!
//! The tap is bounded and lossy ([`super::live`]), so no listener can ever
//! backpressure the reader. The reader drains the producer's stdout on its own
//! thread, which is also what keeps the pipe from filling — a full pipe would
//! block `arecord` in `write(2)` while leaving it *alive*, the same silent-deaf
//! failure the stderr drainer exists to prevent.
//!
//! If the segment writer fails (a full disk, a vanished mount), the tee thread
//! stops. That is deliberate: [`super::process::CaptureProcess::is_running`]
//! reports the source as down, and the supervisor restarts it with backoff and
//! lights up the health gauge — rather than the station continuing to look
//! healthy while writing nothing.

use std::io::Read;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

use super::live::{BYTES_PER_SAMPLE, LiveTap, PcmSpec};
use super::segment::SegmentWriter;
use super::types::{AudioPipeline, DC_BLOCK_CUTOFF_HZ, HIGH_PASS_CUTOFF_HZ};
use crate::audio::eq::{EqChain, EqProcessor};

/// Size of the reader's staging buffer.
///
/// Not a latency knob: a pipe `read` returns as soon as the producer has
/// written anything, so live latency tracks `arecord`'s period size, not this.
/// It only bounds the syscall rate and the size of an individual disk write.
const READ_BUFFER_BYTES: usize = 16 * 1024;

/// Everything that shapes the samples between the capture tool and the split.
///
/// Bundled rather than passed as five parameters because they travel together
/// and always come from the same source row — and because a five-argument
/// audio path is where a caller eventually passes the sample rate where the
/// channel count belongs.
#[derive(Debug, Clone)]
pub struct Shaping {
    /// Software capture gain in dB; `0.0` is unity and costs nothing.
    pub gain_db: f32,
    /// Per-source conditioning: DC block, high-pass, AGC.
    pub pipeline: AudioPipeline,
    /// The operator's filter chain. Non-empty replaces the two high-passes in
    /// `pipeline`; `agc` is unaffected. Empty is the default and leaves this
    /// path byte-identical to what it was before the chain existed.
    pub eq: EqChain,
    /// The PCM format arriving from the capture tool, needed to size the
    /// filters' per-channel state and their coefficients.
    pub spec: PcmSpec,
    /// Which half of a stereo pair to keep, if the operator picked one.
    pub pick: Option<ChannelPick>,
}

/// Walks interleaved S16LE PCM sample by sample, tracking the channel index
/// and the odd byte a pipe read can split a sample across.
///
/// Factored out because the conditioning happens in two passes that sit on
/// opposite sides of the gain, and each needs its own buffer and its own
/// straddling remainder — sharing one would alias the borrow and, worse, would
/// hand the second pass the first pass's leftover byte.
#[derive(Debug)]
struct SampleWalker {
    /// Which channel the next sample belongs to. Carried across reads because
    /// a chunk boundary does not respect frame boundaries.
    channel: usize,
    channels: usize,
    carry: Option<u8>,
    out: Vec<u8>,
}

impl SampleWalker {
    fn new(channels: usize) -> Self {
        Self {
            channel: 0,
            channels: channels.max(1),
            carry: None,
            out: Vec::with_capacity(READ_BUFFER_BYTES + BYTES_PER_SAMPLE),
        }
    }

    /// Apply `f(sample, channel)` to every whole sample in `input`.
    fn walk(&mut self, input: &[u8], mut f: impl FnMut(i16, usize) -> i16) -> &[u8] {
        self.out.clear();
        let mut rest = input;

        if let Some(low) = self.carry.take() {
            let Some((&high, tail)) = rest.split_first() else {
                self.carry = Some(low);
                return &self.out;
            };
            let v = f(i16::from_le_bytes([low, high]), self.channel);
            self.out.extend_from_slice(&v.to_le_bytes());
            self.channel = (self.channel + 1) % self.channels;
            rest = tail;
        }

        let aligned = rest.len() - rest.len() % BYTES_PER_SAMPLE;
        for pair in rest[..aligned].as_chunks::<BYTES_PER_SAMPLE>().0 {
            let v = f(i16::from_le_bytes([pair[0], pair[1]]), self.channel);
            self.out.extend_from_slice(&v.to_le_bytes());
            self.channel = (self.channel + 1) % self.channels;
        }
        if aligned < rest.len() {
            self.carry = Some(rest[aligned]);
        }
        &self.out
    }
}

/// Convert one 16-bit sample to normalised float and back, applying `f`.
#[allow(
    clippy::cast_possible_truncation,
    reason = "the value is clamped into i16's range immediately before the cast"
)]
fn through_float(sample: i16, f: impl FnOnce(f32) -> f32) -> i16 {
    let y = f(f32::from(sample) / f32::from(i16::MAX));
    (y * f32::from(i16::MAX))
        .clamp(f32::from(i16::MIN), f32::from(i16::MAX))
        .round() as i16
}

/// The conditioning that runs **before** the gain: DC block, then high-pass.
///
/// The teed microphone path is `arecord → this process → segments`, with no
/// ffmpeg anywhere, so the `-af` chain the RTSP and `PipeWire` sources get has
/// to be reproduced here — otherwise the per-source toggles would apply to
/// network cameras and silently not to the microphone plugged into the Pi,
/// which is the commonest deployment there is.
///
/// State is per channel and carried across reads: a filter that reset every
/// chunk would put a step at every buffer boundary, audible as a click and
/// worse for the quality gate than the rumble it removes.
#[derive(Debug)]
struct PreConditioner {
    stages: PreStages,
    walker: SampleWalker,
}

/// Which of the two conditioning designs this source is running.
///
/// An enum rather than three `Option` fields because they are exclusive: an
/// operator's chain *replaces* the fixed high-passes, and a shape that could
/// hold both would eventually hold both.
#[derive(Debug)]
enum PreStages {
    /// The two fixed one-pole high-passes selected by the pipeline flags.
    Flags {
        /// One DC blocker per channel, when `dc_removal` is on.
        dc: Option<Vec<OnePoleHighPass>>,
        /// One high-pass per channel, when `high_pass` is on.
        hp: Option<Vec<OnePoleHighPass>>,
    },
    /// The operator's chain, one independent processor per channel.
    Chain(Vec<EqProcessor>),
}

impl PreConditioner {
    /// `None` when nothing is enabled, so the untouched path stays byte-exact
    /// and allocation-free — the contract [`Gain`] keeps at unity.
    ///
    /// A chain that cannot be built at this sample rate (a stage above Nyquist
    /// on a 16 kHz source, say) falls back to the flags rather than failing.
    /// A station is better off recording with the wrong filter than not
    /// recording, and the fallback is loud in the log.
    fn new(
        pipeline: AudioPipeline,
        eq: &EqChain,
        sample_rate: u32,
        channels: usize,
    ) -> Option<Self> {
        let lanes = channels.max(1);
        if !eq.is_empty() {
            match eq.build(sample_rate) {
                Ok(proto) => {
                    return Some(Self {
                        stages: PreStages::Chain(vec![proto; lanes]),
                        walker: SampleWalker::new(channels),
                    });
                }
                Err(e) => tracing::warn!(
                    error = %e,
                    spec = %eq.to_spec(),
                    sample_rate,
                    "equaliser chain rejected at this sample rate; using the pipeline flags"
                ),
            }
        }
        if !pipeline.dc_removal && !pipeline.high_pass {
            return None;
        }
        let per_channel = |cutoff| {
            (0..lanes)
                .map(|_| OnePoleHighPass::new(sample_rate, cutoff))
                .collect::<Vec<_>>()
        };
        Some(Self {
            stages: PreStages::Flags {
                dc: pipeline.dc_removal.then(|| per_channel(DC_BLOCK_CUTOFF_HZ)),
                hp: pipeline.high_pass.then(|| per_channel(HIGH_PASS_CUTOFF_HZ)),
            },
            walker: SampleWalker::new(channels),
        })
    }

    fn apply(&mut self, input: &[u8]) -> &[u8] {
        let Self { stages, walker } = self;
        match stages {
            PreStages::Flags { dc, hp } => walker.walk(input, |sample, ch| {
                through_float(sample, |mut x| {
                    if let Some(dc) = dc.as_mut() {
                        x = dc[ch].step(x);
                    }
                    if let Some(hp) = hp.as_mut() {
                        x = hp[ch].step(x);
                    }
                    x
                })
            }),
            PreStages::Chain(lanes) => walker.walk(input, |sample, ch| {
                through_float(sample, |x| lanes[ch].process(x))
            }),
        }
    }
}

/// The conditioning that runs **after** the gain: automatic gain control.
#[derive(Debug)]
struct AgcStage {
    agc: PeakAgc,
    walker: SampleWalker,
}

impl AgcStage {
    fn new(pipeline: AudioPipeline, sample_rate: u32, channels: usize) -> Option<Self> {
        pipeline.agc.then(|| Self {
            agc: PeakAgc::new(sample_rate),
            walker: SampleWalker::new(channels),
        })
    }

    fn apply(&mut self, input: &[u8]) -> &[u8] {
        let Self { agc, walker } = self;
        walker.walk(input, |sample, _ch| through_float(sample, |x| agc.step(x)))
    }
}

/// First-order IIR high-pass: `y[n] = α(y[n−1] + x[n] − x[n−1])`.
///
/// The same transfer function as `quality::rain_detector`'s block-local
/// helper, but holding its state between calls so it can run on a stream.
#[derive(Debug, Clone, Copy)]
struct OnePoleHighPass {
    alpha: f32,
    prev_x: f32,
    prev_y: f32,
}

impl OnePoleHighPass {
    fn new(sample_rate: u32, cutoff_hz: f32) -> Self {
        let rc = 1.0 / (2.0 * std::f32::consts::PI * cutoff_hz);
        #[allow(clippy::cast_precision_loss)]
        let dt = 1.0 / sample_rate as f32;
        Self {
            alpha: rc / (rc + dt),
            prev_x: 0.0,
            prev_y: 0.0,
        }
    }

    fn step(&mut self, x: f32) -> f32 {
        let y = self.alpha * (self.prev_y + x - self.prev_x);
        self.prev_x = x;
        self.prev_y = y;
        y
    }
}

/// Peak-following automatic gain control.
///
/// Tracks the signal's recent peak and moves a gain factor towards the one
/// that would put that peak at [`AGC_TARGET_PEAK`]. Reacting downwards is
/// immediate so a sudden loud call cannot clip; recovering upwards is slow so
/// the gain does not ride up into the noise floor during the gaps between
/// songs — the failure that makes a naive AGC worse than none on a dawn
/// chorus. The gain is capped at [`AGC_MAX_GAIN`] so a silent channel cannot
/// be amplified into full-scale hiss.
#[derive(Debug)]
struct PeakAgc {
    peak: f32,
    gain: f32,
    attack: f32,
    release: f32,
}

/// Target peak for [`PeakAgc`], about −3 dBFS.
const AGC_TARGET_PEAK: f32 = 0.71;
/// Ceiling on [`PeakAgc`]'s gain (+20 dB).
const AGC_MAX_GAIN: f32 = 10.0;

impl PeakAgc {
    fn new(sample_rate: u32) -> Self {
        #[allow(clippy::cast_precision_loss)]
        let sr = sample_rate as f32;
        Self {
            peak: 0.0,
            gain: 1.0,
            // ~10 ms to follow a rise, ~2 s to fall back.
            attack: (-1.0 / (0.010 * sr)).exp(),
            release: (-1.0 / (2.000 * sr)).exp(),
        }
    }

    fn step(&mut self, x: f32) -> f32 {
        let mag = x.abs();
        self.peak = if mag > self.peak {
            (self.peak - mag).mul_add(self.attack, mag)
        } else {
            (self.peak - mag).mul_add(self.release, mag)
        };

        let want = if self.peak > 1e-6 {
            (AGC_TARGET_PEAK / self.peak).min(AGC_MAX_GAIN)
        } else {
            AGC_MAX_GAIN
        };
        self.gain = if want < self.gain {
            want
        } else {
            (self.gain - want).mul_add(self.release, want)
        };
        (x * self.gain).clamp(-1.0, 1.0)
    }
}

/// Software capture gain applied to interleaved S16LE PCM.
///
/// `arecord` has no gain control, which is why a gain-configured microphone
/// used to be routed through `ffmpeg -f alsa` and its `volume` filter instead.
/// Now that the samples pass through this process anyway, applying the gain
/// here removes that whole second capture backend — along with the mismatch it
/// carried, where [`super::process::required_tool`] still reported `arecord`
/// for a source that actually needed `ffmpeg`.
///
/// Gain is applied **once**, upstream of the split, so what a listener hears on
/// `/stream` is exactly what the detector will classify.
#[derive(Debug)]
struct Gain {
    /// Linear amplitude multiplier, `10^(dB/20)`.
    factor: f32,
    /// A sample split across two reads: pipes are byte streams and give no
    /// alignment guarantee, so the odd byte waits here for its partner.
    carry: Option<u8>,
    /// Scaled output, reused between chunks to keep the reader allocation-free.
    out: Vec<u8>,
}

impl Gain {
    fn new(gain_db: f32) -> Self {
        Self {
            factor: 10f32.powf(gain_db / 20.0),
            carry: None,
            out: Vec::with_capacity(READ_BUFFER_BYTES + BYTES_PER_SAMPLE),
        }
    }

    /// Scale one 16-bit sample, saturating at the format's limits exactly as
    /// ffmpeg's `volume` filter does for s16 — a boost loud enough to clip
    /// clips, rather than wrapping into a loud burst of noise.
    #[allow(
        clippy::cast_possible_truncation,
        reason = "the value is clamped into i16's range immediately before the cast"
    )]
    fn scale(&self, sample: i16) -> i16 {
        let scaled = f32::from(sample) * self.factor;
        scaled
            .clamp(f32::from(i16::MIN), f32::from(i16::MAX))
            .round() as i16
    }

    /// Return `input` scaled. The result may be one byte longer or shorter than
    /// the input, as a straddling sample is completed or deferred.
    fn apply(&mut self, input: &[u8]) -> &[u8] {
        self.out.clear();
        let mut rest = input;
        if let Some(low) = self.carry.take() {
            let Some((&high, tail)) = rest.split_first() else {
                // Still only half a sample — keep waiting.
                self.carry = Some(low);
                return &self.out;
            };
            self.out
                .extend_from_slice(&self.scale(i16::from_le_bytes([low, high])).to_le_bytes());
            rest = tail;
        }
        let aligned = rest.len() - rest.len() % BYTES_PER_SAMPLE;
        for pair in rest[..aligned].as_chunks::<BYTES_PER_SAMPLE>().0 {
            self.out.extend_from_slice(
                &self
                    .scale(i16::from_le_bytes([pair[0], pair[1]]))
                    .to_le_bytes(),
            );
        }
        if let Some(&odd) = rest[aligned..].first() {
            self.carry = Some(odd);
        }
        &self.out
    }
}

/// Which half of a stereo capture to keep.
///
/// The Audio admin page has always offered Mono / Left / Right / Stereo, but
/// Left and Right did nothing: both collapsed to `channels: 1` at the capture
/// source and were never distinguished again, so picking Right gave exactly
/// what Mono gave.
///
/// Selecting a channel is not a cosmetic choice. `Stereo` keeps both channels,
/// and the decoder mixes them to mono by averaging — which for a **spaced**
/// pair is a comb filter, not a noise reduction. Measured through this
/// project's own decode path with one wavefront reaching the capsules half a
/// period apart, averaging drops the signal by roughly 66 dB; a quarter period
/// costs 3 dB, and the notches move with the bird's direction. A coincident
/// pair is unaffected. Picking one channel is the mitigation, which is what
/// these options are for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelPick {
    /// Keep the first channel of an interleaved stereo stream.
    Left,
    /// Keep the second.
    Right,
}

impl ChannelPick {
    /// Byte offset of this channel within an interleaved S16 stereo frame.
    const fn byte_offset(self) -> usize {
        match self {
            Self::Left => 0,
            Self::Right => BYTES_PER_SAMPLE,
        }
    }
}

/// Bytes in one interleaved S16 stereo frame (two channels, two bytes each).
const STEREO_FRAME_BYTES: usize = 2 * BYTES_PER_SAMPLE;

/// Reduce an interleaved stereo S16LE stream to one channel by *selecting* it.
///
/// Applied upstream of the split, for the same reason the gain is: what a
/// listener hears on `/stream` is then exactly what the detector classifies,
/// and the segments on disk are already mono, so nothing downstream — decode,
/// spectrogram, clip extraction — needs to know a choice was made.
///
/// Carries a partial *frame* rather than a partial sample: a pipe can split a
/// read anywhere, and a stereo frame is four bytes. Dropping the remainder
/// instead would slip the channel alignment by one sample and silently swap
/// left for right for the rest of the stream.
#[derive(Debug)]
struct ChannelSelector {
    offset: usize,
    /// Bytes of an incomplete frame awaiting the rest of their read.
    carry: Vec<u8>,
    /// Selected output, reused between chunks to keep the reader
    /// allocation-free.
    out: Vec<u8>,
}

impl ChannelSelector {
    fn new(pick: ChannelPick) -> Self {
        Self {
            offset: pick.byte_offset(),
            carry: Vec::with_capacity(STEREO_FRAME_BYTES),
            out: Vec::with_capacity(READ_BUFFER_BYTES / 2 + STEREO_FRAME_BYTES),
        }
    }

    /// Return the selected channel's samples from `input`. The result is about
    /// half the input's length.
    fn apply(&mut self, input: &[u8]) -> &[u8] {
        self.out.clear();
        let mut rest = input;

        // Complete a frame straddling the previous read first.
        if !self.carry.is_empty() {
            let needed = STEREO_FRAME_BYTES - self.carry.len();
            let take = needed.min(rest.len());
            self.carry.extend_from_slice(&rest[..take]);
            rest = &rest[take..];
            if self.carry.len() < STEREO_FRAME_BYTES {
                return &self.out; // still incomplete
            }
            self.out
                .extend_from_slice(&self.carry[self.offset..self.offset + BYTES_PER_SAMPLE]);
            self.carry.clear();
        }

        let aligned = rest.len() - rest.len() % STEREO_FRAME_BYTES;
        for frame in rest[..aligned].as_chunks::<STEREO_FRAME_BYTES>().0 {
            self.out
                .extend_from_slice(&frame[self.offset..self.offset + BYTES_PER_SAMPLE]);
        }
        self.carry.extend_from_slice(&rest[aligned..]);
        &self.out
    }
}

/// A running capture tee: the reader thread plus its stop latch.
#[derive(Debug)]
pub struct Tee {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Tee {
    /// Whether the reader thread is still running.
    ///
    /// A tee that has exited means the source is not recording, whatever the
    /// producer process is doing — the supervisor treats it as death.
    pub fn is_alive(&self) -> bool {
        self.handle.as_ref().is_some_and(|h| !h.is_finished())
    }

    /// Ask the reader to stop and wait for it.
    ///
    /// The caller must kill the producer **first**: the reader spends its life
    /// blocked in `read`, and it is the resulting EOF — not this latch — that
    /// wakes it. The latch only covers the window between two reads.
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take()
            && handle.join().is_err()
        {
            tracing::warn!("capture tee thread panicked");
        }
    }
}

impl Drop for Tee {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Start a tee reading PCM from `source`.
///
/// `gain_db` is applied in-process; a value below the audible threshold skips
/// the sample loop entirely and the bytes are forwarded untouched, which is
/// what makes "the segments concatenate back to exactly what the device
/// produced" true on the unity-gain path.
///
/// `pick` selects one half of a stereo capture, and is applied *before* the
/// gain so the gain only walks the samples that survive.
///
/// # Errors
///
/// Returns the `io::Error` from spawning the reader thread.
pub fn spawn<R>(
    label: String,
    source: R,
    writer: SegmentWriter,
    tap: Option<Arc<LiveTap>>,
    shaping: Shaping,
) -> std::io::Result<Tee>
where
    R: Read + Send + 'static,
{
    let stop = Arc::new(AtomicBool::new(false));
    let stop_for_thread = Arc::clone(&stop);
    let handle = std::thread::Builder::new()
        .name("capture-tee".to_owned())
        .spawn(move || {
            run(
                &label,
                source,
                writer,
                tap.as_deref(),
                shaping,
                &stop_for_thread,
            );
        })?;
    Ok(Tee {
        stop,
        handle: Some(handle),
    })
}

/// The reader loop: drain `source`, fan each chunk out to both consumers.
fn run<R: Read>(
    label: &str,
    mut source: R,
    mut writer: SegmentWriter,
    tap: Option<&LiveTap>,
    shaping: Shaping,
    stop: &AtomicBool,
) {
    let Shaping {
        gain_db,
        pipeline,
        eq,
        spec,
        pick,
    } = shaping;
    let mut gain = super::process::gain_is_active(gain_db).then(|| Gain::new(gain_db));
    let channels = spec.channels as usize;
    let mut pre = PreConditioner::new(pipeline, &eq, spec.sample_rate, channels);
    let mut agc = AgcStage::new(pipeline, spec.sample_rate, channels);
    let mut selector = pick.map(ChannelSelector::new);
    let mut buf = vec![0u8; READ_BUFFER_BYTES];
    let mut bytes_seen: u64 = 0;

    while !stop.load(Ordering::Relaxed) {
        let read = match source.read(&mut buf) {
            Ok(0) => {
                tracing::debug!(source = label, bytes = bytes_seen, "capture stream ended");
                break;
            }
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => {
                tracing::warn!(source = label, error = %e, "capture stream read failed");
                break;
            }
        };
        bytes_seen += read as u64;

        // Gain, then channel selection. Both are linear and both sit upstream
        // of the split, so the order does not change what is written or heard;
        // this way round each stage borrows a different object and no
        // intermediate copy is needed. Gain's carry is a half-*sample* and the
        // selector's is a partial *frame*, so a read that ends anywhere is
        // reassembled correctly by whichever stage straddles it.
        // Conditioning (DC block, high-pass) → gain → AGC → channel selection.
        // The first three reproduce, in the same order, the `-af` chain
        // `process::audio_filter_chain` hands the ffmpeg-backed sources, so a
        // toggle means the same thing whichever backend a source happens to
        // use. Each stage owns its own buffer, so no borrow aliases another and
        // no intermediate copy is needed; each also carries its own straddling
        // remainder (half a sample for the sample stages, a partial frame for
        // the selector), so a read that ends anywhere is reassembled correctly
        // by whichever stage the boundary lands in.
        let raw = &buf[..read];
        let conditioned: &[u8] = pre.as_mut().map_or(raw, |p| p.apply(raw));
        let gained: &[u8] = gain.as_mut().map_or(conditioned, |g| g.apply(conditioned));
        let levelled: &[u8] = agc.as_mut().map_or(gained, |a| a.apply(gained));
        let payload: &[u8] = selector.as_mut().map_or(levelled, |s| s.apply(levelled));
        if payload.is_empty() {
            continue;
        }

        // The tap first: it cannot block or fail, so live audio is never held
        // up by a slow filesystem, and a disk error still reaches the listener
        // as silence rather than a stalled connection.
        if let Some(tap) = tap {
            tap.push(payload);
        }
        if let Err(e) = writer.write(payload) {
            // Stopping is the point — see the module docs. A source that
            // cannot write must look down, not healthy-but-silent.
            tracing::error!(
                source = label,
                error = %e,
                "capture segment write failed; stopping this source so the \
                 supervisor restarts it"
            );
            break;
        }
    }
    writer.finish();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::capture::live::PcmSpec;
    use crate::audio::capture::segment::SegmentClock;
    use crate::audio::capture::types::{AudioFormat, LocalOffset};
    use std::io::Write;
    use std::path::PathBuf;
    use std::sync::atomic::AtomicI64;
    use std::time::{Duration, Instant};

    /// 10 bytes/second, 2 bytes/frame — small enough that segment counts are
    /// obvious in assertions.
    const TINY: PcmSpec = PcmSpec {
        sample_rate: 5,
        channels: 1,
    };

    /// 2026-08-12 12:03:15 UTC.
    const T0: i64 = 1_786_536_195;

    struct Fixture {
        dir: tempfile::TempDir,
        clock: Arc<AtomicI64>,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                dir: tempfile::tempdir().expect("tempdir"),
                clock: Arc::new(AtomicI64::new(T0)),
            }
        }

        fn writer(&self, secs: u32) -> SegmentWriter {
            SegmentWriter::new(
                self.dir.path().to_path_buf(),
                Some("src_seed_1".to_owned()),
                AudioFormat::Wav,
                TINY,
                secs,
                LocalOffset::utc(),
                SegmentClock::Ticking(Arc::clone(&self.clock)),
            )
        }

        fn segments(&self) -> Vec<PathBuf> {
            let mut files: Vec<PathBuf> = std::fs::read_dir(self.dir.path())
                .expect("read_dir")
                .flatten()
                .map(|e| e.path())
                .collect();
            files.sort();
            files
        }

        /// Every segment's PCM payload, concatenated in filename order.
        fn recorded_pcm(&self) -> Vec<u8> {
            let mut out = Vec::new();
            for path in self.segments() {
                let bytes = std::fs::read(&path).expect("read segment");
                out.extend_from_slice(&bytes[44..]);
            }
            out
        }

        /// Block until `f` holds or the deadline passes.
        fn wait_for(what: &str, mut f: impl FnMut() -> bool) {
            let deadline = Instant::now() + Duration::from_secs(10);
            while Instant::now() < deadline {
                if f() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            panic!("timed out waiting for {what}");
        }
    }

    // ---- channel selection (pure) ------------------------------------------

    /// Build interleaved stereo S16LE from per-channel sample lists.
    fn interleave(left: &[i16], right: &[i16]) -> Vec<u8> {
        let mut out = Vec::with_capacity(left.len() * 4);
        for (l, r) in left.iter().zip(right) {
            out.extend_from_slice(&l.to_le_bytes());
            out.extend_from_slice(&r.to_le_bytes());
        }
        out
    }

    fn samples(bytes: &[u8]) -> Vec<i16> {
        bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|p| i16::from_le_bytes([p[0], p[1]]))
            .collect()
    }

    #[test]
    fn channel_selector_keeps_the_channel_it_names() {
        let stereo = interleave(&[1, 2, 3, 4], &[-1, -2, -3, -4]);

        let mut left = ChannelSelector::new(ChannelPick::Left);
        assert_eq!(samples(left.apply(&stereo)), vec![1, 2, 3, 4]);

        let mut right = ChannelSelector::new(ChannelPick::Right);
        assert_eq!(samples(right.apply(&stereo)), vec![-1, -2, -3, -4]);
    }

    /// A read that ends mid-frame must not slip the channel alignment.
    ///
    /// A pipe splits wherever it likes. Discarding the partial frame instead of
    /// carrying it would shift every later frame by one sample and silently
    /// swap left for right for the rest of the capture — audible to nobody,
    /// visible in no log, and wrong for as long as the process runs.
    #[test]
    fn channel_selector_carries_a_frame_split_across_reads() {
        let stereo = interleave(&[10, 20, 30, 40], &[-10, -20, -30, -40]);

        // Every split point, including the three that land mid-frame.
        for split in 1..stereo.len() {
            let mut sel = ChannelSelector::new(ChannelPick::Right);
            let mut got = samples(sel.apply(&stereo[..split]));
            got.extend(samples(sel.apply(&stereo[split..])));
            assert_eq!(
                got,
                vec![-10, -20, -30, -40],
                "right channel wrong when the stream is split at byte {split}"
            );
        }
    }

    /// Byte-at-a-time delivery is the pathological case of the above.
    #[test]
    fn channel_selector_survives_one_byte_reads() {
        let stereo = interleave(&[7, 8, 9], &[-7, -8, -9]);
        let mut sel = ChannelSelector::new(ChannelPick::Left);
        let mut got = Vec::new();
        for byte in &stereo {
            got.extend(samples(sel.apply(std::slice::from_ref(byte))));
        }
        assert_eq!(got, vec![7, 8, 9]);
    }

    /// Selection halves the stream: two channels in, one out.
    #[test]
    fn channel_selector_halves_the_byte_count() {
        let stereo = interleave(&[1; 64], &[2; 64]);
        let mut sel = ChannelSelector::new(ChannelPick::Left);
        assert_eq!(sel.apply(&stereo).len(), stereo.len() / 2);
    }

    // ---- gain (pure) -------------------------------------------------------

    #[test]
    fn unity_gain_is_never_constructed() {
        // The reader skips the sample loop entirely below the epsilon, which is
        // what keeps the unity-gain path byte-exact.
        assert!(!super::super::process::gain_is_active(0.0));
        assert!(super::super::process::gain_is_active(6.0));
    }

    #[test]
    fn gain_scales_samples_by_the_decibel_factor() {
        let mut g = Gain::new(6.0206); // ×2
        let input: Vec<u8> = [100i16, -100, 0, 1000]
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        let out = g.apply(&input).to_vec();
        let got: Vec<i16> = out
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| i16::from_le_bytes([c[0], c[1]]))
            .collect();
        assert_eq!(got, vec![200, -200, 0, 2000]);
    }

    #[test]
    fn gain_clips_rather_than_wrapping() {
        // A wrapping cast would turn a loud boost into a burst of full-scale
        // noise of the *opposite* sign — audibly catastrophic.
        let mut g = Gain::new(40.0); // ×100
        let input: Vec<u8> = [20_000i16, -20_000]
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        let out = g.apply(&input).to_vec();
        let got: Vec<i16> = out
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| i16::from_le_bytes([c[0], c[1]]))
            .collect();
        assert_eq!(got, vec![i16::MAX, i16::MIN]);
    }

    #[test]
    fn gain_cuts_as_well_as_boosts() {
        let mut g = Gain::new(-6.0206); // ×0.5
        let input: Vec<u8> = 1000i16.to_le_bytes().to_vec();
        let out = g.apply(&input).to_vec();
        assert_eq!(i16::from_le_bytes([out[0], out[1]]), 500);
    }

    /// A pipe is a byte stream: a read can end mid-sample. The deferred byte
    /// must be scaled with its partner, not dropped and not scaled alone.
    #[test]
    fn gain_carries_a_sample_split_across_two_reads() {
        let mut g = Gain::new(6.0206); // ×2
        let whole = 1000i16.to_le_bytes();
        // First read delivers only the low byte.
        assert!(
            g.apply(&whole[..1]).is_empty(),
            "half a sample emits nothing"
        );
        // Second read completes it.
        let out = g.apply(&whole[1..]).to_vec();
        assert_eq!(out.len(), 2);
        assert_eq!(i16::from_le_bytes([out[0], out[1]]), 2000);
    }

    #[test]
    fn gain_handles_an_empty_read_while_carrying() {
        let mut g = Gain::new(6.0206);
        let whole = 1000i16.to_le_bytes();
        assert!(g.apply(&whole[..1]).is_empty());
        assert!(g.apply(&[]).is_empty(), "the carry survives an empty read");
        let out = g.apply(&whole[1..]).to_vec();
        assert_eq!(i16::from_le_bytes([out[0], out[1]]), 2000);
    }

    // ---- the tee itself ----------------------------------------------------

    #[test]
    fn tee_writes_everything_the_producer_emitted() {
        let fx = Fixture::new();
        let stream: Vec<u8> = (0..=200u8).collect();
        let mut tee = spawn(
            "src_seed_1".to_owned(),
            std::io::Cursor::new(stream.clone()),
            fx.writer(3),
            None,
            Shaping {
                gain_db: 0.0,
                pipeline: AudioPipeline::none(),
                eq: EqChain::default(),
                spec: TINY,
                pick: None,
            },
        )
        .expect("spawn tee");
        Fixture::wait_for("the tee to drain the producer", || !tee.is_alive());
        tee.stop();
        assert_eq!(
            fx.recorded_pcm(),
            stream,
            "every byte the producer emitted must reach a segment"
        );
    }

    #[test]
    fn tee_reports_dead_once_the_producer_ends() {
        let fx = Fixture::new();
        let mut tee = spawn(
            "src_seed_1".to_owned(),
            std::io::Cursor::new(vec![0u8; 8]),
            fx.writer(3),
            None,
            Shaping {
                gain_db: 0.0,
                pipeline: AudioPipeline::none(),
                eq: EqChain::default(),
                spec: TINY,
                pick: None,
            },
        )
        .expect("spawn tee");
        Fixture::wait_for("the tee thread to exit", || !tee.is_alive());
        tee.stop();
    }

    #[test]
    fn tee_feeds_the_live_tap_with_the_same_audio_it_records() {
        let fx = Fixture::new();
        let tap = Arc::new(LiveTap::new(TINY));
        let mut sub = tap.subscribe();
        let (reader, mut writer_end) = std::io::pipe().expect("pipe");
        let mut tee = spawn(
            "src_seed_1".to_owned(),
            reader,
            fx.writer(3),
            Some(Arc::clone(&tap)),
            Shaping {
                gain_db: 0.0,
                pipeline: AudioPipeline::none(),
                eq: EqChain::default(),
                spec: TINY,
                pick: None,
            },
        )
        .expect("spawn tee");

        writer_end.write_all(b"live-audio!").expect("produce");
        let mut heard = Vec::new();
        Fixture::wait_for("the tap to receive the audio", || {
            let mut buf = [0u8; 64];
            let n = sub.read(&mut buf, Duration::from_millis(50));
            heard.extend_from_slice(&buf[..n]);
            heard.len() >= 11
        });
        assert_eq!(heard, b"live-audio!");

        drop(writer_end);
        Fixture::wait_for("the tee to finish", || !tee.is_alive());
        tee.stop();
        assert_eq!(fx.recorded_pcm(), b"live-audio!");
    }

    /// The guarantee the whole design exists for: a listener that stops reading
    /// must not stop the recorder. Subscribe, never read, push far more than
    /// the ring can hold, and assert the segments still land intact.
    #[test]
    fn a_stalled_listener_cannot_stall_recording() {
        let fx = Fixture::new();
        let tap = Arc::new(LiveTap::new(TINY));
        // Subscribe and then abandon it — the subscriber never calls read().
        let stalled = tap.subscribe();
        let (reader, mut writer_end) = std::io::pipe().expect("pipe");
        let mut tee = spawn(
            "src_seed_1".to_owned(),
            reader,
            fx.writer(3),
            Some(Arc::clone(&tap)),
            Shaping {
                gain_db: 0.0,
                pipeline: AudioPipeline::none(),
                eq: EqChain::default(),
                spec: TINY,
                pick: None,
            },
        )
        .expect("spawn tee");

        // Far more than the ring's capacity (floored at 64 KiB).
        let stream: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        writer_end.write_all(&stream).expect("produce");
        drop(writer_end);
        Fixture::wait_for("the tee to finish", || !tee.is_alive());
        tee.stop();

        assert_eq!(
            fx.recorded_pcm(),
            stream,
            "a listener that never reads must not cost the recording a byte"
        );
        assert!(
            fx.segments().len() > 1,
            "the stream should have rotated several times"
        );
        // The stalled subscriber is the one that lost data, as designed.
        drop(stalled);
    }

    #[test]
    fn tee_applies_gain_to_both_consumers() {
        let fx = Fixture::new();
        let tap = Arc::new(LiveTap::new(TINY));
        let mut sub = tap.subscribe();
        let (reader, mut writer_end) = std::io::pipe().expect("pipe");
        let mut tee = spawn(
            "src_seed_1".to_owned(),
            reader,
            fx.writer(30),
            Some(Arc::clone(&tap)),
            Shaping {
                gain_db: 6.0206,
                pipeline: AudioPipeline::none(),
                eq: EqChain::default(),
                spec: TINY,
                pick: None,
            },
        )
        .expect("spawn tee");

        let input: Vec<u8> = [1000i16, -1000]
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        writer_end.write_all(&input).expect("produce");

        let mut heard = Vec::new();
        Fixture::wait_for("the tap to receive the boosted audio", || {
            let mut buf = [0u8; 16];
            let n = sub.read(&mut buf, Duration::from_millis(50));
            heard.extend_from_slice(&buf[..n]);
            heard.len() >= 4
        });
        drop(writer_end);
        Fixture::wait_for("the tee to finish", || !tee.is_alive());
        tee.stop();

        let expected: Vec<u8> = [2000i16, -2000]
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        assert_eq!(heard, expected, "live audio carries the configured gain");
        assert_eq!(
            fx.recorded_pcm(),
            expected,
            "…and so does what the detector will classify"
        );
    }

    /// The counter-test for the whole design, at the level this sandbox can
    /// reach: **three** consumers of one device open — the recorder and two
    /// independent listeners — all receive the full stream.
    ///
    /// Before the tee there was exactly one consumer possible, because the
    /// second opener of an ALSA `plughw:` device gets `EBUSY`; a `/stream`
    /// listener and the recorder could not coexist at all. (The `EBUSY` half of
    /// that statement is hardware behaviour and is not reproduced here — there
    /// is no ALSA device on this runner. What is proven here is that the
    /// replacement genuinely serves several consumers from one stream.)
    #[test]
    fn one_device_open_feeds_the_recorder_and_several_listeners() {
        let fx = Fixture::new();
        let tap = Arc::new(LiveTap::new(TINY));
        let mut listener_a = tap.subscribe();
        let mut listener_b = tap.subscribe();
        let (reader, mut writer_end) = std::io::pipe().expect("pipe");
        let mut tee = spawn(
            "src_seed_1".to_owned(),
            reader,
            fx.writer(3),
            Some(Arc::clone(&tap)),
            Shaping {
                gain_db: 0.0,
                pipeline: AudioPipeline::none(),
                eq: EqChain::default(),
                spec: TINY,
                pick: None,
            },
        )
        .expect("spawn tee");

        let stream: Vec<u8> = (0..=120u8).collect();
        writer_end.write_all(&stream).expect("produce");

        let mut heard_a = Vec::new();
        let mut heard_b = Vec::new();
        Fixture::wait_for("both listeners to hear the whole stream", || {
            let mut buf = [0u8; 256];
            let n = listener_a.read(&mut buf, Duration::from_millis(50));
            heard_a.extend_from_slice(&buf[..n]);
            let n = listener_b.read(&mut buf, Duration::from_millis(50));
            heard_b.extend_from_slice(&buf[..n]);
            heard_a.len() >= stream.len() && heard_b.len() >= stream.len()
        });

        drop(writer_end);
        Fixture::wait_for("the tee to finish", || !tee.is_alive());
        tee.stop();

        assert_eq!(heard_a, stream, "listener A heard the whole stream");
        assert_eq!(heard_b, stream, "listener B heard the whole stream");
        assert_eq!(
            fx.recorded_pcm(),
            stream,
            "…and the recorder still got every byte"
        );
    }

    /// Rotation must be driven by the sample count, so the recorded stream is
    /// contiguous across a boundary rather than losing whatever arrived while a
    /// file was being swapped.
    #[test]
    fn rotation_loses_nothing_at_the_boundary() {
        let fx = Fixture::new();
        let (reader, mut writer_end) = std::io::pipe().expect("pipe");
        let mut tee = spawn(
            "src_seed_1".to_owned(),
            reader,
            fx.writer(3), // 30 bytes per segment
            None,
            Shaping {
                gain_db: 0.0,
                pipeline: AudioPipeline::none(),
                eq: EqChain::default(),
                spec: TINY,
                pick: None,
            },
        )
        .expect("spawn tee");

        // Write in 7-byte chunks so no write aligns with the 30-byte boundary,
        // advancing the clock so each segment gets its own filename.
        let stream: Vec<u8> = (0..=209u8).collect();
        for chunk in stream.chunks(7) {
            writer_end.write_all(chunk).expect("produce");
        }
        drop(writer_end);
        Fixture::wait_for("the tee to finish", || !tee.is_alive());
        tee.stop();

        assert_eq!(fx.recorded_pcm(), stream);
        assert_eq!(
            fx.recorded_pcm().len(),
            210,
            "byte count across rotations must be exact"
        );
    }
}

// ── the tee applies the same conditioning ffmpeg sources get ────────────
//
// The ffmpeg-backed sources get an `-af` chain; the teed microphone gets these
// stages. Both are driven by the same `AudioPipeline`, so if these did nothing
// the toggles would work on a network camera and silently not on the
// microphone plugged into the Pi.
#[cfg(test)]
mod conditioning_tests {
    use super::*;

    const SR: u32 = 48_000;

    fn pcm(samples: &[i16]) -> Vec<u8> {
        samples.iter().flat_map(|s| s.to_le_bytes()).collect()
    }

    fn unpcm(bytes: &[u8]) -> Vec<i16> {
        bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|p| i16::from_le_bytes(*p))
            .collect()
    }

    fn only(high_pass: bool, dc_removal: bool, agc: bool) -> AudioPipeline {
        AudioPipeline {
            high_pass,
            dc_removal,
            agc,
            rtsp_stall_timeout: false,
        }
    }

    /// A sine at `hz`, `n` samples long, at amplitude `amp` of full scale.
    fn tone(hz: f32, n: usize, amp: f32) -> Vec<i16> {
        (0..n)
            .map(|i| {
                #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
                let t = i as f32 / SR as f32;
                #[allow(clippy::cast_possible_truncation)]
                let v = (amp * f32::from(i16::MAX) * (2.0 * std::f32::consts::PI * hz * t).sin())
                    as i16;
                v
            })
            .collect()
    }

    /// Peak absolute value, as a fraction of full scale.
    fn peak(samples: &[i16]) -> f32 {
        samples
            .iter()
            .map(|s| f32::from(s.abs()) / f32::from(i16::MAX))
            .fold(0.0_f32, f32::max)
    }

    /// Nothing enabled constructs nothing, so the untouched path stays
    /// byte-exact — the same contract `Gain` keeps at unity.
    #[test]
    fn all_stages_off_constructs_no_stage() {
        assert!(PreConditioner::new(AudioPipeline::none(), &EqChain::default(), SR, 1).is_none());
        assert!(AgcStage::new(AudioPipeline::none(), SR, 1).is_none());
    }

    /// DC removal removes a constant offset.
    #[test]
    fn dc_removal_converges_a_constant_offset_to_zero() {
        let mut pre =
            PreConditioner::new(only(false, true, false), &EqChain::default(), SR, 1).unwrap();
        let input = pcm(&vec![8_000_i16; SR as usize]); // 1 s of pure DC
        let out = unpcm(pre.apply(&input));

        let first = out[0];
        let last = *out.last().unwrap();
        assert!(
            first.abs() > 4_000,
            "the step itself must survive the first sample: {first}"
        );
        assert!(
            last.abs() < 100,
            "a constant offset must decay to ~0, got {last}"
        );
    }

    /// The high-pass attenuates rumble far more than birdsong. The counterpart
    /// half is what makes it a discriminator rather than an "attenuates
    /// everything" alarm: a filter that killed both bands would pass the first
    /// assertion and fail the second.
    #[test]
    fn high_pass_cuts_rumble_and_keeps_the_bird_band() {
        let n = SR as usize / 2;
        let attenuation = |hz: f32| {
            let mut pre =
                PreConditioner::new(only(true, false, false), &EqChain::default(), SR, 1).unwrap();
            let input = tone(hz, n, 0.5);
            let out = unpcm(pre.apply(&pcm(&input)));
            // Skip the settling transient.
            peak(&out[n / 4..]) / peak(&input[n / 4..])
        };

        let rumble = attenuation(30.0);
        let song = attenuation(2_000.0);
        assert!(
            rumble < 0.35,
            "30 Hz rumble must be well attenuated, kept {rumble:.3}"
        );
        assert!(
            song > 0.95,
            "2 kHz birdsong must pass essentially untouched, kept {song:.3}"
        );
    }

    /// Filter state carries across reads. Without this every buffer boundary
    /// would restart the filter and put a step into the signal — the reason
    /// the stages hold state at all rather than reusing the block-local
    /// helpers in `quality::rain_detector`.
    #[test]
    fn output_is_identical_whether_split_across_reads() {
        let input = pcm(&tone(200.0, 4_000, 0.4));

        let mut whole =
            PreConditioner::new(only(true, true, false), &EqChain::default(), SR, 1).unwrap();
        let a = whole.apply(&input).to_vec();

        let mut split =
            PreConditioner::new(only(true, true, false), &EqChain::default(), SR, 1).unwrap();
        // 777 is deliberately odd, so a sample straddles the boundary.
        let mut b = split.apply(&input[..777]).to_vec();
        b.extend_from_slice(split.apply(&input[777..]));

        assert_eq!(a, b, "a chunk boundary must not change the output");
    }

    /// Channels are filtered independently. Interleaved PCM filtered with one
    /// shared state would let a DC offset on the left channel bleed into the
    /// right, which is exactly what a per-source stereo microphone would show.
    #[test]
    fn stereo_channels_do_not_bleed_into_each_other() {
        let mut pre =
            PreConditioner::new(only(false, true, false), &EqChain::default(), SR, 2).unwrap();
        // Left carries a large DC offset; right is silent.
        let frames: Vec<i16> = (0..SR as usize).flat_map(|_| [12_000_i16, 0]).collect();
        let out = unpcm(pre.apply(&pcm(&frames)));

        let right_max = out
            .iter()
            .skip(1)
            .step_by(2)
            .map(|s| s.abs())
            .max()
            .unwrap();
        assert!(
            right_max < 50,
            "a silent right channel must stay silent, peaked at {right_max}"
        );
    }

    /// AGC lifts a quiet signal towards the target without exceeding it.
    #[test]
    fn agc_brings_a_quiet_signal_up_and_does_not_clip() {
        let mut agc = AgcStage::new(only(false, false, true), SR, 1).unwrap();
        let quiet = tone(1_000.0, SR as usize * 4, 0.02); // −34 dBFS
        let out = unpcm(agc.apply(&pcm(&quiet)));

        let tail = &out[out.len() * 3 / 4..];
        let lifted = peak(tail);
        assert!(
            lifted > 0.10,
            "a −34 dBFS signal must be lifted well above its input level, reached {lifted:.3}"
        );
        assert!(lifted <= 1.0, "output must never exceed full scale");
    }

    /// The counterpart: a signal already at a healthy level is not shoved into
    /// the clipper. Without this, "AGC works" would be satisfied by a stage
    /// that simply multiplied everything by ten.
    #[test]
    fn agc_does_not_amplify_an_already_loud_signal_into_clipping() {
        let mut agc = AgcStage::new(only(false, false, true), SR, 1).unwrap();
        let loud = tone(1_000.0, SR as usize * 2, 0.8);
        let out = unpcm(agc.apply(&pcm(&loud)));

        let tail = &out[out.len() / 2..];
        let clipped = tail.iter().filter(|s| s.abs() >= i16::MAX - 1).count();
        assert!(
            clipped < tail.len() / 100,
            "a loud input must not be driven into the rail: {clipped} clipped of {}",
            tail.len()
        );
    }
}

#[cfg(test)]
mod eq_chain_tests {
    use super::*;
    use crate::audio::biquad::Biquad;
    use std::f32::consts::{FRAC_1_SQRT_2, TAU};

    const SR: u32 = 48_000;

    fn pipeline(high_pass: bool, dc_removal: bool) -> AudioPipeline {
        AudioPipeline {
            high_pass,
            dc_removal,
            agc: false,
            rtsp_stall_timeout: false,
        }
    }

    /// Response of a filter *as it actually runs*, by measuring the audio,
    /// rather than from its coefficients. Four seconds, second half only, so
    /// the start-up transient is gone.
    fn measured_db(mut step: impl FnMut(f32) -> f32, hz: f32) -> f64 {
        let n = SR as usize * 4;
        let out: Vec<f32> = (0..n)
            .map(|i| {
                #[allow(clippy::cast_precision_loss)]
                let x = (TAU * hz * i as f32 / SR as f32).sin();
                step(x)
            })
            .collect();
        let tail = &out[n / 2..];
        let rms = (tail
            .iter()
            .map(|s| f64::from(*s) * f64::from(*s))
            .sum::<f64>()
            / f64::from(u32::try_from(tail.len()).expect("tail fits")))
        .sqrt();
        20.0 * (rms * std::f64::consts::SQRT_2).log10()
    }

    /// The divergence table in [`AudioPipeline`]'s documentation, pinned.
    ///
    /// Two backends implement `high_pass` from the same flag and they are not
    /// the same filter: the tee has one pole, ffmpeg's `highpass` defaults to
    /// two. Writing that down is only worth anything if the numbers are real,
    /// and a doc comment cannot be run — so the table is asserted here, to
    /// 0.05 dB, and a reader who changes either filter finds out immediately.
    ///
    /// The ffmpeg column is computed from the RBJ two-pole high-pass at
    /// Q = 1/√2, which is what `af_biquads.c` implements for `highpass` at its
    /// default `poles=2, width_type=q, width=0.707`. It is not a measurement
    /// of the installed ffmpeg, which is not present on every machine that
    /// runs this suite.
    #[test]
    fn the_two_backends_high_pass_differently_and_by_how_much() {
        let table = [
            (20.0_f32, -15.68_f64, -31.13_f64),
            (30.0, -12.31, -24.10),
            (50.0, -8.31, -15.34),
            (60.0, -7.00, -12.30),
            (80.0, -5.14, -7.83),
            (120.0, -3.04, -3.01),
        ];
        for (hz, want_one, want_two) in table {
            let mut one = OnePoleHighPass::new(SR, HIGH_PASS_CUTOFF_HZ);
            let got_one = measured_db(|x| one.step(x), hz);
            let got_two = 20.0
                * Biquad::high_pass(HIGH_PASS_CUTOFF_HZ, SR, FRAC_1_SQRT_2)
                    .expect("designs")
                    .magnitude_at(hz, SR)
                    .log10();
            assert!(
                (got_one - want_one).abs() < 0.05,
                "tee one-pole at {hz} Hz: documented {want_one:.2} dB, measured {got_one:.2} dB"
            );
            assert!(
                (got_two - want_two).abs() < 0.05,
                "ffmpeg two-pole at {hz} Hz: documented {want_two:.2} dB, computed {got_two:.2} dB"
            );
        }
    }

    /// An empty chain is not "a chain that does nothing" — it selects the
    /// flags. The distinction is the whole no-op-upgrade guarantee: every
    /// station that has never opened the equaliser must keep the audio path it
    /// had, one pole and all.
    #[test]
    fn an_empty_chain_leaves_the_pipeline_flags_in_charge() {
        let mut with = PreConditioner::new(pipeline(true, false), &EqChain::default(), SR, 1)
            .expect("the high-pass flag alone builds a conditioner");
        assert!(matches!(with.stages, PreStages::Flags { .. }));

        // And it is the one-pole, measured through the real byte path.
        let hz = 30.0_f32;
        let n = SR as usize * 4;
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            #[allow(clippy::cast_precision_loss)]
            let x = (TAU * hz * i as f32 / SR as f32).sin();
            #[allow(clippy::cast_possible_truncation)]
            let s = (x * f32::from(i16::MAX) * 0.5) as i16;
            out.extend_from_slice(with.apply(&s.to_le_bytes()));
        }
        let samples: Vec<f32> = out
            .as_chunks::<2>()
            .0
            .iter()
            .map(|p| f32::from(i16::from_le_bytes(*p)) / (f32::from(i16::MAX) * 0.5))
            .collect();
        let tail = &samples[n / 2..];
        let rms = (tail
            .iter()
            .map(|s| f64::from(*s) * f64::from(*s))
            .sum::<f64>()
            / f64::from(u32::try_from(tail.len()).expect("tail fits")))
        .sqrt();
        let db = 20.0 * (rms * std::f64::consts::SQRT_2).log10();
        assert!(
            (db + 12.31).abs() < 0.2,
            "an empty chain must still give the one-pole's -12.31 dB at 30 Hz, got {db:.2}"
        );
    }

    /// A chain *replaces* the fixed high-passes rather than stacking on them.
    /// Stacking would be the easy mistake and the wrong one: an operator who
    /// writes their own 120 Hz corner would silently get two.
    #[test]
    fn a_chain_replaces_the_flags_it_supersedes() {
        let chain = EqChain::parse("highpass:120").expect("parses");
        let pre = PreConditioner::new(pipeline(true, true), &chain, SR, 1)
            .expect("a chain always builds a conditioner");
        match &pre.stages {
            PreStages::Chain(lanes) => {
                assert_eq!(lanes.len(), 1);
                assert_eq!(lanes[0].section_count(), 1, "one stage, not three");
            }
            PreStages::Flags { .. } => panic!("the chain must win over the flags"),
        }
    }

    /// Each channel needs its own filter state. Sharing one would cross-couple
    /// a stereo pair — the left channel's history colouring the right — which
    /// is inaudible on noise and obvious on a hard-panned call.
    #[test]
    fn every_channel_gets_its_own_state() {
        let chain = EqChain::parse("highpass:500:2").expect("parses");
        let mut pre = PreConditioner::new(pipeline(false, false), &chain, SR, 2)
            .expect("a chain always builds a conditioner");

        // Left silent, right a step. With shared state the left channel would
        // pick up the right channel's ringing.
        let mut left_energy = 0.0_f64;
        for _ in 0..2000 {
            let frame = [0_i16, 20_000];
            let bytes: Vec<u8> = frame.iter().flat_map(|s| s.to_le_bytes()).collect();
            let out = pre.apply(&bytes);
            let got: Vec<i16> = out
                .as_chunks::<2>()
                .0
                .iter()
                .map(|p| i16::from_le_bytes(*p))
                .collect();
            left_energy = f64::from(got[0]).mul_add(f64::from(got[0]), left_energy);
        }
        assert!(
            left_energy < 1.0,
            "a silent channel must stay silent; leaked energy {left_energy}"
        );
    }

    /// A chain that cannot be realised at this rate must not silence the
    /// station. An 8 kHz source has a 4 kHz Nyquist, so a 6 kHz stage is
    /// undesignable — capture keeps running on the flags.
    #[test]
    fn an_unbuildable_chain_falls_back_to_the_flags() {
        let chain = EqChain::parse("peaking:6000:1:3").expect("parses");
        let pre = PreConditioner::new(pipeline(true, false), &chain, 8_000, 1)
            .expect("the flags still build a conditioner");
        assert!(
            matches!(pre.stages, PreStages::Flags { .. }),
            "an undesignable chain must fall back, not take over"
        );
    }

    /// ...and when the flags are off too, the fallback is "no conditioning at
    /// all" rather than a half-built chain. The counterpart to the test above:
    /// without it, `Flags` would look like the answer to everything.
    #[test]
    fn an_unbuildable_chain_with_no_flags_conditions_nothing() {
        let chain = EqChain::parse("peaking:6000:1:3").expect("parses");
        assert!(
            PreConditioner::new(pipeline(false, false), &chain, 8_000, 1).is_none(),
            "nothing to do means no conditioner, and no allocation"
        );
    }
}
