use std::collections::BTreeMap;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tokio::sync::watch;
use tracing::{debug, warn};

use crate::cache::SharedMetricCache;
use telemon_collectors::game::live_window::RollingFrameWindow;
#[cfg(target_os = "linux")]
use telemon_collectors::game::mangoapp::MangoAppFrameReader;
use telemon_collectors::game::steam::SteamGameTitleResolver;
use telemon_collectors::linux::gamescope::{DeckGameState, GamescopeDetector};
use telemon_core::config::{AppConfig, SteamDeckFpsConfig, SteamDeckGameStateConfig};
use telemon_core::metrics::model::{MetricKind, MetricSample};
use telemon_core::metrics::names;

const GAMESCOPE_STATE_SOURCE: &str = "gamescope";
const GAMESCOPE_FRAME_SOURCE: &str = "gamescope_mangoapp";
const FRAME_SOURCE_UP_STALE_AFTER: Duration = Duration::from_secs(2);

pub fn disabled_metrics() -> Vec<MetricSample> {
    let source_labels = labels(&[("source", GAMESCOPE_FRAME_SOURCE)]);
    vec![
        MetricSample::gauge(
            names::GAME_ACTIVE,
            "Whether a game session is active.",
            labels(&[("source", GAMESCOPE_STATE_SOURCE)]),
            0.0,
        ),
        MetricSample::gauge(
            names::GAME_FOCUSED,
            "Whether the active game is focused and visible.",
            labels(&[("source", GAMESCOPE_STATE_SOURCE)]),
            0.0,
        ),
        MetricSample::gauge(
            names::GAME_FRAME_SOURCE_SUPPORTED,
            "Whether a game frame timing source is available.",
            source_labels.clone(),
            0.0,
        ),
        MetricSample::gauge(
            names::GAME_FRAME_SOURCE_UP,
            "Whether the game frame timing source is currently healthy.",
            source_labels,
            0.0,
        ),
    ]
}

pub async fn run_game_fps(
    config: AppConfig,
    cache: SharedMetricCache,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut runtime = GameFpsRuntime::new(
        config.collectors.steam_deck_fps.clone(),
        config.collectors.steam_deck_game_state.clone(),
    );
    runtime.publish(&cache);

    loop {
        let sleep = tokio::time::sleep(Duration::from_millis(
            runtime.config.poll_interval_milliseconds,
        ));
        tokio::pin!(sleep);

        tokio::select! {
            _ = &mut sleep => {
                runtime.poll_once();
                runtime.publish(&cache);
            }
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    break;
                }
            }
        }
    }
}

#[derive(Debug)]
struct WindowState {
    label: String,
    window: RollingFrameWindow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FrameInput {
    visible_frametime_ns: u64,
}

struct GameFpsRuntime {
    config: SteamDeckFpsConfig,
    detector: Option<GamescopeDetector>,
    title_resolver: SteamGameTitleResolver,
    windows: Vec<WindowState>,
    current_app_id: Option<u32>,
    current_game_name: Option<String>,
    last_state: DeckGameState,
    source_supported: bool,
    source_up: bool,
    samples_total: u64,
    dropped_zero_total: u64,
    dropped_too_large_total: u64,
    dropped_invalid_sentinel_total: u64,
    dropped_unsupported_version_total: u64,
    dropped_too_short_total: u64,
    last_sample_timestamp_seconds: Option<u64>,
    last_sample_instant: Option<Instant>,
    start: Instant,
    last_open_attempt: Option<Instant>,
    #[cfg(target_os = "linux")]
    reader: Option<MangoAppFrameReader>,
}

impl GameFpsRuntime {
    fn new(config: SteamDeckFpsConfig, game_state_config: SteamDeckGameStateConfig) -> Self {
        let mut windows_seconds = config.windows_seconds.clone();
        windows_seconds.sort_unstable();
        windows_seconds.dedup();
        let windows = windows_seconds
            .into_iter()
            .map(|seconds| WindowState {
                label: format!("{seconds}s"),
                window: RollingFrameWindow::new(seconds.saturating_mul(1_000_000_000)),
            })
            .collect();
        let detector = game_state_config
            .enabled
            .then(|| GamescopeDetector::new(game_state_config));

        Self {
            title_resolver: SteamGameTitleResolver::new(&config.steam_library_roots),
            config,
            detector,
            windows,
            current_app_id: None,
            current_game_name: None,
            last_state: DeckGameState::Idle,
            source_supported: false,
            source_up: false,
            samples_total: 0,
            dropped_zero_total: 0,
            dropped_too_large_total: 0,
            dropped_invalid_sentinel_total: 0,
            dropped_unsupported_version_total: 0,
            dropped_too_short_total: 0,
            last_sample_timestamp_seconds: None,
            last_sample_instant: None,
            start: Instant::now(),
            last_open_attempt: None,
            #[cfg(target_os = "linux")]
            reader: None,
        }
    }

