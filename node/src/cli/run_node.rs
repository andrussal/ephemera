use std::sync::Arc;

use clap::Parser;
use log::trace;
use tokio::signal::unix::{signal, SignalKind};

use crate::{
    api::application::CheckBlockResult,
    cli::PEERS_CONFIG_FILE,
    codec::EphemeraEncoder,
    config::Configuration,
    core::builder::EphemeraStarter,
    crypto::EphemeraKeypair,
    crypto::Keypair,
    ephemera_api::{
        ApiBlock, ApiEphemeraMessage, Application, DummyApplication, RawApiEphemeraMessage, Result,
    },
    membership::{DummyMembersProvider, HttpMembersProvider},
    network::members::ConfigMembersProvider,
    utilities::encoding::Encoder,
};

#[derive(Debug, Clone, Parser)]
pub struct RunExternalNodeCmd {
    #[clap(short, long)]
    pub config_file: String,
}

impl RunExternalNodeCmd {
    pub async fn execute(&self) -> anyhow::Result<()> {
        let ephemera_conf = match Configuration::try_load(self.config_file.clone()) {
            Ok(conf) => conf,
            Err(err) => anyhow::bail!("Error loading configuration file: {err:?}"),
        };

        let _dummy_members_provider = DummyMembersProvider::empty_peers_list();
        let _config_members_provider = Self::config_members_provider()?;
        let http_members_provider =
            Self::http_members_provider("http://localhost:8000/peers".to_string())?;

        let ephemera = EphemeraStarter::new(ephemera_conf.clone())
            .unwrap()
            .with_application(DummyApplication)
            .with_members_provider(Box::pin(http_members_provider))
            .init_tasks()
            .await
            .unwrap();

        let mut ephemera_shutdown = ephemera.ephemera_handle.shutdown.clone();

        let ephemera_handle = tokio::spawn(ephemera.run());

        let shutdown = async {
            let mut stream_int = signal(SignalKind::interrupt()).unwrap();
            let mut stream_term = signal(SignalKind::terminate()).unwrap();
            tokio::select! {
                _ = stream_int.recv() => {
                    ephemera_shutdown.shutdown();
                }
                _ = stream_term.recv() => {
                   ephemera_shutdown.shutdown();
                }
            }
        };

        //Wait shutdown signal
        shutdown.await;
        ephemera_handle.await.unwrap();
        Ok(())
    }

    fn config_members_provider() -> anyhow::Result<ConfigMembersProvider> {
        let peers_conf_path = Configuration::ephemera_root_dir()
            .unwrap()
            .join(PEERS_CONFIG_FILE);

        let peers_conf = match ConfigMembersProvider::init(peers_conf_path) {
            Ok(conf) => conf,
            Err(err) => anyhow::bail!("Error loading peers file: {err:?}"),
        };
        Ok(peers_conf)
    }

    fn http_members_provider(url: String) -> anyhow::Result<HttpMembersProvider> {
        let http_members_provider = HttpMembersProvider::new(url);
        Ok(http_members_provider)
    }
}

pub struct SignatureVerificationApplication {
    keypair: Arc<Keypair>,
}

impl SignatureVerificationApplication {
    pub fn new(keypair: Arc<Keypair>) -> Self {
        Self { keypair }
    }

    pub(crate) fn verify_message(&self, msg: ApiEphemeraMessage) -> anyhow::Result<()> {
        let signature = msg.certificate.clone();
        let raw_message: RawApiEphemeraMessage = msg.into();
        let encoded_message = Encoder::encode(&raw_message)?;
        if self.keypair.verify(&encoded_message, &signature.signature) {
            Ok(())
        } else {
            anyhow::bail!("Invalid signature")
        }
    }
}

impl Application for SignatureVerificationApplication {
    fn check_tx(&self, tx: ApiEphemeraMessage) -> Result<bool> {
        trace!("SignatureVerificationApplicationHook::check_tx");
        self.verify_message(tx)?;
        Ok(true)
    }

    fn check_block(&self, _block: &ApiBlock) -> Result<CheckBlockResult> {
        Ok(CheckBlockResult::Accept)
    }

    fn deliver_block(&self, _block: ApiBlock) -> Result<()> {
        trace!("SignatureVerificationApplicationHook::deliver_block");
        Ok(())
    }
}
