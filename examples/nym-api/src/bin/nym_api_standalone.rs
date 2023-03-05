//! # Nym-Api rewarding simulation(without Ephemera integration)
//!
//! # Overview
//!
//! It's easier to test out Ephemera integration without full Nym-Api. Simulation the actual rewarding
//! appears to be straightforward.
//!
//! # Metrics generation
//! It generates with configurable interval artificial metrics about mix-nodes availability, represented
//! as a percentage of uptime(0%-100%). Metrics are stored in a database.
//!
//! # Rewards data submittion to smart contract
//! Metrics are collected frequently to get accurate data about mix-nodes uptime. For each Epoch, metrics
//! average is calculated which is simply average of the all the metrics collected during previous Epoch.
//! Average is then sent to the smart contract.

use clap::Parser;
use tokio::signal::unix::{signal, SignalKind};

use nym_api::contract::SmartContract;
use nym_api::nym_api_standalone::NymApi;
use nym_api::{Args, ContractArgs};

#[tokio::main]
async fn main() {
    pretty_env_logger::init();

    let args = Args::parse();
    let contract_args = ContractArgs::parse();

    let nh = tokio::spawn(NymApi::run(args.clone()));

    let sh = tokio::spawn(SmartContract::start(
        contract_args.url,
        contract_args.db_path,
        args.epoch_duration_seconds,
    ));

    let mut stream_int = signal(SignalKind::interrupt()).unwrap();
    let mut stream_term = signal(SignalKind::terminate()).unwrap();
    tokio::select! {
         _ = stream_int.recv() => {
        }
        _ = stream_term.recv() => {
        }
        _ = sh => {
            log::info!("Smart contract exited");
        }
        _ = nh => {
            log::info!("Nym API exited");
        }
    }
}