use clap::{CommandFactory, Parser};
use telemon_registry::cli::RegistryCli;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = RegistryCli::parse();

    match cli.command {
        Some(command) => telemon_registry::handle_command(command).await,
        None => {
            println!("No command selected yet. Use --help to see available options.");
            RegistryCli::command().print_help()?;
            println!();
            Ok(())
        }
    }
}
