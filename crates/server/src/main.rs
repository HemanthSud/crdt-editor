// Milestone 3: the server now owns a canonical crdt_core::RgaDoc replica per
// document instead of blindly relaying raw text (Milestone 2). Each connecting
// client is assigned a unique site id (server-issued, so two clients can never
// collide), gets the full op history to seed its own replica, and from then on
// only real CRDT ops flow over the wire. Because RgaDoc::apply_remote_ops is
// idempotent (see crdt-core), rebroadcasting an op back to its own sender is
// harmless - no echo-suppression logic needed, unlike the Milestone 2 relay.

use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
};

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, State,
    },
    response::IntoResponse,
    routing::get,
    Router,
};
use crdt_core::{Op, RgaDoc};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

type DocId = String;

struct DocRoom {
    doc: Mutex<RgaDoc>,
    tx: broadcast::Sender<Op>,
}

#[derive(Clone)]
struct AppState {
    rooms: Arc<Mutex<HashMap<DocId, Arc<DocRoom>>>>,
    next_site_id: Arc<AtomicU64>,
}

impl Default for AppState {
    fn default() -> Self {
        // Site id 0 is reserved for the server's own canonical replica (it never
        // produces local ops, so it never needs a "real" id of its own).
        Self {
            rooms: Arc::new(Mutex::new(HashMap::new())),
            next_site_id: Arc::new(AtomicU64::new(1)),
        }
    }
}

impl AppState {
    fn room(&self, doc_id: &str) -> Arc<DocRoom> {
        let mut rooms = self.rooms.lock().unwrap();
        rooms
            .entry(doc_id.to_string())
            .or_insert_with(|| {
                Arc::new(DocRoom {
                    doc: Mutex::new(RgaDoc::new(0)),
                    tx: broadcast::channel(1024).0,
                })
            })
            .clone()
    }

    fn assign_site_id(&self) -> u64 {
        self.next_site_id.fetch_add(1, Ordering::Relaxed)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerMsg {
    Welcome { site_id: u64, ops: Vec<Op> },
    Op { op: Op },
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientMsg {
    Op { op: Op },
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let state = AppState::default();
    let app = Router::new()
        .route("/ws/{doc_id}", get(ws_handler))
        .with_state(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], 8787));
    tracing::info!("CRDT server listening on ws://{addr}/ws/:doc_id");
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    Path(doc_id): Path<String>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, doc_id, state))
}

async fn handle_socket(socket: WebSocket, doc_id: DocId, state: AppState) {
    let room = state.room(&doc_id);
    let site_id = state.assign_site_id();
    let mut rx = room.tx.subscribe();
    let (mut sender, mut receiver) = socket.split();

    let welcome = {
        let doc = room.doc.lock().unwrap();
        ServerMsg::Welcome {
            site_id,
            ops: doc.ops_since(&HashMap::new()),
        }
    };
    if sender
        .send(Message::Text(serde_json::to_string(&welcome).unwrap().into()))
        .await
        .is_err()
    {
        return;
    }

    let mut send_task = tokio::spawn(async move {
        while let Ok(op) = rx.recv().await {
            let msg = ServerMsg::Op { op };
            let Ok(text) = serde_json::to_string(&msg) else { continue };
            if sender.send(Message::Text(text.into())).await.is_err() {
                break;
            }
        }
    });

    let room_for_recv = room.clone();
    let mut recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            let Message::Text(text) = msg else { continue };
            let Ok(ClientMsg::Op { op }) = serde_json::from_str::<ClientMsg>(&text) else {
                continue;
            };
            {
                let mut doc = room_for_recv.doc.lock().unwrap();
                doc.apply_remote_ops(vec![op.clone()]);
            }
            let _ = room_for_recv.tx.send(op);
        }
    });

    tokio::select! {
        _ = &mut send_task => recv_task.abort(),
        _ = &mut recv_task => send_task.abort(),
    }
}
