//! WebSocket endpoint for live detection streaming.
//!
//! Clients connect to `/api/v2/ws/detections` to receive real-time
//! detection events as they occur. Uses axum's built-in WebSocket support.

use axum::Router;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use axum::routing::get;
use serde::Serialize;
use std::sync::Arc;
use tokio::sync::broadcast;

use crate::state::AppState;

/// A detection event broadcast to WebSocket clients.
#[derive(Debug, Clone, Serialize)]
pub struct WsDetectionEvent {
    /// Event type (always "detection").
    pub event: &'static str,
    /// Species common name.
    pub common_name: String,
    /// Species scientific name.
    pub scientific_name: String,
    /// Confidence score (0.0 - 1.0).
    pub confidence: f32,
    /// Detection date (YYYY-MM-DD).
    pub date: String,
    /// Detection time (HH:MM:SS).
    pub time: String,
    /// Start time in seconds within the recording.
    pub start: f32,
    /// End time in seconds within the recording.
    pub stop: f32,
}

/// Shared broadcast channel for detection events.
#[derive(Debug, Clone)]
pub struct DetectionBroadcast {
    tx: broadcast::Sender<Arc<String>>,
}

impl DetectionBroadcast {
    /// Create a new broadcast channel with the specified capacity.
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    /// Broadcast a detection event to all connected WebSocket clients.
    ///
    /// Serializes the event to JSON once, then sends to all receivers.
    pub fn send(&self, event: &WsDetectionEvent) {
        if self.tx.receiver_count() == 0 {
            return;
        }

        match serde_json::to_string(event) {
            Ok(json) => {
                let _ = self.tx.send(Arc::new(json));
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to serialize WebSocket event");
            }
        }
    }

    /// Get a new receiver for detection events.
    pub fn subscribe(&self) -> broadcast::Receiver<Arc<String>> {
        self.tx.subscribe()
    }

    /// Number of currently connected clients.
    pub fn client_count(&self) -> usize {
        self.tx.receiver_count()
    }
}

/// WebSocket routes.
pub fn router() -> Router<AppState> {
    Router::new().route("/ws/detections", get(ws_handler))
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    let broadcast = state.detection_broadcast();
    ws.on_upgrade(move |socket| handle_ws_connection(socket, broadcast))
}

async fn handle_ws_connection(mut socket: WebSocket, broadcast: DetectionBroadcast) {
    tracing::info!(
        clients = broadcast.client_count() + 1,
        "WebSocket client connected"
    );

    let mut rx = broadcast.subscribe();

    // Send welcome message
    let welcome = serde_json::json!({
        "event": "connected",
        "message": "BirdNet-Behavior live detection stream",
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
            // Forward broadcast events to this client
            result = rx.recv() => {
                match result {
                    Ok(json) => {
                        if socket.send(Message::Text((*json).clone().into())).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(count)) => {
                        tracing::debug!(missed = count, "WebSocket client lagged");
                        let lag_msg = serde_json::json!({
                            "event": "lagged",
                            "missed": count,
                        });
                        if let Ok(json) = serde_json::to_string(&lag_msg) {
                            let _ = socket.send(Message::Text(json.into())).await;
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
            // Handle incoming messages from client (ping/pong, close)
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Ping(data))) => {
                        if socket.send(Message::Pong(data)).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Close(_)) | Err(_)) | None => break,
                    _ => {} // Ignore text/binary from client
                }
            }
        }
    }

    tracing::info!(
        clients = broadcast.client_count().saturating_sub(1),
        "WebSocket client disconnected"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn broadcast_with_no_receivers_does_not_panic() {
        let broadcast = DetectionBroadcast::new(16);
        let event = WsDetectionEvent {
            event: "detection",
            common_name: "Eurasian Blackbird".into(),
            scientific_name: "Turdus merula".into(),
            confidence: 0.87,
            date: "2026-03-12".into(),
            time: "06:30:00".into(),
            start: 3.0,
            stop: 6.0,
        };
        broadcast.send(&event); // Should not panic
        assert_eq!(broadcast.client_count(), 0);
    }

    #[test]
    fn broadcast_delivers_to_subscriber() {
        let broadcast = DetectionBroadcast::new(16);
        let mut rx = broadcast.subscribe();

        let event = WsDetectionEvent {
            event: "detection",
            common_name: "European Robin".into(),
            scientific_name: "Erithacus rubecula".into(),
            confidence: 0.92,
            date: "2026-03-12".into(),
            time: "06:45:00".into(),
            start: 0.0,
            stop: 3.0,
        };

        broadcast.send(&event);

        let received = rx.try_recv().unwrap();
        assert!(received.contains("European Robin"));
        assert!(received.contains("0.92"));
    }

    #[test]
    fn broadcast_subscriber_count() {
        let broadcast = DetectionBroadcast::new(16);
        assert_eq!(broadcast.client_count(), 0);

        let rx1 = broadcast.subscribe();
        assert_eq!(broadcast.client_count(), 1);

        let rx2 = broadcast.subscribe();
        assert_eq!(broadcast.client_count(), 2);

        drop(rx1);
        // Keep rx2 alive for count assertion above
        drop(rx2);
    }

    #[test]
    fn ws_detection_event_serializes() {
        let event = WsDetectionEvent {
            event: "detection",
            common_name: "Great Tit".into(),
            scientific_name: "Parus major".into(),
            confidence: 0.80,
            date: "2026-03-12".into(),
            time: "07:00:00".into(),
            start: 6.0,
            stop: 9.0,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"event\":\"detection\""));
        assert!(json.contains("Great Tit"));
    }
}
