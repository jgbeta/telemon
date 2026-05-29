//! Minimal library wrapper around the macmon 0.6.1 sampler code.
//!
//! Telemon vendors only the sampler modules needed by the exporter so the main
//! workspace does not inherit macmon CLI dependencies such as Clap or Ratatui.

#[cfg(target_os = "macos")]
pub mod metrics;
#[cfg(target_os = "macos")]
pub mod sources;

#[cfg(target_os = "macos")]
pub use metrics::{MemMetrics, Metrics, Sampler, TempMetrics};
#[cfg(target_os = "macos")]
pub use sources::SocInfo;
