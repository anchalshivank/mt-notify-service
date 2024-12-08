use std::{cell::RefCell, io, rc::Rc, time::Duration, time::Instant};
use std::sync::{Arc, Mutex};
use futures::channel::mpsc;
use futures::channel::mpsc::UnboundedSender;
use ntex::web;
use ntex::util::Bytes;
use ntex::{fn_service, chain};
use ntex::{channel::oneshot, rt, time};
use futures::future::{ready, select, Either};
use futures::{SinkExt, StreamExt};
use log::info;
use ntex::service::{fn_factory_with_config, fn_shutdown, map_config, Service};
use notify_service::controllers::{health, notify_machine};

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
const CLIENT_TIMEOUT: Duration = Duration::from_secs(10);

struct WsState {
    hb: Instant,

}

async fn ws_service(

    (sink, mut server, machine_id):(web::ws::WsSink, web::types::State<mpsc::UnboundedSender<ServerMessage>>, String)
) -> Result<
    impl Service<web::ws::Frame, Response = Option<web::ws::Message>, Error = io::Error>,
    web::Error,
> {

    let mut srv = server.get_ref().clone();
    let state = Rc::new(RefCell::new(WsState { hb: Instant::now() }));



    // disconnect notification
    let (tx, rx) = oneshot::channel();

    // start heartbeat task
    rt::spawn(heartbeat(state.clone(), sink, rx));

    // handler service for incoming websockets frames
    let service = fn_service(move |frame| {
        let item = match frame {
            // update heartbeat
            web::ws::Frame::Ping(msg) => {
                info!("Ping: {:?}", msg);
                state.borrow_mut().hb = Instant::now();
                Some(web::ws::Message::Pong(msg))
            }
            // update heartbeat
            web::ws::Frame::Pong(_) => {
                info!("Pong");
                state.borrow_mut().hb = Instant::now();
                None
            }
            // send message back
            web::ws::Frame::Text(text) => Some(web::ws::Message::Text(
                String::from_utf8(Vec::from(text.as_ref())).unwrap().into(),
            )),
            web::ws::Frame::Binary(bin) => Some(web::ws::Message::Binary(bin)),
            // close connection
            web::ws::Frame::Close(reason) => Some(web::ws::Message::Close(reason)),
            // ignore other frames
            _ => None,
        };
        ready(Ok(item))
    });

    // handler service for shutdown notification that stop heartbeat task
    let on_shutdown = fn_shutdown(move || {
        let _ = tx.send(());
    });

    // pipe our service with on_shutdown callback
    Ok(chain(service).and_then(on_shutdown))
}

async fn heartbeat(
    state: Rc<RefCell<WsState>>,
    sink: web::ws::WsSink,
    mut rx: oneshot::Receiver<()>,
) {
    loop {
        match select(Box::pin(time::sleep(HEARTBEAT_INTERVAL)), &mut rx).await {
            Either::Left(_) => {
                // check client heartbeats
                if Instant::now().duration_since(state.borrow().hb) > CLIENT_TIMEOUT {
                    // heartbeat timed out
                    println!("Websocket Client heartbeat failed, disconnecting!");
                    return;
                }

                // send ping
                if sink
                    .send(web::ws::Message::Ping(Bytes::default()))
                    .await
                    .is_err()
                {
                    return;
                }
            }
            Either::Right(_) => {
                println!("Connection is dropped, stop heartbeat task");
                return;
            }
        }
    }
}

#[derive(Debug)]
enum ServerMessage{
    Message(String)
}

async fn start() -> UnboundedSender<ServerMessage>{
    let (tx, mut rx) = mpsc::unbounded();
    rt::Arbiter::new().exec_fn( move ||{
        rt::spawn(async move {
            while let Some(msg) = rx.next().await{
                info!("Received message {:?}", msg);
            }
            rt::Arbiter::current().stop();
        });
    });
    tx
}
async fn ws_index(req: web::HttpRequest, state: web::types::State<mpsc::UnboundedSender<ServerMessage>>, path: web::types::Path<String>) -> Result<web::HttpResponse, web::Error> {
    let machine_id = path.clone();
    let config
        = map_config(fn_factory_with_config(ws_service), move |cfg| {
        (cfg, state.clone(), machine_id.clone())
    });
    web::ws::start(req, config.clone()).await
}

#[ntex::main]
async fn main() -> std::io::Result<()> {

    env_logger::init();

    // let state = Bingo::new();
    let state = start().await;
    web::server(move || {
        web::App::new()
            .state(state.clone())
            // enable logger
            .wrap(web::middleware::Logger::default())
            // websocket route
            .service(web::resource("/ws/{machine_id}").route(web::get().to(ws_index)))
            .service(notify_machine)
            .service(health)
    })
        // start http server on 127.0.0.1:8080
        .bind("127.0.0.1:8080")?
        .run()
        .await
}