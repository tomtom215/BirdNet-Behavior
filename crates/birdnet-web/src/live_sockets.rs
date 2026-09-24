//! Who may open the live `WebSockets`, and how many at once.
//!
//! A `WebSocket` is not subject to the same-origin policy or to CORS, and the
//! CSRF guard only looks at state-changing methods — a handshake is a `GET`.
//! Without a check here any page anyone on the network visited could open
//! `/api/v2/ws/detections` and `/api/v2/ws/spectrogram` on an open station and
//! read them. A browser always sends `Origin` on a handshake, so the same rule
//! as the CSRF guard applies: an `Origin` must name this station; a client
//! that sends none (a script, `websocat`) is not a browser a website can
//! drive.
//!
//! The cap bounds the copies: every broadcast frame is cloned once per client.
//! A permit is held for the life of the connection and released when it ends.

use std::sync::Arc;

use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// Default cap on detection sockets. Every page opens one, so this is a
/// household's tabs across its devices with room to spare.
pub const DETECTION_SOCKETS: usize = 64;

/// Default cap on spectrogram sockets. Each carries every mel frame of the
/// live microphone, and only the Today card and Recordings → Live open one.
pub const SPECTROGRAM_SOCKETS: usize = 16;

/// Which live socket a handshake is for.
#[derive(Debug, Clone, Copy)]
pub enum Kind {
    /// `/api/v2/ws/detections`.
    Detections,
    /// `/api/v2/ws/spectrogram`.
    Spectrogram,
}

/// The per-station admission state for the live sockets.
#[derive(Debug, Clone)]
pub struct LiveSockets {
    detections: Arc<Semaphore>,
    spectrogram: Arc<Semaphore>,
}

impl Default for LiveSockets {
    fn default() -> Self {
        Self {
            detections: Arc::new(Semaphore::new(DETECTION_SOCKETS)),
            spectrogram: Arc::new(Semaphore::new(SPECTROGRAM_SOCKETS)),
        }
    }
}

impl LiveSockets {
    /// Both kinds capped at `limit` — for tests, which cannot open 64 sockets
    /// from one address without meeting the rate limiter first.
    #[must_use]
    pub fn with_limit(limit: usize) -> Self {
        Self {
            detections: Arc::new(Semaphore::new(limit)),
            spectrogram: Arc::new(Semaphore::new(limit)),
        }
    }

    /// Admit a handshake, or say why not: `403` for another site's page,
    /// `503` when every slot of this kind is taken. The permit must be held
    /// for as long as the connection is open.
    ///
    /// # Errors
    ///
    /// The refusal response.
    #[allow(clippy::result_large_err)]
    pub fn admit(&self, kind: Kind, headers: &HeaderMap) -> Result<OwnedSemaphorePermit, Response> {
        if !crate::security::is_same_origin(headers) {
            tracing::info!(
                origin = headers
                    .get(axum::http::header::ORIGIN)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or(""),
                ?kind,
                "live socket refused: another site's page"
            );
            return Err((
                StatusCode::FORBIDDEN,
                "The live stream is only available to this station's own pages.",
            )
                .into_response());
        }
        let slots = match kind {
            Kind::Detections => &self.detections,
            Kind::Spectrogram => &self.spectrogram,
        };
        Arc::clone(slots).try_acquire_owned().map_err(|_| {
            tracing::warn!(?kind, "live socket refused: every slot is taken");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "Too many live connections are open; close a tab and try again.",
            )
                .into_response()
        })
    }
}
