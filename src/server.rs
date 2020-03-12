use actix::prelude::*;
use actix_broker::BrokerSubscribe;
use rand;

use std::collections::HashMap;
use std::mem;

// #[derive(Message, Clone)]
// #[rtype(result = "()")]
// pub struct ChatMessage(pub String);

#[derive(Message, Clone)]
#[rtype(result = "()")]
pub enum ChatResponseMessage {
    Message { name: String, message: String },
    Joined { room: String, name: String },
    NameChange { name: String },
    ListRooms { rooms: Vec<String> },
}

#[derive(Clone, Message)]
#[rtype(result = "usize")]
pub struct JoinRoom(
    pub String,
    pub Option<String>,
    pub Recipient<ChatResponseMessage>,
);

#[derive(Clone, Message)]
#[rtype(result = "()")]
pub struct LeaveRoom(pub String, pub usize);

#[derive(Clone, Message)]
#[rtype(result = "Vec<String>")]
pub struct ListRooms;

#[derive(Clone, Message)]
#[rtype(result = "()")]
pub struct SendMessage {
    pub name: String,
    pub room_name: String,
    pub id: usize,
    pub msg: String,
}

type Client = Recipient<ChatResponseMessage>;
type Room = HashMap<usize, Client>;
#[derive(Default)]
pub struct WsChatServer {
    rooms: HashMap<String, Room>,
}

impl WsChatServer {
    fn take_room(&mut self, room_name: &str) -> Option<Room> {
        let room = self.rooms.get_mut(room_name)?;
        let room = mem::replace(room, HashMap::new());
        Some(room)
    }

    fn add_client_to_room(&mut self, room_name: &str, id: Option<usize>, client: Client) -> usize {
        let mut id = id.unwrap_or_else(rand::random::<usize>);
        if let Some(room) = self.rooms.get_mut(room_name) {
            loop {
                if room.contains_key(&id) {
                    id = rand::random::<usize>();
                } else {
                    break;
                }
            }
            room.insert(id, client);
            return id;
        }
        // Create a new room for the first client
        let mut room: Room = HashMap::new();
        room.insert(id, client);
        self.rooms.insert(room_name.to_owned(), room);
        id
    }

    fn send_to_room(
        &mut self,
        room_name: &str,
        msg: ChatResponseMessage,
        _src: usize,
    ) -> Option<()> {
        let mut room = self.take_room(room_name)?;
        for (id, client) in room.drain() {
            if client.do_send(msg.clone()).is_ok() {
                self.add_client_to_room(room_name, Some(id), client);
            }
        }
        Some(())
    }
}

impl Actor for WsChatServer {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        self.subscribe_system_async::<LeaveRoom>(ctx);
        self.subscribe_system_async::<SendMessage>(ctx);
    }
}

impl Handler<JoinRoom> for WsChatServer {
    type Result = MessageResult<JoinRoom>;

    fn handle(&mut self, msg: JoinRoom, _ctx: &mut Self::Context) -> Self::Result {
        let JoinRoom(room_name, client_name, client) = msg;
        let id = self.add_client_to_room(&room_name, None, client);
        let name = client_name.unwrap_or_else(|| "anon".to_string());
        self.send_to_room(
            &room_name,
            ChatResponseMessage::Joined {
                room: room_name.to_owned(),
                name,
            },
            id,
        );
        MessageResult(id)
    }
}

impl Handler<LeaveRoom> for WsChatServer {
    type Result = ();

    fn handle(&mut self, msg: LeaveRoom, _ctx: &mut Self::Context) {
        if let Some(room) = self.rooms.get_mut(&msg.0) {
            room.remove(&msg.1);
        }
    }
}

impl Handler<ListRooms> for WsChatServer {
    type Result = MessageResult<ListRooms>;

    fn handle(&mut self, _: ListRooms, _ctx: &mut Self::Context) -> Self::Result {
        MessageResult(self.rooms.keys().cloned().collect())
    }
}

impl Handler<SendMessage> for WsChatServer {
    type Result = ();

    fn handle(&mut self, msg: SendMessage, _ctx: &mut Self::Context) {
        let SendMessage { name, room_name, id, msg } = msg;
        self.send_to_room(
            &room_name,
            ChatResponseMessage::Message { name, message: msg },
            id,
        );
    }
}

impl SystemService for WsChatServer {}
impl Supervised for WsChatServer {}
