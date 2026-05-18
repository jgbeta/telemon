use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "telemon-registry",
    version,
    about = "Telemon device registry and Prometheus service discovery"
)]
pub struct RegistryCli {
    #[command(subcommand)]
    pub command: Option<RegistryCommand>,
}

#[derive(Debug, Subcommand)]
pub enum RegistryCommand {
    /// Start the device registry and Prometheus discovery server.
    Run(ConfigArgs),
}

#[derive(Debug, Args)]
pub struct ConfigArgs {
    #[arg(long, value_name = "PATH")]
    pub config: PathBuf,
}
