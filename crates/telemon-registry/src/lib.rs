pub mod cli;
pub mod registry;

use tracing::info;

use crate::cli::RegistryCommand;
use telemon_core::{config::RegistryAppConfig, logging};

pub async fn handle_command(command: RegistryCommand) -> anyhow::Result<()> {
    match command {
        RegistryCommand::Run(args) => {
            let config = RegistryAppConfig::load_from_path(&args.config)?;
            logging::init(&config.logging.level)?;
            info!(
                command = "registry run",
                config_path = %args.config.display(),
                "starting registry"
            );
            registry::run(config).await?;
        }
    }

    Ok(())
}
