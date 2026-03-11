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
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::DecoderOptions;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    let file = std::fs::File::open(path)?;
    let mss = MediaSourceStream::new(Box::new(file), MediaSourceStreamOptions::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|e| DecodeError::Format(e.to_string()))?;

    let mut format = probed.format;
    let track = format.default_track().ok_or(DecodeError::NoTracks)?;
    let sample_rate = track
        .codec_params
        .sample_rate
        .ok_or_else(|| DecodeError::Format("unknown sample rate".into()))?;

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| DecodeError::Format(e.to_string()))?;

    let track_id = track.id;
    let mut samples = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(symphonia::core::errors::Error::IoError(ref e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(e) => return Err(DecodeError::Format(e.to_string())),
        };

        if packet.track_id() != track_id {
            continue;
        }

        let audio_buf = decoder
            .decode(&packet)
            .map_err(|e| DecodeError::Format(e.to_string()))?;

        let spec = *audio_buf.spec();
        let num_channels = spec.channels.count();
        let num_frames = audio_buf.frames();

        let mut sample_buf = SampleBuffer::<f32>::new(num_frames as u64, spec);
        sample_buf.copy_interleaved_ref(audio_buf);

        let interleaved = sample_buf.samples();

        // Mix to mono by averaging channels
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
