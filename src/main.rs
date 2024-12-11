use dotenv::dotenv;
use futures::StreamExt;
use notify_service::controllers::{clients, health, notify_machine, ClientMap};
use ntex::web;
use std::{collections::HashMap, sync::{Arc, Mutex}};
use notify_service::services::websocket_service::ws_index;

#[ntex::main]
async fn main() -> std::io::Result<()> {
    dotenv().ok();
    env_logger::init();

    let clients_map: ClientMap = Arc::new(Mutex::new(HashMap::new()));

    web::server(move || {
        web::App::new()
            .state(clients_map.clone())
            .wrap(web::middleware::Logger::default())
            .service(web::resource("/ws/{machine_id}").route(web::get().to(ws_index)))
            .service(health)
            .service(notify_machine)
            .service(clients)
    })
        .bind("127.0.0.1:8080")?
        .run()
        .await
}