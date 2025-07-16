use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use futures::channel::mpsc;
use futures::StreamExt;
use log::info;
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

// Type alias for storing client connections
async fn ws_service(
    (sink, clients_state, machine_id): (ws::WsSink, web::types::State<ClientMap>, String),
) -> Result<
    impl Service<ws::Frame, Response = Option<ws::Message>, Error = std::io::Error>,
    web::Error,
> {
    // Create a channel for communication with the WebSocket client
    let (tx, mut rx) = mpsc::unbounded();

    {
        let mut clients_map = match clients_state.lock() {
            Ok(map) => map,
            Err(_) => {
                return Err(web::Error::from(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "Failed to acquire state lock",
                )));
            }
        };

        if clients_map.contains_key(&machine_id) {
            return Err(web::Error::from(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "Client already connected",
            )));
        }
        clients_map.insert(machine_id.clone(), tx);
    }

    // Task to forward messages from `rx` to the WebSocket sink
    let sink_clone = sink.clone();
    rt::spawn(async move {
        while let Some(msg) = rx.next().await {
            if let Err(e) = sink_clone.send(msg).await {
                info!("Failed to send message to client: {}", e);
                break;
            }
        }
    });

    let state = Arc::new(Mutex::new(WsState { hb: Instant::now() }));

    // Disconnect notification
    let (shutdown_tx, shutdown_rx) = oneshot::channel();

    // Start heartbeat task
    rt::spawn(heartbeat(state.clone(), sink.clone(), shutdown_rx));

    // Handler service for incoming WebSocket frames
    let service = fn_service(move |frame| {
        let mut ws_state = match state.lock() {
            Ok(state) => state,
            Err(_) => {
                info!("Failed to acquire WebSocket state lock");
                return futures::future::ready(Ok(None));
            }
        };

        let response = match frame {
            ws::Frame::Ping(msg) => {
                info!("Ping: {:?}", msg);
                ws_state.hb = Instant::now();
                Some(ws::Message::Pong(msg))
            }
            ws::Frame::Pong(_) => {
                info!("Pong");
                ws_state.hb = Instant::now();
                None
            }
            ws::Frame::Text(text) => {
                info!("Text: {:?}", text);
                Some(ws::Message::Text(String::from_utf8(Vec::from(text.as_ref())).unwrap_or_default().into()))
            }
            ws::Frame::Binary(bin) => Some(ws::Message::Binary(bin)),
            ws::Frame::Close(reason) => Some(ws::Message::Close(reason)),
            _ => None,
        };
        futures::future::ready(Ok(response))
    });

    let on_shutdown = fn_shutdown(move || {
        let _ = shutdown_tx.send(());
        info!("Client disconnected: {}", machine_id);

        let mut clients_map = match clients_state.lock() {
            Ok(map) => map,
            Err(_) => {
                info!("Failed to acquire state lock on shutdown");
                return;
            }
        };

        if clients_map.remove(&machine_id).is_some() {
            info!("Client {} successfully removed from the state.", machine_id);
        } else {
            info!("Client {} was not found in the state.", machine_id);
        }
    });

    Ok(chain(service).and_then(on_shutdown))
}

async fn heartbeat(
    state: Arc<Mutex<WsState>>,
    sink: ws::WsSink,
    mut shutdown: oneshot::Receiver<()>,
) {
    loop {
        match futures::future::select(Box::pin(ntex::time::sleep(HEARTBEAT_INTERVAL)), &mut shutdown).await {
            futures::future::Either::Left(_) => {
                let now = Instant::now();
                let ws_state = match state.lock() {
                    Ok(state) => state,
                    Err(_) => {
                        info!("Failed to acquire WebSocket state lock during heartbeat");
                        return;
                    }
                };

                if now.duration_since(ws_state.hb) > CLIENT_TIMEOUT {
                    info!("Client heartbeat failed, disconnecting!");
                    return;
                }

                if sink
                    .send(ws::Message::Ping(Bytes::default()))
                    .await
                    .is_err()
                {
                    return;
                }
            }
            futures::future::Either::Right(_) => {
                info!("Connection dropped, stopping heartbeat task.");
                return;
            }
        }
    }
}

pub async fn ws_index(
    req: web::HttpRequest,
    state: web::types::State<ClientMap>,
    path: web::types::Path<String>,
) -> Result<web::HttpResponse, web::Error> {
    let machine_id = path.clone();
      if let Some(upgrade) = req.headers().get("upgrade") {
        log::info!("Upgrade header: {:?}", upgrade);
    }
    if let Some(connection) = req.headers().get("connection") {
        log::info!("Connection header: {:?}", connection);
    }
    if let Some(ws_key) = req.headers().get("sec-websocket-key") {
        log::info!("WebSocket key: {:?}", ws_key);
    }
    let config = map_config(fn_factory_with_config(ws_service), move |cfg| {
        (cfg, state.clone(), machine_id.clone())
    });
    web::ws::start(req, config).await
}
