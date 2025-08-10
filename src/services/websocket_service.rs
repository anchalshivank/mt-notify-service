use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use futures::channel::mpsc;
use futures::StreamExt;
use log::{info, warn, error};
use ntex::{chain, fn_service, rt, web, ws, Service};
use ntex::channel::oneshot;
use ntex::service::{fn_factory_with_config, fn_shutdown, map_config};
use ntex::util::Bytes;
use crate::controllers::ClientMap;
use crate::models::notify::NotifyMachineRequest;

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
const CLIENT_TIMEOUT: Duration = Duration::from_secs(10);

struct WsState {
    hb: Instant,
}

async fn ws_service(
    (sink, clients_state, machine_id): (ws::WsSink, web::types::State<ClientMap>, String),
) -> Result<
    impl Service<ws::Frame, Response = Option<ws::Message>, Error = std::io::Error>,
    web::Error,
> {
    info!("Attempting to establish WebSocket connection for machine_id: {}", machine_id);
    
    // Create a channel for communication with the WebSocket client
    let (tx, mut rx) = mpsc::unbounded();

    // Handle client registration with better error handling
    {
        let mut clients_map = match clients_state.lock() {
            Ok(map) => map,
            Err(e) => {
                error!("Failed to acquire state lock: {:?}", e);
                return Err(web::Error::from(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "Failed to acquire state lock",
                )));
            }
        };

        // Remove any existing stale connection for this machine_id
        if let Some(old_tx) = clients_map.remove(&machine_id) {
            warn!("Replacing existing connection for machine_id: {}", machine_id);
            // Try to close the old sender gracefully
            drop(old_tx);
        }
        
        clients_map.insert(machine_id.clone(), tx);
        info!("Successfully registered WebSocket connection for machine_id: {}", machine_id);
    }

    // Task to forward messages from `rx` to the WebSocket sink
    let sink_clone = sink.clone();
    let machine_id_clone = machine_id.clone();
    rt::spawn(async move {
        while let Some(msg) = rx.next().await {
            if let Err(e) = sink_clone.send(msg).await {
                warn!("Failed to send message to client {}: {}", machine_id_clone, e);
                break;
            }
        }
        info!("Message forwarding task ended for client: {}", machine_id_clone);
    });

    let state = Arc::new(Mutex::new(WsState { hb: Instant::now() }));

    // Disconnect notification
    let (shutdown_tx, shutdown_rx) = oneshot::channel();

    // Start heartbeat task
    let heartbeat_machine_id = machine_id.clone();
    let heartbeat_clients_state = clients_state.clone();
    rt::spawn(heartbeat(
        state.clone(), 
        sink.clone(), 
        shutdown_rx, 
        heartbeat_machine_id,
        heartbeat_clients_state
    ));

    // Handler service for incoming WebSocket frames
    let frame_state = state.clone();
    let service = fn_service(move |frame| {
        let mut ws_state = match frame_state.lock() {
            Ok(state) => state,
            Err(e) => {
                error!("Failed to acquire WebSocket state lock: {:?}", e);
                return futures::future::ready(Ok(None));
            }
        };

        let response = match frame {
            ws::Frame::Ping(msg) => {
                info!("Ping received from client");
                ws_state.hb = Instant::now();
                Some(ws::Message::Pong(msg))
            }
            ws::Frame::Pong(_) => {
                info!("Pong received from client");
                ws_state.hb = Instant::now();
                None
            }
            ws::Frame::Text(text) => {
                info!("Text message received: {:?}", text);
                ws_state.hb = Instant::now();
                Some(ws::Message::Text(String::from_utf8(Vec::from(text.as_ref())).unwrap_or_default().into()))
            }
            ws::Frame::Binary(bin) => {
                info!("Binary message received");
                ws_state.hb = Instant::now();
                Some(ws::Message::Binary(bin))
            }
            ws::Frame::Close(reason) => {
                info!("Close frame received: {:?}", reason);
                Some(ws::Message::Close(reason))
            }
            _ => None,
        };
        futures::future::ready(Ok(response))
    });

    let cleanup_machine_id = machine_id.clone();
    let cleanup_clients_state = clients_state.clone();
    let on_shutdown = fn_shutdown(move || {
        let _ = shutdown_tx.send(());
        info!("Client disconnected: {}", cleanup_machine_id);

        // Clean up the client from the map
        let mut clients_map = match cleanup_clients_state.lock() {
            Ok(map) => map,
            Err(e) => {
                error!("Failed to acquire state lock on shutdown: {:?}", e);
                return;
            }
        };

        if clients_map.remove(&cleanup_machine_id).is_some() {
            info!("Client {} successfully removed from the state.", cleanup_machine_id);
        } else {
            warn!("Client {} was not found in the state during cleanup.", cleanup_machine_id);
        }
    });

    Ok(chain(service).and_then(on_shutdown))
}

