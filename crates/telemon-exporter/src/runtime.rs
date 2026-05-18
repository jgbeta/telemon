use anyhow::Result;
use tokio::sync::watch;
use tracing::{info, warn};

use crate::adaptive::AdaptiveSamplingState;
use crate::cache::MetricCache;
use crate::{diagnostics, http, scheduler};
use telemon_core::config::AppConfig;

pub async fn run(config: AppConfig) -> Result<()> {
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let signal_tx = shutdown_tx.clone();
    tokio::spawn(async move {
        if let Err(error) = tokio::signal::ctrl_c().await {
            warn!(%error, "failed to listen for ctrl+c");
            return;
        }
        info!("shutdown signal received");
        let _ = signal_tx.send(true);
    });

    run_with_shutdown(config, shutdown_tx, shutdown_rx).await
}

pub async fn run_with_shutdown(
    config: AppConfig,
    shutdown_tx: watch::Sender<bool>,
    shutdown_rx: watch::Receiver<bool>,
) -> Result<()> {
    let dynamic_cache = MetricCache::shared();
    let static_cache = MetricCache::shared();
    let collectors = diagnostics::build_scheduled_collectors(&config);
    let adaptive_state = AdaptiveSamplingState::new(config.adaptive_sampling.levels.normal_seconds);

    let scheduler_handle = tokio::spawn(scheduler::run_scheduler(
        collectors,
        dynamic_cache.clone(),
        static_cache.clone(),
        config.adaptive_sampling.clone(),
        adaptive_state.clone(),
        shutdown_rx.clone(),
    ));
    let registration_handle = if config.registration.enabled {
        Some(tokio::spawn(crate::registration::run_client(
            config.clone(),
            adaptive_state.clone(),
            shutdown_rx.clone(),
        )))
    } else {
        None
    };

    let http_result = http::serve(&config, dynamic_cache, static_cache, shutdown_rx).await;
    let _ = shutdown_tx.send(true);
    let _ = scheduler_handle.await;
    if let Some(handle) = registration_handle {
        let _ = handle.await;
    }

    http_result
}
