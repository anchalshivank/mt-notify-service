mod response;

use crate::models::notify::NotifyMachineRequest;
use futures::channel::mpsc;
use futures::SinkExt;
use log::info;
use ntex::web::types::Json;
use ntex::web::{self, HttpResponse, Responder};
use ntex::ws::WsSink;
use ntex::{fn_service, ws};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use crate::controllers::response::ApiResponse;

pub struct NotifyController;

pub type ClientMap = Arc<Mutex<HashMap<String, mpsc::UnboundedSender<ws::Message>>>>;

#[web::get("/health")]
pub async fn health() -> impl Responder {
    HttpResponse::Ok().json(&ApiResponse::<(), ()>::success("Service is running", None))
}
#[web::post("/notify-machine")]
pub async fn notify_machine(
    req: web::types::Json<NotifyMachineRequest>,
    state: web::types::State<ClientMap>,
) -> impl web::Responder {
    let message = format!(
        "Machine ID: {}, User ID: {}, Message: {}",
        req.machine_id, req.user_id, req.message
    );

    info!("{}", message);

    let clients_map = match state.lock() {
        Ok(map) => map,
        Err(_) => {
            return HttpResponse::InternalServerError().json(&ApiResponse::<(), String>::error(
                "Failed to acquire state lock",
                "Lock is poisoned".to_string(),
            ));
        }
    };

    if let Some(tx) = clients_map.get(&req.machine_id) {
        if let Err(e) = tx.unbounded_send(ws::Message::Text(message.clone().into())) {
            info!("Failed to send message to client {}: {}", req.machine_id, e);
            return HttpResponse::InternalServerError().json(&ApiResponse::<(), String>::error(
                "Failed to send message",
                e.to_string(),
            ));
        }
        HttpResponse::Ok().json(&ApiResponse::<(), ()>::success(
            &format!("Notification sent successfully to machine with id {}", req.machine_id),
            None,
        ))
    } else {
        HttpResponse::NotFound().json(&ApiResponse::<(), String>::error(
            "Client is not connected",
            format!("There is no socket connection with id {}", req.machine_id.clone()),
        ))
    }
}


#[web::get("/clients")]
pub async fn clients(state: web::types::State<ClientMap>) -> impl web::Responder {
    let clients = state.lock().unwrap();
    let client_list: Vec<String> = clients.keys().cloned().collect();
    HttpResponse::Ok().json(&ApiResponse::<Vec<String>, ()>::success(
        "List of connected clients",
        Option::from(client_list),
    ))
}
