//! Audio decoding via symphonia.
//!
//! Decodes WAV, FLAC, and MP3 files into f32 sample buffers.
//! Replaces `librosa.load()` and `soundfile.read()`.

use std::fmt;
use std::path::Path;

/// Decoded audio data as mono f32 samples at a known sample rate.
#[derive(Debug, Clone)]
pub struct AudioData {
    /// Mono audio samples normalized to [-1.0, 1.0].
    pub samples: Vec<f32>,
    /// Sample rate in Hz.
    pub sample_rate: u32,
}

/// Errors that can occur during audio decoding.
#[derive(Debug)]
pub enum DecodeError {
    /// File not found or inaccessible.
    Io(std::io::Error),
    /// Unsupported or corrupt audio format.
    Format(String),
    /// No audio tracks in the file.
    NoTracks,
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/O error: {e}"),
            Self::Format(msg) => write!(f, "format error: {msg}"),
            Self::NoTracks => write!(f, "no audio tracks found"),
        }
    }
}

impl std::error::Error for DecodeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Format(_) | Self::NoTracks => None,
        }
    }
}

impl From<std::io::Error> for DecodeError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// Decode an audio file to mono f32 samples.
///
/// # Errors
///
/// Returns `DecodeError` if the file cannot be read, decoded, or contains no audio.
#[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
pub fn decode_file(path: &Path) -> Result<AudioData, DecodeError> {
    use symphonia::core::audio::GenericAudioBufferRef;
    use symphonia::core::codecs::audio::AudioDecoderOptions;
    use symphonia::core::errors::Error as SymphoniaError;
    use symphonia::core::formats::probe::Hint;
    use symphonia::core::formats::{FormatOptions, TrackType};
    use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
    use symphonia::core::meta::MetadataOptions;

    let file = std::fs::File::open(path)?;
    let mss = MediaSourceStream::new(Box::new(file), MediaSourceStreamOptions::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    // Symphonia 0.6: `probe` takes options by value and returns the
    // `FormatReader` directly (no more `ProbeResult` wrapper).
    let mut format = symphonia::default::get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|e| DecodeError::Format(e.to_string()))?;

    // 0.6 splits tracks by `TrackType`; we only want audio.
    let track = format
        .default_track(TrackType::Audio)
        .ok_or(DecodeError::NoTracks)?;
    let track_id = track.id;
    let audio_params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or_else(|| DecodeError::Format("track has no audio codec params".into()))?;
    let sample_rate = audio_params
        .sample_rate
        .ok_or_else(|| DecodeError::Format("unknown sample rate".into()))?;

    // `make_audio_decoder` now takes the audio-specific codec params
    // (previously `make` accepted the whole CodecParameters struct).
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(audio_params, &AudioDecoderOptions::default())
        .map_err(|e| DecodeError::Format(e.to_string()))?;

    let mut samples = Vec::new();
    let mut interleaved: Vec<f32> = Vec::new();

    // 0.6: `next_packet` returns `Result<Option<Packet>>` (EOF is now `None`
    // rather than a sentinel `UnexpectedEof` error).
    while let Some(packet) = format
        .next_packet()
        .map_err(|e| DecodeError::Format(e.to_string()))?
    {
        // `track_id` is a struct field now, not a method call.
        if packet.track_id != track_id {
            continue;
        }

        let audio_buf: GenericAudioBufferRef<'_> = match decoder.decode(&packet) {
            Ok(b) => b,
            // Recoverable decode errors (typical at format edges) — skip
            // this packet and continue.
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(e) => return Err(DecodeError::Format(e.to_string())),
        };

        let num_channels = audio_buf.num_planes();
        let num_frames = audio_buf.frames();
        if num_channels == 0 || num_frames == 0 {
            continue;
        }

        interleaved.resize(audio_buf.samples_interleaved(), 0.0_f32);
        audio_buf.copy_to_slice_interleaved(&mut interleaved[..]);

        // Mix to mono by averaging channels.
        for frame in 0..num_frames {
            let mut sum = 0.0_f32;
            for ch in 0..num_channels {
                sum += interleaved[frame * num_channels + ch];
            }
            samples.push(sum / num_channels as f32);
        }
    }

    Ok(AudioData {
        samples,
        sample_rate,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn decode_nonexistent_file_returns_error() {
        let result = decode_file(&PathBuf::from("/nonexistent/file.wav"));
        assert!(result.is_err());
    }
}
