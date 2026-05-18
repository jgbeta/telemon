use anyhow::bail;

use crate::cli::ServiceCommand;

pub async fn handle(command: ServiceCommand) -> anyhow::Result<()> {
    let name = command_name(&command);
    match command {
        ServiceCommand::Install(args) | ServiceCommand::Run(args) => {
            bail!(
                "service command is unsupported on this OS in Phase 3: {name} with config {}",
                args.config.display()
            )
        }
        ServiceCommand::Uninstall
        | ServiceCommand::Start
        | ServiceCommand::Stop
        | ServiceCommand::Status => {
            bail!("service command is unsupported on this OS in Phase 3: {name}")
        }
    }
}

fn command_name(command: &ServiceCommand) -> &'static str {
    match command {
        ServiceCommand::Install(_) => "install",
        ServiceCommand::Uninstall => "uninstall",
        ServiceCommand::Start => "start",
        ServiceCommand::Stop => "stop",
        ServiceCommand::Status => "status",
        ServiceCommand::Run(_) => "run",
    }
}
