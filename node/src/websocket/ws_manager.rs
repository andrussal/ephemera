use anyhow::Result;
use futures_util::SinkExt;
use std::net::SocketAddr;
use tokio::{
    net::{TcpListener, TcpStream},
    sync::broadcast,
};
use tokio_tungstenite::{tungstenite::Message, WebSocketStream};

use crate::api::types::ApiBlock;
use crate::block::types::block::Block;
use crate::config::WsConfig;

pub struct WsConnection {
    socket: WebSocketStream<TcpStream>,
    pending_messages_rx: broadcast::Receiver<Message>,
    address: SocketAddr,
}

impl WsConnection {
    pub fn new(
        socket: WebSocketStream<TcpStream>,
        pending_messages_rx: broadcast::Receiver<Message>,
        address: SocketAddr,
    ) -> WsConnection {
        WsConnection {
            socket,
            pending_messages_rx,
            address,
        }
    }

    pub async fn accept_messages(mut self) {
        loop {
            match self.pending_messages_rx.recv().await {
                Ok(msg) => {
                    log::debug!("Sending message to {}", self.address);
                    if let Err(err) = self.socket.send(msg).await {
                        log::error!(
                            "Error sending message to websocket client: {:?}, dropping connection",
                            err
                        );
                        break;
                    }
                }
                Err(e) => {
                    //TODO:: shutdown?
                    log::error!(
                        "Error receiving message from broadcast channel: {:?}, dropping connection",
                        e
                    );
                    break;
                }
            }
        }
    }
}

#[derive(Clone)]
pub(crate) struct WsMessageBroadcaster {
    pub(crate) pending_messages_tx: broadcast::Sender<Message>,
}

impl WsMessageBroadcaster {
    pub(crate) fn new(pending_messages_tx: broadcast::Sender<Message>) -> WsMessageBroadcaster {
        WsMessageBroadcaster {
            pending_messages_tx,
        }
    }

    pub(crate) fn send_block(&self, block: &Block) -> Result<()> {
        log::debug!("Sending block {} to websocket clients", block.header.hash);
        let json = serde_json::to_string::<ApiBlock>(block.into())?;
        let msg = Message::Text(json);
        self.pending_messages_tx.send(msg)?;
        Ok(())
    }
}

pub(crate) struct WsManager {
    pub(crate) listener: Option<TcpListener>,
    pub(crate) config: WsConfig,
    pub(crate) pending_messages_tx: broadcast::Sender<Message>,
    _pending_messages_rcv: broadcast::Receiver<Message>,
}

impl WsManager {
    pub(crate) fn new(config: WsConfig) -> (WsManager, WsMessageBroadcaster) {
        let (pending_messages_tx, _pending_messages_rcv) = broadcast::channel(1000);
        let ws_message_broadcast = WsMessageBroadcaster::new(pending_messages_tx.clone());
        let manager = WsManager {
            listener: None,
            config,
            pending_messages_tx,
            _pending_messages_rcv,
        };
        (manager, ws_message_broadcast)
    }

    pub(crate) async fn listen(&mut self) -> Result<()> {
        let listener = TcpListener::bind(&self.config.ws_address).await?;
        log::info!(
            "Listening for websocket connections on {}",
            self.config.ws_address
        );
        self.listener = Some(listener);
        Ok(())
    }

    pub async fn run(mut self) -> Result<()> {
        let listener = self.listener.take().expect("Listener not set");
        loop {
            tokio::select! {
                res = listener.accept() => {
                    match res {
                        Ok((stream, addr)) => {
                            log::debug!("Accepted websocket connection from: {}", addr);
                            self.handle_connection(stream, addr);
                        }
                        Err(err) => {
                            return Err(err.into());
                        }
                    }
                }
            }
        }
    }

    pub fn handle_connection(&self, stream: TcpStream, addr: SocketAddr) {
        let pending_messages_rx = self.pending_messages_tx.subscribe();
        tokio::spawn(async move {
            match tokio_tungstenite::accept_async(stream).await {
                Ok(ws_stream) => {
                    let connection = WsConnection::new(ws_stream, pending_messages_rx, addr);
                    connection.accept_messages().await;
                }
                Err(err) => {
                    log::error!("Error accepting websocket connection: {:?}", err);
                }
            }
            log::debug!("Websocket connection closed");
        });
    }
}
