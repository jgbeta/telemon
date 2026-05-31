use anyhow::Result;
use tokio::sync::watch;
use tracing::{info, warn};

use crate::adaptive::AdaptiveSamplingState;
use crate::cache::MetricCache;
use crate::{diagnostics, fps, game_state, http, runtime_diagnostics, scheduler};
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
    let game_cache = config
        .collectors
        .steam_deck_fps
        .enabled
        .then(MetricCache::shared);
    let collectors = diagnostics::build_scheduled_collectors(&config);
    let adaptive_state = AdaptiveSamplingState::new(config.adaptive_sampling.levels.normal_seconds);
    let sampling_override = game_state::sampling_override(&config);
    let exporter_diagnostics =
        runtime_diagnostics::ExporterDiagnostics::new(config.diagnostics.clone());

    let scheduler_handle = tokio::spawn(scheduler::run_scheduler(
        scheduler::SchedulerRuntime {
            collectors,
            dynamic_cache: dynamic_cache.clone(),
            static_cache: static_cache.clone(),
            adaptive_config: config.adaptive_sampling.clone(),
            adaptive_state: adaptive_state.clone(),
            sampling_override,
            exporter_diagnostics: exporter_diagnostics.clone(),
        },
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
    let fps_handle = game_cache.as_ref().map(|cache| {
        tokio::spawn(fps::run_game_fps(
            config.clone(),
            cache.clone(),
            shutdown_rx.clone(),
        ))
    });

    let http_result = http::serve(
        &config,
        dynamic_cache,
        static_cache,
        game_cache,
        exporter_diagnostics,
        shutdown_rx,
    )
    .await;
    let _ = shutdown_tx.send(true);
    let _ = scheduler_handle.await;
    if let Some(handle) = registration_handle {
        let _ = handle.await;
    }
    if let Some(handle) = fps_handle {
        let _ = handle.await;
    }

    http_result
}
