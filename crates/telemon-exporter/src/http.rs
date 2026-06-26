use std::sync::Arc;

use anyhow::Result;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{watch, Semaphore};
use tokio::time::{timeout, Duration};
use tracing::{debug, info};

use crate::cache::{MetricCacheMetadata, SharedMetricCache};
use crate::fps;
use crate::macmon_json;
use crate::runtime_diagnostics::ExporterDiagnostics;
use telemon_core::config::AppConfig;
use telemon_core::metrics::encode;

const MAX_CONCURRENT_CONNECTIONS: usize = 128;
const REQUEST_READ_TIMEOUT_SECONDS: u64 = 5;

#[derive(Clone)]
struct HttpState {
    dynamic_cache: SharedMetricCache,
    static_cache: SharedMetricCache,
    game_cache: Option<SharedMetricCache>,
    diagnostics: ExporterDiagnostics,
    metrics_path: String,
    static_metrics_path: String,
    fps_metrics_path: String,
    fps_debug_metrics_path: String,
    stale_after_seconds: u64,
    dynamic_scrape_gap_threshold_seconds: u64,
    static_scrape_gap_threshold_seconds: u64,
}

pub async fn serve(
    config: &AppConfig,
    dynamic_cache: SharedMetricCache,
    static_cache: SharedMetricCache,
    game_cache: Option<SharedMetricCache>,
    diagnostics: ExporterDiagnostics,
    mut shutdown: watch::Receiver<bool>,
) -> Result<()> {
    let addr = config.listen_addr()?;
    let listener = TcpListener::bind(addr).await?;
    let connection_permits = Arc::new(Semaphore::new(MAX_CONCURRENT_CONNECTIONS));
    let state = Arc::new(HttpState {
        dynamic_cache,
        static_cache,
        game_cache,
        diagnostics,
        metrics_path: config.server.metrics_path.clone(),
        static_metrics_path: config.server.static_metrics_path.clone(),
        fps_metrics_path: config.server.fps_metrics_path.clone(),
        fps_debug_metrics_path: fps::debug_metrics_path(&config.server.fps_metrics_path),
        stale_after_seconds: config.collection.scrape_cache_stale_after_seconds,
        dynamic_scrape_gap_threshold_seconds: config.diagnostics.scrape_gap_threshold_seconds,
        static_scrape_gap_threshold_seconds: config.diagnostics.scrape_gap_threshold_seconds.max(
            config
                .collection
                .static_info_interval_seconds
                .saturating_mul(2),
        ),
    });

    info!(listen = %addr, "http server listening");

    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                let Ok(permit) = Arc::clone(&connection_permits).try_acquire_owned() else {
                    tokio::spawn(async move {
                        let mut stream = stream;
                        let _ = write_response(
                            &mut stream,
                            503,
                            "text/plain; charset=utf-8",
                            "server busy\n",
                        )
                        .await;
                    });
                    continue;
                };
                let state = Arc::clone(&state);
                tokio::spawn(async move {
                    let _permit = permit;
                    if let Err(error) = handle_connection(stream, state).await {
                        debug!(%error, "http request failed");
                    }
                });
            }
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    break;
                }
            }
        }
    }

    Ok(())
}

