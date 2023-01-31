//! # Ephemera Node

//! An Ephemera node does reliable broadcast of inbound messages to all other Ephemera nodes in the cluster.
//!
//! Each node has a unique ID, and each message is signed by the node that first received it.
//! Messages are then re-broadcast and re-signed by all other nodes in the cluster.
//!
//! At the end of the process, each message is signed by every node in the cluster, and each node has also
//! signed all messages that were broadcast by other nodes. This means that nodes are unable to repudiate messages
//! once they are seen and signed, so there is a strong guarantee of message integrity within the cluster.
//!
//! # Why would I want this?
//!
//! Let's say you have blockchain system that needs to ship large amounts of information around, but the information
//! is relatively short-lived. You could use a blockchain to store the information, but that would be expensive,
//! slow, and wasteful. Instead, you could use Ephemera to broadcast the information to all nodes in the cluster,
//! and then store only a cryptographic commitment in the blockchain's data store.
//!
//! Ephemera nodes then keep messages around for inspection in a data availability layer (accessible over HTTP)
//! so that interested parties can verify correctness. Ephemeral information can then be automatically discarded
//! once it's no longer useful.
//!
//! This gives very similar guarantees to a blockchain, but without incurring the permanent storage costs.
//!
//! Note that it *requires* a blockchain to be present.

pub mod api;
pub mod broadcast_protocol;
pub mod cli;
pub mod config;
pub mod crypto;
mod database;
pub mod http;
pub mod logging;
pub mod network;

pub mod request {
    include!(concat!(env!("OUT_DIR"), "/broadcast.rs"));
}
