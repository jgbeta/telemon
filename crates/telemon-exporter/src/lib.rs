pub mod adaptive;
pub mod cache;
pub mod cli;
pub mod diagnostics;
pub mod hardware_inspect;
pub mod http;
pub mod registration;
pub mod runtime;
pub mod scheduler;
pub mod service;

use tracing::info;

use crate::cli::ExporterCommand;
use telemon_core::{config::AppConfig, logging};

pub async fn handle_command(command: ExporterCommand) -> anyhow::Result<()> {
    match command {
        ExporterCommand::Run(args) => {
            let config = load_config_and_init_logging(&args.config)?;
            info!(command = "run", config_path = %args.config.display(), "starting exporter");
            runtime::run(config).await?;
        }
        ExporterCommand::Check(args) => {
            let config = load_config_and_init_logging(&args.config)?;
            info!(command = "check", config_path = %args.config.display(), "checking config");
            print!("{}", diagnostics::check_report(&config));
        }
        ExporterCommand::PrintConfig(args) => {
            let config = load_config_and_init_logging(&args.config)?;
            info!(command = "print-config", config_path = %args.config.display(), "printing config");
            print!("{}", serde_yaml::to_string(&config)?);
        }
        ExporterCommand::PrintMetrics(args) => {
            let config = load_config_and_init_logging(&args.config)?;
            info!(command = "print-metrics", config_path = %args.config.display(), "printing one metrics snapshot");
            print!("{}", diagnostics::print_metrics(&config));
        }
        ExporterCommand::Discover(args) => {
            let config = load_config_and_init_logging(&args.config)?;
            info!(command = "discover", config_path = %args.config.display(), "discovering collectors");
            print!("{}", diagnostics::discover_report(&config));
        }
        ExporterCommand::InspectHardware(args) => {
            if args.format != "json" {
                anyhow::bail!("unsupported inspect-hardware format: {}", args.format);
            }
            let config = AppConfig::load_from_path(&args.config)?;
            println!("{}", hardware_inspect::inspect_hardware_json(&config)?);
        }
        ExporterCommand::Service(command) => {
            service::handle(command).await?;
        }
    }

    Ok(())
}

fn load_config_and_init_logging(path: &std::path::Path) -> anyhow::Result<AppConfig> {
    let config = AppConfig::load_from_path(path)?;
    logging::init(&config.logging.level)?;
    Ok(config)
}
