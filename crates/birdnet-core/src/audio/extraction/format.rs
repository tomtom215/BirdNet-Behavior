//! Supported audio output formats for extracted clips.

use std::fmt;

/// Supported audio output formats for extracted clips.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioFormat {
    /// WAV (PCM 16-bit) — no external tools required.
    Wav,
    /// MP3 — requires ffmpeg or sox.
    Mp3,
    /// FLAC — requires ffmpeg or sox.
    Flac,
    /// OGG Vorbis — requires ffmpeg or sox.
    Ogg,
}

impl AudioFormat {
    /// File extension for this format.
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Wav => "wav",
            Self::Mp3 => "mp3",
            Self::Flac => "flac",
            Self::Ogg => "ogg",
        }
    }

    /// Parse a format string (case-insensitive).
    ///
    /// Returns `Wav` for unrecognized formats.
    pub fn parse(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "mp3" => Self::Mp3,
            "flac" => Self::Flac,
            "ogg" | "vorbis" => Self::Ogg,
            _ => Self::Wav,
        }
    }

    /// Whether this format requires external conversion from WAV.
    pub const fn needs_conversion(self) -> bool {
        !matches!(self, Self::Wav)
    }
}

impl fmt::Display for AudioFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.extension())
    }
}