    fn poll_once(&mut self) {
        let detection = self.detector.as_ref().map(|detector| detector.detect());
        self.last_state = detection
            .as_ref()
            .map(|detection| detection.state)
            .unwrap_or(DeckGameState::Idle);
        let active_app_id = detection
            .as_ref()
            .and_then(|detection| detection.snapshot.active_game_app_id);

        if !self.last_state.is_game_running() {
            self.set_current_game(None);
            self.clear_windows();
            self.try_open_reader();
            self.source_up = false;
            return;
        }

        self.set_current_game(active_app_id);
        self.try_open_reader();
        let frames = self.read_frames();
        if frames.is_empty() {
            return;
        }

        let now_ns = self.monotonic_nanos();
        for frame in frames {
            for window in &mut self.windows {
                window
                    .window
                    .push_frame_time(now_ns, frame.visible_frametime_ns);
            }
        }
    }

    fn set_current_game(&mut self, app_id: Option<u32>) {
        if self.current_app_id == app_id {
            return;
        }
        self.current_app_id = app_id;
        self.current_game_name = app_id.and_then(|id| self.title_resolver.resolve_name(id));
        self.clear_windows();
    }

    fn clear_windows(&mut self) {
        for window in &mut self.windows {
            window.window.clear();
        }
    }

    fn publish(&self, cache: &SharedMetricCache) {
        if let Ok(mut cache) = cache.write() {
            cache.replace_snapshot(self.metrics());
        }
    }

