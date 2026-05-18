use clap::{CommandFactory, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "telemon",
    version,
    about = "Human-facing CLI for Telemon services"
)]
struct TelemonCli {
    #[command(subcommand)]
    command: Option<TelemonCommand>,
}

#[derive(Debug, Subcommand)]
enum TelemonCommand {
    /// Run or manage the Telemon exporter.
    #[command(subcommand)]
    Exporter(telemon_exporter::cli::ExporterCommand),
    /// Run the Telemon registry and Prometheus service-discovery server.
    #[command(subcommand)]
    Registry(telemon_registry::cli::RegistryCommand),
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = TelemonCli::parse();

    match cli.command {
        Some(TelemonCommand::Exporter(command)) => telemon_exporter::handle_command(command).await,
        Some(TelemonCommand::Registry(command)) => telemon_registry::handle_command(command).await,
        None => {
            println!("No command selected yet. Use --help to see available options.");
            TelemonCli::command().print_help()?;
            println!();
            Ok(())
        }
    }
}
