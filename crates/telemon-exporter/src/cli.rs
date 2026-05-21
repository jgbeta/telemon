use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "telemon-exporter",
    version,
    about = "Native Prometheus exporter for Telemon hardware telemetry"
)]
pub struct ExporterCli {
    #[command(subcommand)]
    pub command: Option<ExporterCommand>,
}

#[derive(Debug, Subcommand)]
pub enum ExporterCommand {
    /// Start the exporter.
    Run(ConfigArgs),
    /// Validate configuration.
    Check(ConfigArgs),
    /// Print the parsed configuration.
    PrintConfig(ConfigArgs),
    /// Print one metrics snapshot.
    PrintMetrics(ConfigArgs),
    /// Discover available collectors.
    Discover(ConfigArgs),
    /// Print local hardware discovery details as JSON.
    InspectHardware(InspectHardwareArgs),
    /// Manage the native OS service.
    #[command(subcommand)]
    Service(ServiceCommand),
}

#[derive(Debug, Subcommand)]
pub enum ServiceCommand {
    /// Install the native OS service.
    Install(ConfigArgs),
    /// Uninstall the native OS service.
    Uninstall,
    /// Start the native OS service.
    Start,
    /// Stop the native OS service.
    Stop,
    /// Show native OS service status.
    Status,
    /// Run under the native service manager.
    Run(ConfigArgs),
}

#[derive(Debug, Args)]
pub struct ConfigArgs {
    #[arg(long, value_name = "PATH")]
    pub config: PathBuf,
}

#[derive(Debug, Args)]
pub struct InspectHardwareArgs {
    #[arg(long, value_name = "PATH")]
    pub config: PathBuf,
    #[arg(long, value_name = "FORMAT", default_value = "json")]
    pub format: String,
}
