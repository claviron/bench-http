//! Shared CLI and production-mode setup; registration remains in each binary.

use std::path::PathBuf;

use clap::Parser;
use claviron::{AppState, FleetName, Server, ServerName, ServerOptions, registry};

pub const HOST: &str = "default";

#[derive(Parser)]
pub struct Args {
    #[arg(long, default_value = "config")]
    config_dir: PathBuf,
    #[arg(long, default_value = "bench-http")]
    name: ServerName,
    #[arg(long, default_value = "bhttp")]
    fleet: FleetName,
    /// Worker processes; 0 runs standalone.
    #[cfg(feature = "supervisor")]
    #[arg(long, default_value_t = 0)]
    workers: u8,
}

impl From<Args> for ServerOptions {
    fn from(args: Args) -> Self {
        let options = Self::new().config_dir(args.config_dir)
            .name(args.name).fleet_name(args.fleet).production();
        #[cfg(feature = "supervisor")]
        let options = options.workers(args.workers);
        options
    }
}

pub async fn serve(register: fn(&AppState) -> registry::Result<()>) -> claviron::Result<()> {
    Server::with_options(Args::parse().into()).register(register).serve().await
}