async fn handle_connection(mut stream: TcpStream, state: Arc<HttpState>) -> Result<()> {
    let mut buffer = [0_u8; 4096];
    let bytes_read = match timeout(
        Duration::from_secs(REQUEST_READ_TIMEOUT_SECONDS),
        stream.read(&mut buffer),
    )
    .await
    {
        Ok(result) => result?,
        Err(_) => {
            write_response(
                &mut stream,
                408,
                "text/plain; charset=utf-8",
                "request timeout\n",
            )
            .await?;
            return Ok(());
        }
    };
    let request = String::from_utf8_lossy(&buffer[..bytes_read]);
    let Some(request_line) = request.lines().next() else {
        write_response(
            &mut stream,
            400,
            "text/plain; charset=utf-8",
            "bad request\n",
        )
        .await?;
        return Ok(());
    };

    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let path = parts.next().unwrap_or_default();

    if method != "GET" {
        write_response(
            &mut stream,
            405,
            "text/plain; charset=utf-8",
            "method not allowed\n",
        )
        .await?;
        return Ok(());
    }

    if path == state.metrics_path {
        state.diagnostics.record_scrape(
            &state.metrics_path,
            200,
            state.dynamic_scrape_gap_threshold_seconds,
        );
        let (mut samples, dynamic_metadata) = snapshot_and_metadata(&state.dynamic_cache);
        let (_, static_metadata) = snapshot_and_metadata(&state.static_cache);
        samples.extend(state.diagnostics.metrics(dynamic_metadata, static_metadata));
        let metrics = encode::encode(&samples);
        write_response(
            &mut stream,
            200,
            "text/plain; version=0.0.4; charset=utf-8",
            &metrics,
        )
        .await?;
    } else if path == state.static_metrics_path {
        state.diagnostics.record_scrape(
            &state.static_metrics_path,
            200,
            state.static_scrape_gap_threshold_seconds,
        );
        let (_, dynamic_metadata) = snapshot_and_metadata(&state.dynamic_cache);
        let (mut samples, static_metadata) = snapshot_and_metadata(&state.static_cache);
        samples.extend(state.diagnostics.metrics(dynamic_metadata, static_metadata));
        let metrics = encode::encode(&samples);
        write_response(
            &mut stream,
            200,
            "text/plain; version=0.0.4; charset=utf-8",
            &metrics,
        )
        .await?;
    } else if path == state.fps_metrics_path {
        state.diagnostics.record_scrape(
            &state.fps_metrics_path,
            200,
            state.dynamic_scrape_gap_threshold_seconds,
        );
        let samples = state
            .game_cache
            .as_ref()
            .and_then(|cache| cache.read().ok().map(|cache| cache.snapshot()))
            .unwrap_or_else(fps::disabled_metrics);
        let metrics = encode::encode(&fps::clean_metrics(samples));
        write_response(
            &mut stream,
            200,
            "text/plain; version=0.0.4; charset=utf-8",
            &metrics,
        )
        .await?;
    } else if path == state.fps_debug_metrics_path {
        state.diagnostics.record_scrape(
            &state.fps_debug_metrics_path,
            200,
            state.dynamic_scrape_gap_threshold_seconds,
        );
        let mut samples = state
            .game_cache
            .as_ref()
            .and_then(|cache| cache.read().ok().map(|cache| cache.snapshot()))
            .unwrap_or_else(fps::disabled_metrics);
        let (_, dynamic_metadata) = snapshot_and_metadata(&state.dynamic_cache);
        let (_, static_metadata) = snapshot_and_metadata(&state.static_cache);
        samples.extend(state.diagnostics.metrics(dynamic_metadata, static_metadata));
        let metrics = encode::encode(&samples);
        write_response(
            &mut stream,
            200,
            "text/plain; version=0.0.4; charset=utf-8",
            &metrics,
        )
        .await?;
    } else if path == "/json" {
        let dynamic_samples = state
            .dynamic_cache
            .read()
            .map(|cache| cache.snapshot())
            .unwrap_or_default();
        let static_samples = state
            .static_cache
            .read()
            .map(|cache| cache.snapshot())
            .unwrap_or_default();
        let body = macmon_json::encode_snapshot(&dynamic_samples, &static_samples);
        write_response(&mut stream, 200, "application/json; charset=utf-8", &body).await?;
    } else if path == "/healthz" {
        write_response(&mut stream, 200, "text/plain; charset=utf-8", "ok\n").await?;
    } else if path == "/readyz" {
        let ready = state
            .dynamic_cache
            .read()
            .map(|cache| cache.has_snapshot() && !cache.is_stale(state.stale_after_seconds))
            .unwrap_or(false);
        if ready {
            write_response(&mut stream, 200, "text/plain; charset=utf-8", "ready\n").await?;
        } else {
            write_response(&mut stream, 503, "text/plain; charset=utf-8", "not ready\n").await?;
        }
    } else if path == "/" {
        write_response(
            &mut stream,
            200,
            "text/plain; charset=utf-8",
            "telemon-exporter\nendpoints: /metrics /metrics/static /fps /fps/debug /json /healthz /readyz\n",
        )
        .await?;
    } else {
        write_response(&mut stream, 404, "text/plain; charset=utf-8", "not found\n").await?;
    }

    Ok(())
}

fn snapshot_and_metadata(
    cache: &SharedMetricCache,
) -> (
    Vec<telemon_core::metrics::model::MetricSample>,
    MetricCacheMetadata,
) {
    cache
        .read()
        .map(|cache| (cache.snapshot(), cache.metadata()))
        .unwrap_or_default()
}

async fn write_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &str,
) -> Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        503 => "Service Unavailable",
        _ => "OK",
    };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );

    stream.write_all(response.as_bytes()).await?;
    Ok(())
}
