use std::net::SocketAddr;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncWriteExt, DuplexStream};
use tokio_util::codec::Framed;
use zphira_protocol::{ClientBoundPacket, ClientCodec, GameState, Message, ServerBoundPacket};

use super::Env;

pub struct GameClient {
    framed: Framed<DuplexStream, ClientCodec>,
    pub id: i32,
}

const WAIT: Duration = Duration::from_secs(5);

impl Env {
    pub async fn player(&self, id: i32, name: &str) -> GameClient {
        let (client, server) = tokio::io::duplex(1 << 20);
        let game = self.game.clone();
        let peer = SocketAddr::from(([127, 0, 0, 1], 20000 + (id as u16 % 40000)));
        tokio::spawn(async move { game.serve_connection(server, peer).await });
        let mut stream = client;
        stream.write_all(&[1]).await.unwrap();
        let mut c = GameClient {
            framed: Framed::new(stream, ClientCodec::new()),
            id,
        };
        c.send(ServerBoundPacket::Authenticate {
            token: format!("token-{id}-{name}"),
        })
        .await;
        c.until(|p| matches!(p, ClientBoundPacket::Authenticate(r) if r.is_success()))
            .await;
        c
    }
}

impl GameClient {
    pub async fn send(&mut self, packet: ServerBoundPacket) {
        self.framed.send(packet).await.unwrap();
    }

    pub async fn until(
        &mut self,
        done: impl Fn(&ClientBoundPacket) -> bool,
    ) -> Vec<ClientBoundPacket> {
        let mut seen = Vec::new();
        loop {
            match tokio::time::timeout(WAIT, self.framed.next()).await {
                Ok(Some(Ok(packet))) => {
                    let hit = done(&packet);
                    seen.push(packet);
                    if hit {
                        return seen;
                    }
                }
                other => panic!(
                    "client {}: wanted packet never came ({other:?}); got {seen:?}",
                    self.id
                ),
            }
        }
    }

    pub async fn join(&mut self, room: &str, monitor: bool) {
        self.send(ServerBoundPacket::JoinRoom {
            room_id: room.into(),
            monitor,
        })
        .await;
        self.until(|p| matches!(p, ClientBoundPacket::JoinRoom(r) if r.is_success()))
            .await;
    }

    pub async fn system_line(&mut self, text: &str) {
        self.until(|p| {
            matches!(p, ClientBoundPacket::Message(Message::Chat { user: -1, content }) if content == text)
        })
        .await;
    }

    pub async fn state(&mut self, state: GameState) {
        self.until(move |p| matches!(p, ClientBoundPacket::ChangeState(s) if *s == state))
            .await;
    }

    pub async fn closed(&mut self) -> bool {
        loop {
            match tokio::time::timeout(WAIT, self.framed.next()).await {
                Ok(None) | Ok(Some(Err(_))) => return true,
                Ok(Some(Ok(_))) => continue,
                Err(_) => return false,
            }
        }
    }
}
