use std::fmt::Display;
use std::ops::DerefMut;
use std::sync::Arc;

use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use crate::network::topology::BroadcastTopology;
#[cfg(feature = "rocksdb_storage")]
use crate::storage::rocksdb::RocksDbStorage;
#[cfg(feature = "sqlite_storage")]
use crate::storage::sqlite::SqliteStorage;
use crate::storage::EphemeraDatabase;
use crate::{
    api::application::Application,
    api::{http, ApiListener, EphemeraExternalApi},
    block::{builder::BlockManagerBuilder, manager::BlockManager},
    broadcast::bracha::broadcaster::Broadcaster,
    config::Configuration,
    core::{
        api_cmd::ApiCmdProcessor,
        shutdown::{Shutdown, ShutdownHandle, ShutdownManager},
    },
    crypto::Keypair,
    network::{
        discovery::PeerDiscovery,
        libp2p::{
            ephemera_sender::EphemeraToNetworkSender, network_sender::NetCommunicationReceiver,
            swarm_network::SwarmNetwork,
        },
        peer::{PeerId, ToPeerId},
    },
    utilities::crypto::key_manager::KeyManager,
    websocket::ws_manager::{WsManager, WsMessageBroadcaster},
    Ephemera,
};

#[derive(Clone)]
pub(crate) struct NodeInfo {
    pub(crate) ip: String,
    pub(crate) protocol_port: u16,
    pub(crate) http_port: u16,
    pub(crate) ws_port: u16,
    pub(crate) peer_id: PeerId,
    pub(crate) keypair: Arc<Keypair>,
    pub(crate) initial_config: Configuration,
}

impl NodeInfo {
    pub(crate) fn new(config: Configuration) -> anyhow::Result<Self> {
        let keypair = KeyManager::read_keypair_from_str(&config.node.private_key)?;
        let peer_id = keypair.peer_id();

        let ip = config.node.ip.clone();
        let protocol_port = config.libp2p.port;
        let ws_port = config.websocket.port;
        let http_port = config.http.port;

        let info = Self {
            ip,
            protocol_port,
            http_port,
            ws_port,
            peer_id,
            keypair,
            initial_config: config,
        };
        Ok(info)
    }

    pub(crate) fn protocol_address(&self) -> String {
        format!("/ip4/{}/tcp/{}", self.ip, self.protocol_port)
    }

    pub(crate) fn api_address_http(&self) -> String {
        format!("http://{}:{}", self.ip, self.http_port)
    }

    pub(crate) fn ws_address_ws(&self) -> String {
        format!("ws://{}:{}", self.ip, self.ws_port)
    }

    pub(crate) fn ws_address_ip_port(&self) -> String {
        format!("{}:{}", self.ip, self.ws_port)
    }
}

impl Display for NodeInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "NodeInfo {{ ip: {}, protocol_port: {}, http_port: {}, ws_port: {}, peer_id: {}, keypair: {} }}",
            self.ip,
            self.protocol_port,
            self.http_port,
            self.ws_port,
            self.peer_id,
            self.keypair
        )
    }
}

#[derive(Clone)]
pub struct EphemeraHandle {
    /// Ephemera API
    pub api: EphemeraExternalApi,
    /// Ephemera shutdown handle
    pub shutdown: ShutdownHandle,
}

pub struct EphemeraStarter<P: PeerDiscovery, A: Application> {
    config: Configuration,
    pub(crate) node_info: NodeInfo,
    block_manager_builder: Option<BlockManagerBuilder>,
    block_manager: Option<BlockManager>,
    broadcaster: Broadcaster,
    peer_discovery: Option<P>,
    application: Option<A>,
    from_network: Option<NetCommunicationReceiver>,
    to_network: Option<EphemeraToNetworkSender>,
    ws_message_broadcast: Option<WsMessageBroadcaster>,
    storage: Option<Box<dyn EphemeraDatabase>>,
    api_listener: ApiListener,
    api: EphemeraExternalApi,
}

