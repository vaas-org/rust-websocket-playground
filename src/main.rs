#[macro_use]
extern crate log;

use actix::fut;
use actix::prelude::*;
use actix_broker::BrokerIssue;
use actix_files::Files;
use actix_web::{web, App, Error, HttpRequest, HttpResponse, HttpServer};
use actix_web_actors::ws;
use serde::{Deserialize, Serialize};

mod server;

/// Entry point for our route
async fn chat_route(req: HttpRequest, stream: web::Payload) -> Result<HttpResponse, Error> {
    ws::start(WsChatSession::default(), &req, stream)
}

#[derive(Default)]
struct WsChatSession {
    /// unique session id
    id: usize,
    /// joined room
    room: String,
    /// peer name
    name: Option<String>,
}

impl WsChatSession {
    fn join_room(&mut self, room_name: &str, ctx: &mut ws::WebsocketContext<Self>) {
        let room_name = room_name.to_owned();
        // First send a leave message for the current room
        let leave_msg = server::LeaveRoom(self.room.clone(), self.id);
        // issue_sync comes from having the `BrokerIssue` trait in scope.
        self.issue_system_sync(leave_msg, ctx);
        // Then send a join message for th  e new room
        let join_msg = server::JoinRoom(
            room_name.to_owned(),
            self.name.clone(),
            ctx.address().recipient(),
        );

        server::WsChatServer::from_registry()
            .send(join_msg)
            .into_actor(self)
            .then(|id, act, _ctx| {
                if let Ok(id) = id {
                    act.id = id;
                    act.room = room_name;
                }

                fut::ready(())
            })
            .spawn(ctx);
    }

    fn list_rooms(&mut self, ctx: &mut ws::WebsocketContext<Self>) {
        server::WsChatServer::from_registry()
            .send(server::ListRooms)
            .into_actor(self)
            .then(|res, _, ctx| {
                if let Ok(rooms) = res {
                    for room in rooms {
                        ctx.text(room);
                    }
                }
                fut::ready(())
            })
            .spawn(ctx);
    }

    fn send_msg(&self, msg: &str) {
        let name = self.name.clone().unwrap_or_else(|| "anon".to_string());
        let msg = server::SendMessage {
            room_name: self.room.clone(),
            id: self.id,
            msg: msg.to_owned(),
            name,
        };
        // issue_async comes from having the `BrokerIssue` trait in scope.
        self.issue_system_async(msg);
    }

    // TODO: Should probably not just be a function?
    fn send_json<T: Serialize>(&self, ctx: &mut ws::WebsocketContext<Self>, value: &T) {
        match serde_json::to_string(value) {
            Ok(json) => ctx.text(json),
            Err(error) => println!("Failed to convert to JSON {:#?}", error),
        }
    }
}

impl Actor for WsChatSession {
    type Context = ws::WebsocketContext<Self>;

    /// Method is called on actor start.
    /// We register ws session with ChatServer
    fn started(&mut self, ctx: &mut Self::Context) {
        self.join_room("Main", ctx);
    }

    fn stopped(&mut self, _ctx: &mut Self::Context) {
        info!(
            "WsChatSession closed for {}({}) in room {}",
            self.name.clone().unwrap_or_else(|| "anon".to_string()),
            self.id,
            self.room
        );
    }
}

// Handle enum response from server
impl Handler<server::ChatResponseMessage> for WsChatSession {
    type Result = ();

    fn handle(&mut self, msg: server::ChatResponseMessage, ctx: &mut Self::Context) {
        // map to outgoing ws message
        match msg {
            server::ChatResponseMessage::Message { name, message } => {
                self.send_json(ctx, &OutgoingMessage::Message { name, message })
            }
            server::ChatResponseMessage::NameChange { name } => {
                self.send_json(ctx, &OutgoingMessage::NameChange { name })
            }
            server::ChatResponseMessage::Joined { room, name } => {
                self.send_json(ctx, &OutgoingMessage::Joined { room, name })
            }
            server::ChatResponseMessage::ListRooms { rooms } => {
                self.send_json(ctx, &OutgoingMessage::List { rooms })
            }
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum IncomingMessage {
    Message { message: String },
    Join { room: String },
    Name { name: String },
    List,
}

#[derive(Serialize)]
#[serde(tag = "type")]
enum OutgoingMessage {
    Message { name: String, message: String },
    Joined { room: String, name: String },
    NameChange { name: String },
    List { rooms: Vec<String> },
}

/// WebSocket message handler
impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for WsChatSession {
    fn handle(&mut self, msg: Result<ws::Message, ws::ProtocolError>, ctx: &mut Self::Context) {
        let msg = match msg {
            Err(_) => {
                ctx.stop();
                return;
            }
            Ok(msg) => msg,
        };

        println!("WEBSOCKET MESSAGE: {:?}", msg);
        match msg {
            ws::Message::Text(text) => {
                let m = serde_json::from_str(&text);
                if let Err(serde_error) = m {
                    println!("JSON error {:#?}", serde_error);
                    return;
                }
                let m = m.unwrap();
                match m {
                    IncomingMessage::List => self.list_rooms(ctx),
                    IncomingMessage::Join { room } => {
                        self.join_room(&room, ctx);
                    }
                    IncomingMessage::Name { name } => {
                        self.name = Some(name.to_owned());
                        self.send_json(
                            ctx,
                            &OutgoingMessage::NameChange {
                                name: name.to_owned(),
                            },
                        );
                    }
                    IncomingMessage::Message { message } => {
                        // send_json(ctx, &OutgoingMessage::Message { message: message.to_owned() });
                        self.send_msg(&message);
                    }
                }
            }
            ws::Message::Close(_) => {
                ctx.stop();
            }
            _ => {}
        }
    }
}

#[actix_rt::main]
async fn main() -> std::io::Result<()> {
    env_logger::init();
    // Create Http server with websocket support
    HttpServer::new(move || {
        App::new()
            // redirect to websocket.html
            .service(web::resource("/").route(web::get().to(|| {
                HttpResponse::Found()
                    .header("LOCATION", "/static/websocket.html")
                    .finish()
            })))
            // websocket
            .service(web::resource("/ws/").to(chat_route))
            // static resources
            .service(Files::new("/static/", "static/"))
    })
    .bind("127.0.0.1:8080")?
    .run()
    .await
}
