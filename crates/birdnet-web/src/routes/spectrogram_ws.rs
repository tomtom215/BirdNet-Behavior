//! WebSocket endpoint for live spectrogram streaming.
//!
//! Clients connect to `/api/v2/ws/spectrogram` to receive real-time
//! spectrogram frames as new audio files are processed. Each frame is
//! a JSON message containing the mel spectrogram data for visualization.
//!
//! | Method | Path | Action |
//! |--------|------|--------|
//! | GET    | /ws/spectrogram | WebSocket upgrade for live spectrogram |

use axum::Router;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use axum::routing::get;
use serde::Serialize;
use std::sync::Arc;
use tokio::sync::broadcast;

use crate::state::AppState;

/// A spectrogram frame event broadcast to WebSocket clients.
#[derive(Debug, Clone, Serialize)]
pub struct WsSpectrogramEvent {
    /// Event type (always "spectrogram").
    pub event: &'static str,
    /// Source filename.
    pub filename: String,
    /// Number of mel bands (rows).
    pub n_mels: usize,
    /// Number of time frames (columns).
    pub n_frames: usize,
    /// Flattened spectrogram data [0.0–1.0] in row-major order.
    pub data: Vec<f32>,
    /// Sample rate of the source audio.
    pub sample_rate: u32,
}

/// Shared broadcast channel for spectrogram events.
#[derive(Debug, Clone)]
pub struct SpectrogramBroadcast {
    tx: broadcast::Sender<Arc<String>>,
}

impl SpectrogramBroadcast {
    /// Create a new broadcast channel with the specified capacity.
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    /// Broadcast a spectrogram frame to all connected WebSocket clients.
    pub fn send(&self, event: &WsSpectrogramEvent) {
        if self.tx.receiver_count() == 0 {
            return;
        }

        match serde_json::to_string(event) {
            Ok(json) => {
                let _ = self.tx.send(Arc::new(json));
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to serialize spectrogram event");
            }
        }
    }

    /// Get a new receiver for spectrogram events.
    pub fn subscribe(&self) -> broadcast::Receiver<Arc<String>> {
        self.tx.subscribe()
    }

    /// Number of currently connected clients.
    pub fn client_count(&self) -> usize {
        self.tx.receiver_count()
    }
}

/// Spectrogram WebSocket routes.
pub fn router() -> Router<AppState> {
    Router::new().route("/ws/spectrogram", get(ws_handler))
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    let broadcast = state.spectrogram_broadcast();
    ws.on_upgrade(move |socket| handle_ws_connection(socket, broadcast))
}

async fn handle_ws_connection(mut socket: WebSocket, broadcast: SpectrogramBroadcast) {
    tracing::info!(
        clients = broadcast.client_count() + 1,
        "spectrogram WebSocket client connected"
    );

    let mut rx = broadcast.subscribe();

    let welcome = serde_json::json!({
        "event": "connected",
        "message": "BirdNet-Behavior live spectrogram stream",
        "version": env!("CARGO_PKG_VERSION"),
    });

    if let Ok(welcome_json) = serde_json::to_string(&welcome)
        && socket
            .send(Message::Text(welcome_json.into()))
            .await
            .is_err()
    {
        return;
    }

    loop {
        tokio::select! {
            result = rx.recv() => {
                match result {
                    Ok(json) => {
                        if socket.send(Message::Text((*json).clone().into())).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(count)) => {
                        tracing::debug!(missed = count, "spectrogram WS client lagged");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Ping(data))) => {
                        if socket.send(Message::Pong(data)).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Close(_)) | Err(_)) | None => break,
                    _ => {}
                }
            }
        }
    }

    tracing::info!(
        clients = broadcast.client_count().saturating_sub(1),
        "spectrogram WebSocket client disconnected"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn broadcast_with_no_receivers() {
        let broadcast = SpectrogramBroadcast::new(16);
        let event = WsSpectrogramEvent {
            event: "spectrogram",
            filename: "test.wav".into(),
            n_mels: 128,
            n_frames: 64,
            data: vec![0.0; 128 * 64],
            sample_rate: 48000,
        };
        broadcast.send(&event);
        assert_eq!(broadcast.client_count(), 0);
    }

    #[test]
    fn broadcast_delivers_frame() {
        let broadcast = SpectrogramBroadcast::new(16);
        let mut rx = broadcast.subscribe();

        let event = WsSpectrogramEvent {
            event: "spectrogram",
            filename: "bird_clip.wav".into(),
            n_mels: 4,
            n_frames: 2,
            data: vec![0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8],
            sample_rate: 48000,
        };

        broadcast.send(&event);
        let received = rx.try_recv().unwrap();
        assert!(received.contains("bird_clip.wav"));
        assert!(received.contains("\"n_mels\":4"));
    }

    #[test]
    fn ws_spectrogram_event_serializes() {
        let event = WsSpectrogramEvent {
            event: "spectrogram",
            filename: "recording.wav".into(),
            n_mels: 128,
            n_frames: 32,
            data: vec![0.5; 128 * 32],
            sample_rate: 48000,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"event\":\"spectrogram\""));
    }
}
