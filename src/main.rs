use dotenv::dotenv;
use notify_service::controllers::{clients, health, notify_machine, ClientMap};
use notify_service::services::websocket_service::ws_index;
use ntex::web;
use std::net::SocketAddr;
use std::str::FromStr;
use std::{collections::HashMap, sync::{Arc, Mutex}};

#[ntex::main]
async fn main() -> std::io::Result<()> {
    dotenv().ok();
    env_logger::init();


    let port = std::env::var("PORT")
        .unwrap_or_else(|_| "8081".to_string())
        .parse::<u16>()
        .expect("PORT must be a number!");

    let auth_service_addr = std::env::var("AUTH_SERVICE_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:8080".to_string());
    let auth_service_addr = SocketAddr::from_str(auth_service_addr.as_str())
        .expect("Auth service address is not valid!");

    let addr = SocketAddr::from(([0, 0, 0, 0], port));

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
        .bind(addr)?
        .run()
        .await
}