use anyhow::Result;

use crate::cli::ServiceCommand;

#[cfg(not(windows))]
mod stub;
#[cfg(windows)]
mod windows;

#[cfg(not(windows))]
use stub as platform;
#[cfg(windows)]
use windows as platform;

pub async fn handle(command: ServiceCommand) -> Result<()> {
    platform::handle(command).await
}
