mod response;

use crate::models::notify::NotifyMachineRequest;
use futures::SinkExt;
use ntex::web::types::Json;
use ntex::web::{self, HttpResponse, Responder};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use ntex::fn_service;

pub struct NotifyController;

/// Shared state type for managing WebSocket connections

#[web::get("/health")]
pub async fn health() -> impl Responder {
    HttpResponse::Ok().body("OK")
}

#[web::post("/notify-machine")]
pub async fn notify_machine(
    req: Json<NotifyMachineRequest>,
    state: web::types::State<Arc<Mutex<HashMap<String, Instant>>>>,
) -> impl Responder {
    let message = format!(
        "Machine ID: {}, User ID: {}, Message: {}",
        req.machine_id, req.user_id, req.message
    );


    HttpResponse::Ok().body("Notification sent to all clients")
}