    fn metrics(&self) -> Vec<MetricSample> {
        let mut metrics = Vec::new();
        let active = self.last_state.is_game_running();
        let focused = self.last_state == DeckGameState::GameFocusedVisible;
        let state_labels = self.labels(GAMESCOPE_STATE_SOURCE, None);

        metrics.push(MetricSample::gauge(
            names::GAME_ACTIVE,
            "Whether a game session is active.",
            state_labels.clone(),
            if active { 1.0 } else { 0.0 },
        ));
        metrics.push(MetricSample::gauge(
            names::GAME_FOCUSED,
            "Whether the active game is focused and visible.",
            state_labels,
            if focused { 1.0 } else { 0.0 },
        ));

        if let Some(app_id) = self.current_app_id {
            let mut labels = BTreeMap::new();
            labels.insert("appid".to_string(), app_id.to_string());
            labels.insert("source".to_string(), "steam_appmanifest".to_string());
            if let Some(game_name) = &self.current_game_name {
                labels.insert("game_name".to_string(), game_name.clone());
            }
            metrics.push(MetricSample::gauge(
                names::GAME_IDENTITY_INFO,
                "Game identity resolved from local Steam metadata.",
                labels,
                1.0,
            ));
        }

        let source_labels = labels(&[("source", GAMESCOPE_FRAME_SOURCE)]);
        metrics.push(MetricSample::gauge(
            names::GAME_FRAME_SOURCE_SUPPORTED,
            "Whether a game frame timing source is available.",
            source_labels.clone(),
            if self.source_supported { 1.0 } else { 0.0 },
        ));
        metrics.push(MetricSample::gauge(
            names::GAME_FRAME_SOURCE_UP,
            "Whether the game frame timing source is currently healthy.",
            source_labels.clone(),
            if self.source_up { 1.0 } else { 0.0 },
        ));
        metrics.push(MetricSample::counter(
            names::GAME_FRAME_SOURCE_SAMPLES_TOTAL,
            "Total accepted frame timing samples read from the game frame source.",
            source_labels.clone(),
            self.samples_total as f64,
        ));
        metrics.push(MetricSample::counter(
            names::GAME_FRAME_SOURCE_DROPPED_TOTAL,
            "Total frame timing samples dropped by sanity filters.",
            labels(&[("source", GAMESCOPE_FRAME_SOURCE), ("reason", "zero")]),
            self.dropped_zero_total as f64,
        ));
        metrics.push(MetricSample::counter(
            names::GAME_FRAME_SOURCE_DROPPED_TOTAL,
            "Total frame timing samples dropped by sanity filters.",
            labels(&[("source", GAMESCOPE_FRAME_SOURCE), ("reason", "too_large")]),
            self.dropped_too_large_total as f64,
        ));
        metrics.push(MetricSample::counter(
            names::GAME_FRAME_SOURCE_DROPPED_TOTAL,
            "Total frame timing samples dropped by sanity filters.",
            labels(&[
                ("source", GAMESCOPE_FRAME_SOURCE),
                ("reason", "invalid_sentinel"),
            ]),
            self.dropped_invalid_sentinel_total as f64,
        ));
        metrics.push(MetricSample::counter(
            names::GAME_FRAME_SOURCE_DROPPED_TOTAL,
            "Total frame timing samples dropped by sanity filters.",
            labels(&[
                ("source", GAMESCOPE_FRAME_SOURCE),
                ("reason", "unsupported_version"),
            ]),
            self.dropped_unsupported_version_total as f64,
        ));
        metrics.push(MetricSample::counter(
            names::GAME_FRAME_SOURCE_DROPPED_TOTAL,
            "Total frame timing samples dropped by sanity filters.",
            labels(&[("source", GAMESCOPE_FRAME_SOURCE), ("reason", "too_short")]),
            self.dropped_too_short_total as f64,
        ));
        if let Some(timestamp) = self.last_sample_timestamp_seconds {
            metrics.push(MetricSample::gauge(
                names::GAME_FRAME_SOURCE_LAST_SAMPLE_TIMESTAMP_SECONDS,
                "Unix timestamp of the last accepted game frame timing sample.",
                source_labels,
                timestamp as f64,
            ));
        }

        if active {
            for window in &self.windows {
                if let Some(summary) = window.window.summary() {
                    let base_labels = self.labels(GAMESCOPE_FRAME_SOURCE, Some(&window.label));
                    metrics.extend(summary_metrics(base_labels, &summary));
                }
            }
        }

        metrics
    }

    fn labels(&self, source: &str, window: Option<&str>) -> BTreeMap<String, String> {
        let mut labels = BTreeMap::new();
        labels.insert("source".to_string(), source.to_string());
        if let Some(window) = window {
            labels.insert("window".to_string(), window.to_string());
        }
        if self.config.include_appid_label {
            if let Some(app_id) = self.current_app_id {
                labels.insert("appid".to_string(), app_id.to_string());
            }
        }
        if self.config.include_game_name_label {
            if let Some(game_name) = &self.current_game_name {
                labels.insert("game_name".to_string(), game_name.clone());
            }
        }
        labels
    }

    fn monotonic_nanos(&self) -> u64 {
        self.start
            .elapsed()
            .as_nanos()
            .try_into()
            .unwrap_or(u64::MAX)
    }