async fn heartbeat(
    state: Arc<Mutex<WsState>>,
    sink: ws::WsSink,
    mut shutdown: oneshot::Receiver<()>,
    machine_id: String,
    clients_state: web::types::State<ClientMap>,
) {
    info!("Starting heartbeat task for client: {}", machine_id);
    
    loop {
        match futures::future::select(Box::pin(ntex::time::sleep(HEARTBEAT_INTERVAL)), &mut shutdown).await {
            futures::future::Either::Left(_) => {
                let now = Instant::now();
                let last_heartbeat = {
                    let ws_state = match state.lock() {
                        Ok(state) => state,
                        Err(e) => {
                            error!("Failed to acquire WebSocket state lock during heartbeat: {:?}", e);
                            break;
                        }
                    };
                    ws_state.hb
                };

                if now.duration_since(last_heartbeat) > CLIENT_TIMEOUT {
                    error!("Client {} heartbeat timeout, disconnecting!", machine_id);
                    
                    // Clean up the client from the map on timeout
                    if let Ok(mut clients_map) = clients_state.lock() {
                        if clients_map.remove(&machine_id).is_some() {
                            info!("Client {} removed from state due to heartbeat timeout.", machine_id);
                        }
                    }
                    break;
                }

                // Send ping to client
                if let Err(e) = sink.send(ws::Message::Ping(Bytes::default())).await {
                    warn!("Failed to send ping to client {}: {}", machine_id, e);
                    
                    // Clean up the client from the map on send failure
                    if let Ok(mut clients_map) = clients_state.lock() {
                        if clients_map.remove(&machine_id).is_some() {
                            info!("Client {} removed from state due to ping failure.", machine_id);
                        }
                    }
                    break;
                }
            }
            futures::future::Either::Right(_) => {
                info!("Heartbeat task stopping for client: {} (shutdown signal received)", machine_id);
                break;
            }
        }
    }
    
    info!("Heartbeat task ended for client: {}", machine_id);
}

pub async fn ws_index(
    req: web::HttpRequest,
    state: web::types::State<ClientMap>,
    path: web::types::Path<String>,
) -> Result<web::HttpResponse, web::Error> {
    let machine_id = path.clone();
    
    info!("WebSocket connection request for machine_id: {}", machine_id);
    
    // Log headers for debugging
    if let Some(upgrade) = req.headers().get("upgrade") {
        info!("Upgrade header: {:?}", upgrade);
    }
    if let Some(connection) = req.headers().get("connection") {
        info!("Connection header: {:?}", connection);
    }
    if let Some(ws_key) = req.headers().get("sec-websocket-key") {
        info!("WebSocket key: {:?}", ws_key);
    }
    
    // Clone machine_id before moving it into the closure
    let machine_id_for_closure = machine_id.clone();
    let config = map_config(fn_factory_with_config(ws_service), move |cfg| {
        (cfg, state.clone(), machine_id_for_closure.clone())
    });
    
    match web::ws::start(req, config).await {
        Ok(response) => {
            info!("WebSocket connection established successfully for machine_id: {}", machine_id);
            Ok(response)
        }
        Err(e) => {
            error!("Failed to establish WebSocket connection for machine_id {}: {:?}", machine_id, e);
            Err(e)
        }
    }
}