//TODO: make keypair centrally accessible and coping everywhere(even Arc)
impl<P, A> EphemeraStarter<P, A>
where
    P: PeerDiscovery + 'static,
    A: Application + 'static,
{
    //Crate pure data structures, no resource allocation nor threads
    pub fn new(config: Configuration) -> anyhow::Result<Self> {
        let instance_info = NodeInfo::new(config.clone())?;

        let broadcaster = Broadcaster::new(instance_info.peer_id);

        let block_manager_builder =
            BlockManagerBuilder::new(config.block.clone(), instance_info.keypair.clone());

        let (api, api_listener) = EphemeraExternalApi::new();

        let builder = EphemeraStarter {
            config,
            node_info: instance_info,
            block_manager_builder: Some(block_manager_builder),
            block_manager: None,
            broadcaster,
            peer_discovery: None,
            application: None,
            from_network: None,
            to_network: None,
            ws_message_broadcast: None,
            storage: None,
            api_listener,
            api,
        };
        Ok(builder)
    }

    pub fn with_peer_discovery(self, peer_discovery: P) -> Self {
        Self {
            peer_discovery: Some(peer_discovery),
            ..self
        }
    }

    pub fn with_application(self, application: A) -> EphemeraStarter<P, A> {
        Self {
            application: Some(application),
            ..self
        }
    }

    //opens database and spawns dependent tasks
    pub async fn init_tasks(self) -> anyhow::Result<Ephemera<A>> {
        log::info!("Initializing ephemera tasks...");
        cfg_if::cfg_if! {
            if #[cfg(feature = "sqlite_storage")] {
                let starter = self.connect_sqlite().await?;
            } else if #[cfg(feature = "rocksdb_storage")] {
                let starter = self.connect_rocksdb().await?;
            } else {
                compile_error!("Must enable either sqlite or rocksdb feature");
            }
        };

        let mut builder = starter.init_block_manager().await?;

        let (mut shutdown_manager, shutdown_handle) = ShutdownManager::init();
        builder.start_tasks(&mut shutdown_manager).await?;

        let application = builder.application.take().unwrap();
        let ephemera = builder.ephemera(application, shutdown_handle, shutdown_manager);
        Ok(ephemera)
    }

    //Haven't put much thought into in which order start and stop tasks.
    //(also relative to Ephemera).
    //TODO: Need to write down how all these components work together and depend on each other
    async fn start_tasks(&mut self, shutdown_manager: &mut ShutdownManager) -> anyhow::Result<()> {
        log::info!("Starting network...");
        match self.start_network(shutdown_manager.subscribe()) {
            Ok(nw_task) => {
                shutdown_manager.add_handle(nw_task);
            }
            Err(err) => {
                log::error!("Failed to start network: {}", err);
                return Err(err);
            }
        }

        log::info!("Starting http server...");
        match self.start_http(shutdown_manager.subscribe()) {
            Ok(http_task) => {
                shutdown_manager.add_handle(http_task);
            }
            Err(err) => {
                log::error!("Failed to start http server: {}", err);
                return Err(err);
            }
        }

        log::info!("Starting websocket listener...");
        match self.start_websocket(shutdown_manager.subscribe()).await {
            Ok(ws_task) => {
                shutdown_manager.add_handle(ws_task);
            }
            Err(err) => {
                log::error!("Failed to start websocket: {}", err);
                return Err(err);
            }
        }
        Ok(())
    }

    async fn start_websocket(&mut self, mut shutdown: Shutdown) -> anyhow::Result<JoinHandle<()>> {
        let (mut websocket, ws_message_broadcast) =
            WsManager::new(self.node_info.ws_address_ip_port());
        self.ws_message_broadcast = Some(ws_message_broadcast);

        websocket.listen().await?;

        let join_handle = tokio::spawn(async move {
            tokio::select! {
                _ = shutdown.shutdown_signal_rcv.recv() => {
                    log::info!("Shutting down websocket manager");
                }
                ws_stopped = websocket.run() => {
                    match ws_stopped {
                        Ok(_) => log::info!("Websocket stopped unexpectedly"),
                        Err(e) => log::error!("Websocket stopped with error: {}", e),
                    }
                }
            }
            log::info!("Websocket task finished");
        });

        Ok(join_handle)
    }

    fn start_http(&mut self, mut shutdown: Shutdown) -> anyhow::Result<JoinHandle<()>> {
        let http = http::init(&self.node_info, self.api.clone())?;

        let join_handle = tokio::spawn(async move {
            let server_handle = http.handle();

            tokio::select! {
                _ = shutdown.shutdown_signal_rcv.recv() => {
                    log::info!("Shutting down http server");
                    server_handle.stop(true).await;
                }
                http_stopped = http => {
                    match http_stopped {
                        Ok(_) => log::info!("Http server stopped unexpectedly"),
                        Err(e) => log::error!("Http server stopped with error: {}", e),
                        //http_shutdown.notify_error()
                    }
                }
            }
            log::info!("Http task finished");
        });
        Ok(join_handle)
    }

    fn start_network(&mut self, mut shutdown: Shutdown) -> anyhow::Result<JoinHandle<()>> {
        log::info!("Starting network...{:?}", self.peer_discovery.is_some());
        let (mut network, from_network, to_network) =
            SwarmNetwork::new(self.node_info.clone(), self.peer_discovery.take().unwrap());

        self.from_network = Some(from_network);
        self.to_network = Some(to_network);

        network.listen()?;
        let join_handle = tokio::spawn(async move {
            tokio::select! {
                _ = shutdown.shutdown_signal_rcv.recv() => {
                    log::info!("Shutting down network");
                }
                nw_stopped = network.start() => {
                    match nw_stopped {
                        Ok(_) => log::info!("Network stopped unexpectedly"),
                        Err(e) => log::error!("Network stopped with error: {e}",),
                    }
                }
            }
            log::info!("Network task finished");
        });
        Ok(join_handle)
    }

    fn ephemera(
        self,
        application: A,
        shutdown_handle: ShutdownHandle,
        shutdown_manager: ShutdownManager,
    ) -> Ephemera<A> {
        let ephemera_handle = EphemeraHandle {
            api: self.api,
            shutdown: shutdown_handle,
        };
        //TODO: builder pattern should make (statically) sure that all unwraps are satisfied
        Ephemera {
            node_info: self.node_info,
            block_manager: self.block_manager.unwrap(),
            broadcaster: self.broadcaster,
            from_network: self.from_network.unwrap(),
            to_network: self.to_network.unwrap(),
            topology: BroadcastTopology::new(),
            storage: Arc::new(Mutex::new(self.storage.unwrap())),
            ws_message_broadcast: self.ws_message_broadcast.unwrap(),
            api_listener: self.api_listener,
            api_cmd_processor: ApiCmdProcessor::new(),
            application: application.into(),
            ephemera_handle,
            shutdown_manager: Some(shutdown_manager),
        }
    }

    //allocate database connection
    #[cfg(feature = "rocksdb_storage")]
    async fn connect_rocksdb(mut self) -> anyhow::Result<Self> {
        log::info!("Opening database...");
        let database = RocksDbStorage::open(self.config.storage.clone())?;
        self.storage = Some(Box::new(database));
        Ok(self)
    }

    #[cfg(feature = "sqlite_storage")]
    async fn connect_sqlite(mut self) -> anyhow::Result<Self> {
        log::info!("Opening database...");
        let database = SqliteStorage::open(self.config.storage.clone())?;
        self.storage = Some(Box::new(database));
        Ok(self)
    }

    async fn init_block_manager(mut self) -> anyhow::Result<Self> {
        let db = self.storage.take();
        if db.is_none() {
            anyhow::bail!("Database connection is not initialized")
        }
        let mut db = db.unwrap();
        let bm_builder = self.block_manager_builder.take();
        match bm_builder {
            None => {
                anyhow::bail!("Block manager builder is not initialized")
            }
            Some(bm_builder) => {
                let block_manager = bm_builder.build(db.deref_mut())?;
                self.block_manager = Some(block_manager)
            }
        }
        self.storage = Some(db);
        Ok(self)
    }
}
