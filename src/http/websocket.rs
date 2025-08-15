use std::collections::HashMap;
use std::sync::Arc;
use axum::extract::ws::{Utf8Bytes, WebSocket};
use futures::{SinkExt, StreamExt};
use tokio::sync::{mpsc, RwLock};
use tokio::sync::mpsc::UnboundedSender;
use tracing::log::{debug, error};
use uuid::Uuid;
use crate::events::Event;
use crate::tokio_select_with_logging;

#[derive(Clone)]
pub struct WebSocketManager {
    connections: Arc<RwLock<HashMap<String, UnboundedSender<Utf8Bytes>>>>
}
impl WebSocketManager {
    pub fn new() -> Self {
        Self { connections: Arc::new(RwLock::new(HashMap::new())) }
    }

    pub async fn broadcast(&self, event: Event) -> usize {
        let message = match serde_json::to_string(&event) {
            Ok(msg) => Utf8Bytes::from(msg),
            Err(e) => {
                error!("Couldn't broadcast event '{:?}' due to serialization error: {} ", event, e);
                return 0
            }
        };

        let connections = self.connections.read().await;
        let mut successful_sends = 0;
        let mut failed_connections = Vec::new();

        // Try to broadcast to all connections.
        for (id, sender) in connections.iter() {
            if sender.send(message.clone()).is_ok() {
                successful_sends += 1;
            } else {
                failed_connections.push(id.clone());
            }
        }
        drop(connections);

        // Cleanup failed connections (read lock dropped before acquiring write).
        if !failed_connections.is_empty() {
            let mut connections = self.connections.write().await;
            for id in failed_connections {
                connections.remove(&id);
            }
        }
        successful_sends
    }

    pub async fn add_connection(&self, tx: UnboundedSender<Utf8Bytes>) -> String {
        loop {
            let id = Uuid::new_v4().to_string();
            let mut connections = self.connections.write().await;

            if !connections.contains_key(&id) {
                connections.insert(id.clone(), tx);
                return id;
            }
            drop(connections);
        }
    }

    pub async fn remove_connection(&self, id: &str) {
        self.connections.write().await.remove(id);
    }
}

// Called after the connection is upgraded.
pub async fn handle_websocket(socket: WebSocket, manager: WebSocketManager) {
    let (mut sender, mut receiver) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<Utf8Bytes>();

    // Add connection.
    let connection_id = manager.add_connection(tx.clone()).await;
    debug!("WebSocket connection established: {}", connection_id);

    // Writer task.
    let connection_id_for_tx = connection_id.clone();
    let (ping_tx, mut ping_rx) = mpsc::unbounded_channel();
    let tx_task = tokio::spawn(async move {
        loop {
            tokio::select! {
                // Outgoing messages.
                msg = rx.recv() => {
                    match msg {
                        Some(msg) => {
                            if sender.send(axum::extract::ws::Message::Text(msg)).await.is_err() {
                                break;
                            }
                        }
                        None => break // Channel closed
                    }
                }
                // Handle ping responses (pong messages).
                ping_data = ping_rx.recv() => {
                    match ping_data {
                        Some(data) => {
                            if sender.send(axum::extract::ws::Message::Pong(data)).await.is_err() {
                                break;
                            }
                        }
                        None => break // Channel closed
                    }
                }
            }
        }
    });

    // Reader.
    let rx_task = tokio::spawn(async move {
        while let Some(msg) = receiver.next().await {
            match msg {
                Ok(axum::extract::ws::Message::Text(text)) => debug!("Received WebSocket message from {}: {:?}", connection_id, text),
                Ok(axum::extract::ws::Message::Ping(ping)) => {
                    if ping_tx.send(ping).is_err() {
                        break;
                    }
                },
                Ok(axum::extract::ws::Message::Close(_)) => {
                    debug!("WebSocket connection closed: {}", connection_id);
                    break;
                },
                Err(e) => {
                    error!("WebSocket error for {}: {}", connection_id, e);
                    break;
                },
                _ => {}
            }
        }
    });

    tokio_select_with_logging! {
        "WebSocket TX" => tx_task,
        "WebSocket RX" => rx_task
    }

    // Remove connection after either task finishes.
    manager.remove_connection(&connection_id_for_tx).await;
    debug!("WebSocket connection cleaned up: {}", connection_id_for_tx);
}