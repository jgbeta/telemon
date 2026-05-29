use std::sync::Arc;

use anyhow::Result;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tracing::{debug, info};

use crate::cache::SharedMetricCache;
use crate::macmon_json;
use telemon_core::config::AppConfig;
use telemon_core::metrics::encode;

#[derive(Clone)]
struct HttpState {
    dynamic_cache: SharedMetricCache,
    static_cache: SharedMetricCache,
    metrics_path: String,
    static_metrics_path: String,
    stale_after_seconds: u64,
}

pub async fn serve(
    config: &AppConfig,
    dynamic_cache: SharedMetricCache,
    static_cache: SharedMetricCache,
    mut shutdown: watch::Receiver<bool>,
) -> Result<()> {
    let addr = config.listen_addr()?;
    let listener = TcpListener::bind(addr).await?;
    let state = Arc::new(HttpState {
        dynamic_cache,
        static_cache,
        metrics_path: config.server.metrics_path.clone(),
        static_metrics_path: config.server.static_metrics_path.clone(),
        stale_after_seconds: config.collection.scrape_cache_stale_after_seconds,
    });

    info!(listen = %addr, "http server listening");

    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                let state = Arc::clone(&state);
                tokio::spawn(async move {
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
    let bytes_read = stream.read(&mut buffer).await?;
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
        let metrics = state
            .dynamic_cache
            .read()
            .map(|cache| encode::encode(&cache.snapshot()))
            .unwrap_or_default();
        write_response(
            &mut stream,
            200,
            "text/plain; version=0.0.4; charset=utf-8",
            &metrics,
        )
        .await?;
    } else if path == state.static_metrics_path {
        let metrics = state
            .static_cache
            .read()
            .map(|cache| encode::encode(&cache.snapshot()))
            .unwrap_or_default();
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
            "telemon-exporter\nendpoints: /metrics /metrics/static /json /healthz /readyz\n",
        )
        .await?;
    } else {
        write_response(&mut stream, 404, "text/plain; charset=utf-8", "not found\n").await?;
    }

    Ok(())
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
