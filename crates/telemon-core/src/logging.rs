use anyhow::Result;
use tracing_subscriber::EnvFilter;

pub fn init(config_level: &str) -> Result<()> {
    let filter = EnvFilter::try_from_default_env().or_else(|_| EnvFilter::try_new(config_level))?;

    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(false)
        .try_init();

    Ok(())
}
