pub mod cli;

use bytes::BytesMut;
use ephemera::request::rb_msg::ReliableBroadcast::PrePrepare;
use ephemera::request::{PrePrepareMsg, RbMsg};
use prost_types::Timestamp;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use uuid::Uuid;

pub struct RbClient<R> {
    pub payload_stream: R,
    pub node_address: String,
}

impl<R: AsyncRead + Unpin> RbClient<R> {
    pub fn new(node_address: String, payload_stream: R) -> Self {
        RbClient {
            payload_stream,
            node_address,
        }
    }

    pub async fn run_reliable_broadcast(&mut self) {
        let mut conn = TcpStream::connect(&self.node_address).await.unwrap();
        loop {
            let mut buf = BytesMut::new();
            self.payload_stream.read_buf(&mut buf).await.unwrap();
            let payload = buf.to_vec();

            let msg = pre_prepare_msg("client", payload);
            let mut message = quorum_message(msg);
            conn.write_buf(&mut message).await.unwrap();
        }
    }
}

pub fn quorum_message(msg: RbMsg) -> bytes::Bytes {
    println!("Sending request {:?}", msg);

    let mut buf = BytesMut::with_capacity(1028);
    prost::Message::encode_length_delimited(&msg, &mut buf).unwrap();

    buf.freeze()
}

pub fn pre_prepare_msg(sender_id: &str, payload: Vec<u8>) -> RbMsg {
    let timestamp = Timestamp::from(std::time::SystemTime::now());
    let request = RbMsg {
        id: Uuid::new_v4().to_string(),
        node_id: sender_id.to_string(),
        timestamp: Some(timestamp),
        custom_message_id: format!("epoch-{}", Uuid::new_v4().to_string()),
        reliable_broadcast: Some(PrePrepare(PrePrepareMsg { payload })),
    };
    request
}
