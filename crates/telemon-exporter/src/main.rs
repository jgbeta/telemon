use clap::{CommandFactory, Parser};
use telemon_exporter::cli::ExporterCli;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = ExporterCli::parse();

    match cli.command {
        Some(command) => telemon_exporter::handle_command(command).await,
        None => {
            println!("No command selected yet. Use --help to see available options.");
            ExporterCli::command().print_help()?;
            println!();
            Ok(())
        }
    }
}