    fn try_open_reader(&mut self) {
        #[cfg(target_os = "linux")]
        {
            if !self.config.gamescope_mangoapp.enabled {
                self.source_supported = false;
                self.source_up = false;
                return;
            }

            let now = Instant::now();
            if self.reader.is_some() {
                self.source_supported = true;
                self.refresh_source_up(now, self.last_state.is_game_running());
                return;
            }

            if self
                .last_open_attempt
                .is_some_and(|last| now.duration_since(last) < Duration::from_secs(1))
            {
                return;
            }
            self.last_open_attempt = Some(now);
            let max_frame_time_ns = self
                .config
                .max_frame_time_milliseconds
                .saturating_mul(1_000_000);
            match MangoAppFrameReader::open(&self.config.gamescope_mangoapp, max_frame_time_ns) {
                Ok(reader) => {
                    self.reader = Some(reader);
                    self.source_supported = true;
                    self.refresh_source_up(now, self.last_state.is_game_running());
                    debug!(source = GAMESCOPE_FRAME_SOURCE, "game frame source opened");
                }
                Err(error) => {
                    self.source_supported = false;
                    self.source_up = false;
                    debug!(%error, source = GAMESCOPE_FRAME_SOURCE, "game frame source unavailable");
                }
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            self.source_supported = false;
            self.source_up = false;
        }
    }

    fn refresh_source_up(&mut self, now: Instant, active: bool) {
        self.source_up = active
            && self
                .last_sample_instant
                .is_some_and(|last| now.duration_since(last) <= FRAME_SOURCE_UP_STALE_AFTER);
    }

    fn read_frames(&mut self) -> Vec<FrameInput> {
        #[cfg(target_os = "linux")]
        {
            let Some(reader) = &self.reader else {
                self.source_up = false;
                return Vec::new();
            };
            let mut samples = Vec::new();
            let max_messages = self
                .config
                .max_messages_per_poll
                .try_into()
                .unwrap_or(usize::MAX);
            match reader.read_available(max_messages, &mut samples) {
                Ok(result) => {
                    self.source_supported = true;
                    self.samples_total = self
                        .samples_total
                        .saturating_add(result.samples_read as u64);
                    self.dropped_zero_total =
                        self.dropped_zero_total.saturating_add(result.dropped_zero);
                    self.dropped_too_large_total = self
                        .dropped_too_large_total
                        .saturating_add(result.dropped_too_large);
                    self.dropped_invalid_sentinel_total = self
                        .dropped_invalid_sentinel_total
                        .saturating_add(result.dropped_invalid_sentinel);
                    self.dropped_unsupported_version_total = self
                        .dropped_unsupported_version_total
                        .saturating_add(result.dropped_unsupported_version);
                    self.dropped_too_short_total = self
                        .dropped_too_short_total
                        .saturating_add(result.dropped_too_short);
                    if result.samples_read > 0 {
                        self.last_sample_timestamp_seconds = Some(unix_timestamp_seconds());
                        self.last_sample_instant = Some(Instant::now());
                    }
                    self.refresh_source_up(Instant::now(), self.last_state.is_game_running());
                    samples
                        .into_iter()
                        .map(|sample| FrameInput {
                            visible_frametime_ns: sample.visible_frametime_ns,
                        })
                        .collect()
                }
                Err(error) => {
                    warn!(%error, source = GAMESCOPE_FRAME_SOURCE, "game frame source read failed");
                    self.reader = None;
                    self.source_supported = false;
                    self.source_up = false;
                    Vec::new()
                }
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            Vec::new()
        }
    }
}

fn summary_metrics(
    base_labels: BTreeMap<String, String>,
    summary: &telemon_collectors::game::capture::CaptureSummary,
) -> Vec<MetricSample> {
    let mut metrics = Vec::new();
    metrics.push(MetricSample::gauge(
        names::GAME_FRAME_COUNT,
        "Game frames observed in a rolling window.",
        base_labels.clone(),
        summary.frame_count as f64,
    ));

    push_game_metric(
        &mut metrics,
        names::GAME_FRAME_RATE_FPS,
        "Game frame rate in frames per second.",
        base_labels.clone(),
        &[("stat", "average"), ("method", "total_time")],
        summary.average_fps,
    );
    push_game_metric(
        &mut metrics,
        names::GAME_FRAME_RATE_FPS,
        "Game frame rate in frames per second.",
        base_labels.clone(),
        &[("stat", "low_1pct"), ("method", "threshold")],
        summary.low_1pct_fps_threshold,
    );
    push_game_metric(
        &mut metrics,
        names::GAME_FRAME_RATE_FPS,
        "Game frame rate in frames per second.",
        base_labels.clone(),
        &[("stat", "low_1pct"), ("method", "tail_average")],
        summary.low_1pct_fps_average,
    );
    push_game_metric(
        &mut metrics,
        names::GAME_FRAME_RATE_FPS,
        "Game frame rate in frames per second.",
        base_labels.clone(),
        &[("stat", "low_0_1pct"), ("method", "threshold")],
        summary.low_01pct_fps_threshold,
    );
    push_game_metric(
        &mut metrics,
        names::GAME_FRAME_RATE_FPS,
        "Game frame rate in frames per second.",
        base_labels.clone(),
        &[("stat", "low_0_1pct"), ("method", "tail_average")],
        summary.low_01pct_fps_average,
    );
    push_game_metric(
        &mut metrics,
        names::GAME_FRAME_RATE_FPS,
        "Game frame rate in frames per second.",
        base_labels.clone(),
        &[("stat", "high_1pct"), ("method", "threshold")],
        summary.high_1pct_fps_threshold,
    );
    push_game_metric(
        &mut metrics,
        names::GAME_FRAME_RATE_FPS,
        "Game frame rate in frames per second.",
        base_labels.clone(),
        &[("stat", "high_1pct"), ("method", "tail_average")],
        summary.high_1pct_fps_average,
    );

    for (stat, value) in [
        ("average", summary.average_frame_time_seconds),
        ("min", summary.min_frame_time_seconds),
        ("max", summary.max_frame_time_seconds),
        ("p50", summary.p50_frame_time_seconds),
        ("p95", summary.p95_frame_time_seconds),
        ("p99", summary.p99_frame_time_seconds),
    ] {
        push_game_metric(
            &mut metrics,
            names::GAME_FRAMETIME_SECONDS,
            "Game frame time in seconds.",
            base_labels.clone(),
            &[("stat", stat)],
            value,
        );
    }

    if let Some(jitter) = summary.jitter {
        for (stat, value) in [
            ("average", jitter.average_seconds),
            ("p95", jitter.p95_seconds),
            ("p99", jitter.p99_seconds),
            ("max", jitter.max_seconds),
        ] {
            push_game_metric(
                &mut metrics,
                names::GAME_FRAME_PACING_JITTER_SECONDS,
                "Adjacent game frame-time delta in seconds.",
                base_labels.clone(),
                &[("stat", stat)],
                value,
            );
        }
    }

    metrics
}

fn push_game_metric(
    metrics: &mut Vec<MetricSample>,
    name: &str,
    help: &str,
    mut base_labels: BTreeMap<String, String>,
    extra_labels: &[(&str, &str)],
    value: f64,
) {
    for (key, value) in extra_labels {
        base_labels.insert((*key).to_string(), (*value).to_string());
    }
    metrics.push(MetricSample::new(name, help, MetricKind::Gauge, base_labels, value).unwrap());
}

fn labels(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
        .collect()
}

fn unix_timestamp_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use telemon_collectors::game::capture::CaptureStats;

    #[test]
    fn summary_metrics_include_appid_and_game_name_labels() {
        let summary = CaptureStats::from_frame_times_ns([16_000_000, 17_000_000, 33_000_000])
            .summary()
            .unwrap();
        let mut labels = BTreeMap::new();
        labels.insert("source".to_string(), GAMESCOPE_FRAME_SOURCE.to_string());
        labels.insert("window".to_string(), "1s".to_string());
        labels.insert("appid".to_string(), "1234".to_string());
        labels.insert("game_name".to_string(), "Example Game".to_string());

        let metrics = summary_metrics(labels, &summary);

        assert!(metrics
            .iter()
            .any(|metric| metric.name == names::GAME_FRAME_RATE_FPS));
        assert!(metrics.iter().all(|metric| metric
            .labels
            .get("appid")
            .is_some_and(|value| value == "1234")));
        assert!(metrics.iter().all(|metric| metric
            .labels
            .get("game_name")
            .is_some_and(|value| value == "Example Game")));
    }

    #[test]
    fn inactive_runtime_emits_source_health_without_frame_metrics() {
        let config = SteamDeckFpsConfig {
            enabled: true,
            ..Default::default()
        };
        let runtime = GameFpsRuntime::new(config, SteamDeckGameStateConfig::default());
        let metrics = runtime.metrics();

        assert!(metrics
            .iter()
            .any(|metric| metric.name == names::GAME_ACTIVE));
        assert!(!metrics
            .iter()
            .any(|metric| metric.name == names::GAME_FRAME_COUNT));
    }
}
