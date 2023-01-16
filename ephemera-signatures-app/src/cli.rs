use clap::Parser;

#[derive(Parser, Debug, Clone)]
#[command()]
pub struct Args {
    #[arg(long)]
    pub config_file: String,
    #[arg(long)]
    pub client_listener_address: String,
    #[arg(long)]
    pub signatures_file: String,
    #[arg(long)]
    pub ws_listen_addr: String,
    #[arg(long)]
    pub db_url: String,
}

pub fn parse_args() -> Args {
    Args::parse()
}
