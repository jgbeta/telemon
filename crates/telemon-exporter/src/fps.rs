use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tokio::sync::watch;
use tracing::{debug, warn};

use crate::cache::SharedMetricCache;
#[cfg(target_os = "linux")]
use telemon_collectors::game::gamescope_wayland::GamescopeWaylandFrameReader;
use telemon_collectors::game::live_window::RollingFrameWindow;
#[cfg(target_os = "linux")]
use telemon_collectors::game::mangoapp::MangoAppFrameReader;
use telemon_collectors::game::steam::SteamGameTitleResolver;
use telemon_collectors::linux::gamescope::{DeckGameState, GamescopeDetector};
use telemon_core::config::{
    AppConfig, MangoHudLogConfig, SteamDeckFpsConfig, SteamDeckGameStateConfig,
};
use telemon_core::metrics::model::MetricSample;
use telemon_core::metrics::names;

const GAMESCOPE_STATE_SOURCE: &str = "gamescope";
const GAMESCOPE_WAYLAND_SOURCE: &str = "gamescope_wayland";
const GAMESCOPE_FRAME_SOURCE: &str = "gamescope_mangoapp";
const MANGOHUD_LOG_SOURCE: &str = "mangohud_log";
const FRAME_SOURCE_QUEUE_NOT_APPLICABLE: &str = "not_applicable";
const FRAME_SOURCE_QUEUE_UNAVAILABLE: &str = "unavailable";
const FRAME_SOURCE_QUEUE_BLOCKED: &str = "blocked_competing_consumer";
const FRAME_SOURCE_QUEUE_DESTRUCTIVE_READ_DISABLED: &str = "destructive_read_disabled";
const FRAME_SOURCE_UP_STALE_AFTER: Duration = Duration::from_secs(2);
const MANGOHUD_LOG_DISCOVERY_INTERVAL: Duration = Duration::from_secs(1);

pub fn disabled_metrics() -> Vec<MetricSample> {
    let source_labels = labels(&[("source", GAMESCOPE_WAYLAND_SOURCE)]);
    vec![
        MetricSample::gauge(
            names::GAME_SESSION_ACTIVE,
            "Whether a game session is active.",
            labels(&[("state_source", GAMESCOPE_STATE_SOURCE)]),
            0.0,
        ),
        MetricSample::gauge(
            names::GAME_SESSION_FOCUSED,
            "Whether the active game is focused and visible.",
            labels(&[("state_source", GAMESCOPE_STATE_SOURCE)]),
            0.0,
        ),
        MetricSample::gauge(
            names::GAME_FPS_SOURCE_SELECTED,
            "Whether this FPS source is currently selected.",
            source_labels.clone(),
            0.0,
        ),
        MetricSample::gauge(
            names::GAME_FPS_SOURCE_AVAILABLE,
            "Whether this FPS source is available.",
            source_labels.clone(),
            0.0,
        ),
        MetricSample::gauge(
            names::GAME_FPS_SOURCE_HEALTHY,
            "Whether this FPS source is currently healthy.",
            source_labels,
            0.0,
        ),
    ]
}

pub fn clean_metrics(samples: Vec<MetricSample>) -> Vec<MetricSample> {
    samples
        .into_iter()
        .filter(|sample| !is_debug_metric(&sample.name))
        .collect()
}

pub fn debug_metrics_path(fps_metrics_path: &str) -> String {
    format!("{}/debug", fps_metrics_path.trim_end_matches('/'))
}

fn is_debug_metric(name: &str) -> bool {
    [
        names::GAME_FPS_SOURCE_BACKEND_INFO,
        names::GAME_FPS_SOURCE_SAMPLE_PAYLOAD_BYTES,
        names::GAME_FPS_SOURCE_OUTPUT_PIXELS,
    ]
    .contains(&name)
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum FrameSourceKind {
    GamescopeWayland,
    MangoHudLog,
    GamescopeMangoapp,
}

impl FrameSourceKind {
    fn from_config_name(value: &str) -> Option<Self> {
        match value {
            GAMESCOPE_WAYLAND_SOURCE => Some(Self::GamescopeWayland),
            MANGOHUD_LOG_SOURCE => Some(Self::MangoHudLog),
            GAMESCOPE_FRAME_SOURCE => Some(Self::GamescopeMangoapp),
            _ => None,
        }
    }

    fn metric_source(self) -> &'static str {
        match self {
            Self::GamescopeWayland => GAMESCOPE_WAYLAND_SOURCE,
            Self::MangoHudLog => MANGOHUD_LOG_SOURCE,
            Self::GamescopeMangoapp => GAMESCOPE_FRAME_SOURCE,
        }
    }

    fn unavailable_queue_label(self) -> &'static str {
        match self {
            Self::GamescopeWayland => FRAME_SOURCE_QUEUE_NOT_APPLICABLE,
            Self::MangoHudLog => FRAME_SOURCE_QUEUE_NOT_APPLICABLE,
            Self::GamescopeMangoapp => FRAME_SOURCE_QUEUE_UNAVAILABLE,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct FrameSourceHealth {
    supported: bool,
    up: bool,
    queue_label: &'static str,
}

impl FrameSourceHealth {
    fn unavailable(source: FrameSourceKind) -> Self {
        Self {
            supported: false,
            up: false,
            queue_label: source.unavailable_queue_label(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MangoHudLogHeader {
    fps_index: Option<usize>,
    frametime_index: Option<usize>,
}

#[derive(Debug, Default)]
struct MangoHudLogReadResult {
    supported: bool,
    frames: Vec<FrameInput>,
    dropped_zero: u64,
    dropped_too_large: u64,
}

#[derive(Debug, Default)]
struct MangoHudLogTail {
    active_path: Option<PathBuf>,
    offset: u64,
    header: Option<MangoHudLogHeader>,
    cached_candidate: Option<PathBuf>,
    last_discovery: Option<Instant>,
}

impl MangoHudLogTail {
    fn read_available(
        &mut self,
        config: &MangoHudLogConfig,
        max_frame_time_ns: u64,
        max_rows: usize,
    ) -> io::Result<MangoHudLogReadResult> {
        let Some(path) = self.discover_candidate(config) else {
            self.active_path = None;
            self.offset = 0;
            self.header = None;
            return Ok(MangoHudLogReadResult::default());
        };

        let metadata = fs::metadata(&path)?;
        if self.active_path.as_deref() != Some(path.as_path()) || metadata.len() < self.offset {
            self.active_path = Some(path.clone());
            self.offset = 0;
            self.header = None;
            self.initialize_tail(&path)?;
            return Ok(MangoHudLogReadResult {
                supported: true,
                ..Default::default()
            });
        }

        if self.header.is_none() {
            self.initialize_tail(&path)?;
        }

        let Some(header) = self.header else {
            return Ok(MangoHudLogReadResult {
                supported: true,
                ..Default::default()
            });
        };

        if metadata.len() <= self.offset {
            return Ok(MangoHudLogReadResult {
                supported: true,
                ..Default::default()
            });
        }

        let mut file = File::open(&path)?;
        file.seek(SeekFrom::Start(self.offset))?;
        let mut appended = String::new();
        file.read_to_string(&mut appended)?;

        let complete_len = appended
            .rfind('\n')
            .map(|index| index + 1)
            .unwrap_or_default();
        self.offset = self.offset.saturating_add(complete_len as u64);
        let complete = &appended[..complete_len];

        let mut result = MangoHudLogReadResult {
            supported: true,
            ..Default::default()
        };
        for line in complete.lines().take(max_rows) {
            match parse_mangohud_log_frame(line, header, max_frame_time_ns) {
                ParsedMangoHudLogFrame::Frame(frame) => result.frames.push(frame),
                ParsedMangoHudLogFrame::Zero => {
                    result.dropped_zero = result.dropped_zero.saturating_add(1)
                }
                ParsedMangoHudLogFrame::TooLarge => {
                    result.dropped_too_large = result.dropped_too_large.saturating_add(1)
                }
                ParsedMangoHudLogFrame::Ignore => {}
            }
        }

        Ok(result)
    }

    fn discover_candidate(&mut self, config: &MangoHudLogConfig) -> Option<PathBuf> {
        let now = Instant::now();
        if self
            .last_discovery
            .is_some_and(|last| now.duration_since(last) < MANGOHUD_LOG_DISCOVERY_INTERVAL)
        {
            return self.cached_candidate.clone();
        }

        self.last_discovery = Some(now);
        self.cached_candidate = discover_mangohud_log_path(config);
        self.cached_candidate.clone()
    }

    fn initialize_tail(&mut self, path: &Path) -> io::Result<()> {
        let mut file = File::open(path)?;
        let mut content = String::new();
        file.read_to_string(&mut content)?;
        self.header = content.lines().find_map(parse_mangohud_log_header);
        self.offset = content.len() as u64;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParsedMangoHudLogFrame {
    Frame(FrameInput),
    Zero,
    TooLarge,
    Ignore,
}

struct GameFpsRuntime {
    config: SteamDeckFpsConfig,
    detector: Option<GamescopeDetector>,
    title_resolver: SteamGameTitleResolver,
    windows: Vec<WindowState>,
    current_app_id: Option<u32>,
    current_game_name: Option<String>,
    session_start_timestamp_seconds: Option<u64>,
    last_state: DeckGameState,
    source_supported: bool,
    source_up: bool,
    samples_total: u64,
    dropped_zero_total: u64,
    dropped_too_large_total: u64,
    dropped_invalid_sentinel_total: u64,
    dropped_unsupported_version_total: u64,
    dropped_too_short_total: u64,
    dropped_wrong_session_total: u64,
    last_sample_timestamp_seconds: Option<u64>,
    last_sample_instant: Option<Instant>,
    last_sample_interval: Option<Duration>,
    last_sample_payload_bytes: Option<usize>,
    last_sample_output_width: Option<u32>,
    last_sample_output_height: Option<u32>,
    frame_source: FrameSourceKind,
    frame_queue_label: &'static str,
    source_health: BTreeMap<FrameSourceKind, FrameSourceHealth>,
    mangohud_log_tail: MangoHudLogTail,
    start: Instant,
    last_wayland_open_attempt: Option<Instant>,
    last_mangoapp_open_attempt: Option<Instant>,
    #[cfg(target_os = "linux")]
    wayland_reader: Option<GamescopeWaylandFrameReader>,
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

        let frame_source = initial_frame_source(&config);
        let mut source_health = BTreeMap::new();
        for source in configured_frame_sources(&config) {
            source_health.insert(source, FrameSourceHealth::unavailable(source));
        }
        source_health
            .entry(frame_source)
            .or_insert_with(|| FrameSourceHealth::unavailable(frame_source));

        Self {
            title_resolver: SteamGameTitleResolver::new(&config.steam_library_roots),
            config,
            detector,
            windows,
            current_app_id: None,
            current_game_name: None,
            session_start_timestamp_seconds: None,
            last_state: DeckGameState::Idle,
            source_supported: false,
            source_up: false,
            samples_total: 0,
            dropped_zero_total: 0,
            dropped_too_large_total: 0,
            dropped_invalid_sentinel_total: 0,
            dropped_unsupported_version_total: 0,
            dropped_too_short_total: 0,
            dropped_wrong_session_total: 0,
            last_sample_timestamp_seconds: None,
            last_sample_instant: None,
            last_sample_interval: None,
            last_sample_payload_bytes: None,
            last_sample_output_width: None,
            last_sample_output_height: None,
            frame_source,
            frame_queue_label: frame_source.unavailable_queue_label(),
            source_health,
            mangohud_log_tail: MangoHudLogTail::default(),
            start: Instant::now(),
            last_wayland_open_attempt: None,
            last_mangoapp_open_attempt: None,
            #[cfg(target_os = "linux")]
            wayland_reader: None,
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
            self.refresh_frame_source_status();
            self.source_up = false;
            self.record_selected_source_health();
            return;
        }

        self.set_current_game(active_app_id);
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
        self.session_start_timestamp_seconds = app_id.map(|_| unix_timestamp_seconds());
        self.clear_windows();
        self.clear_frame_sample_state();
        self.source_up = false;
    }

    fn clear_windows(&mut self) {
        for window in &mut self.windows {
            window.window.clear();
        }
    }

    fn clear_frame_sample_state(&mut self) {
        self.last_sample_instant = None;
        self.last_sample_interval = None;
        self.last_sample_timestamp_seconds = None;
        self.last_sample_payload_bytes = None;
        self.last_sample_output_width = None;
        self.last_sample_output_height = None;
    }

    fn record_accepted_sample_timestamp(&mut self, now: Instant) {
        self.last_sample_interval = self
            .last_sample_instant
            .map(|previous| now.saturating_duration_since(previous));
        self.last_sample_timestamp_seconds = Some(unix_timestamp_seconds());
        self.last_sample_instant = Some(now);
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
        let state_labels = self.session_labels();

        metrics.push(MetricSample::gauge(
            names::GAME_SESSION_ACTIVE,
            "Whether a game session is active.",
            state_labels.clone(),
            if active { 1.0 } else { 0.0 },
        ));
        metrics.push(MetricSample::gauge(
            names::GAME_SESSION_FOCUSED,
            "Whether the active game is focused and visible.",
            state_labels.clone(),
            if focused { 1.0 } else { 0.0 },
        ));
        if active {
            if let Some(timestamp) = self.session_start_timestamp_seconds {
                metrics.push(MetricSample::gauge(
                    names::GAME_SESSION_START_TS_S,
                    "Unix timestamp when the current game session started.",
                    state_labels.clone(),
                    timestamp as f64,
                ));
            }
        }

        if let Some(app_id) = self.current_app_id {
            let mut labels = BTreeMap::new();
            labels.insert("appid".to_string(), app_id.to_string());
            labels.insert(
                "identity_source".to_string(),
                "steam_appmanifest".to_string(),
            );
            labels.insert(
                "title".to_string(),
                self.current_game_name
                    .clone()
                    .unwrap_or_else(|| "unknown".to_string()),
            );
            metrics.push(MetricSample::gauge(
                names::GAME_SESSION_INFO,
                "Game identity resolved from local Steam metadata.",
                labels,
                1.0,
            ));
        }

        for source in configured_frame_sources(&self.config) {
            let health = self
                .source_health
                .get(&source)
                .copied()
                .unwrap_or_else(|| FrameSourceHealth::unavailable(source));
            let source_labels = self.source_labels_for(source);
            metrics.push(MetricSample::gauge(
                names::GAME_FPS_SOURCE_SELECTED,
                "Whether this FPS source is currently selected.",
                source_labels.clone(),
                if source == self.frame_source {
                    1.0
                } else {
                    0.0
                },
            ));
            metrics.push(MetricSample::gauge(
                names::GAME_FPS_SOURCE_AVAILABLE,
                "Whether this FPS source is available.",
                source_labels.clone(),
                if health.supported { 1.0 } else { 0.0 },
            ));
            metrics.push(MetricSample::gauge(
                names::GAME_FPS_SOURCE_HEALTHY,
                "Whether this FPS source is currently healthy.",
                source_labels,
                if health.up { 1.0 } else { 0.0 },
            ));

            metrics.push(MetricSample::gauge(
                names::GAME_FPS_SOURCE_BACKEND_INFO,
                "Backend-specific FPS source metadata.",
                self.frame_source_debug_labels_for(source, health.queue_label),
                1.0,
            ));
        }

        let source_labels = self.frame_source_labels();
        metrics.push(MetricSample::counter(
            names::GAME_FPS_SOURCE_SAMPLES_TOTAL,
            "Total accepted frame timing samples read from the FPS source.",
            source_labels.clone(),
            self.samples_total as f64,
        ));
        for (reason, value) in [
            ("invalid", self.dropped_invalid_total()),
            ("too_large", self.dropped_too_large_total),
            ("stale", 0),
            ("out_of_order", 0),
            ("wrong_session", self.dropped_wrong_session_total),
        ] {
            metrics.push(MetricSample::counter(
                names::GAME_FPS_SOURCE_SAMPLE_DROPS_TOTAL,
                "Total frame timing samples dropped by source sanity filters.",
                self.frame_source_reason_labels(reason),
                value as f64,
            ));
        }
        if let Some(timestamp) = self.last_sample_timestamp_seconds {
            metrics.push(MetricSample::gauge(
                names::GAME_FPS_SOURCE_SAMPLE_LAST_TS_S,
                "Unix timestamp of the last accepted frame timing sample.",
                source_labels.clone(),
                timestamp as f64,
            ));
        }
        if let Some(last_sample) = self.last_sample_instant {
            metrics.push(MetricSample::gauge(
                names::GAME_FPS_SOURCE_SAMPLE_AGE_S,
                "Age in seconds of the last accepted frame timing sample.",
                source_labels.clone(),
                last_sample.elapsed().as_secs_f64(),
            ));
        }
        if let Some(interval) = self.last_sample_interval {
            metrics.push(MetricSample::gauge(
                names::GAME_FPS_SOURCE_SAMPLE_INTERVAL_MS,
                "Wall-clock interval since the previous accepted frame timing sample.",
                source_labels.clone(),
                interval.as_secs_f64() * 1_000.0,
            ));
        }
        let debug_source_labels = self.frame_source_debug_labels();
        if let Some(payload_bytes) = self.last_sample_payload_bytes {
            metrics.push(MetricSample::gauge(
                names::GAME_FPS_SOURCE_SAMPLE_PAYLOAD_BYTES,
                "Payload bytes in the last accepted frame timing sample.",
                debug_source_labels.clone(),
                payload_bytes as f64,
            ));
        }
        for (axis, value) in [
            ("width", self.last_sample_output_width),
            ("height", self.last_sample_output_height),
        ] {
            if let Some(value) = value {
                let mut labels = debug_source_labels.clone();
                labels.insert("axis".to_string(), axis.to_string());
                metrics.push(MetricSample::gauge(
                    names::GAME_FPS_SOURCE_OUTPUT_PIXELS,
                    "Game frame output dimension in pixels from the FPS source.",
                    labels,
                    value as f64,
                ));
            }
        }

        if active {
            for window in &self.windows {
                if let Some(summary) = window.window.summary() {
                    let base_labels = self.frame_labels(Some(&window.label));
                    metrics.extend(summary_metrics(base_labels, &summary));
                }
            }
        }

        metrics
    }

    fn session_labels(&self) -> BTreeMap<String, String> {
        let mut labels = BTreeMap::new();
        labels.insert(
            "state_source".to_string(),
            GAMESCOPE_STATE_SOURCE.to_string(),
        );
        self.add_game_labels(&mut labels);
        labels
    }

    fn frame_labels(&self, window: Option<&str>) -> BTreeMap<String, String> {
        let mut labels = self.frame_source_labels();
        if let Some(window) = window {
            labels.insert("window".to_string(), window.to_string());
        }
        self.add_game_labels(&mut labels);
        labels
    }

    fn add_game_labels(&self, labels: &mut BTreeMap<String, String>) {
        if self.config.include_appid_label {
            if let Some(app_id) = self.current_app_id {
                labels.insert("appid".to_string(), app_id.to_string());
            }
        }
        if self.config.include_game_name_label {
            if let Some(title) = &self.current_game_name {
                labels.insert("title".to_string(), title.clone());
            }
        }
    }

    fn frame_source_labels(&self) -> BTreeMap<String, String> {
        self.source_labels_for(self.frame_source)
    }

    fn source_labels_for(&self, source: FrameSourceKind) -> BTreeMap<String, String> {
        let mut labels = BTreeMap::new();
        labels.insert("source".to_string(), source.metric_source().to_string());
        labels
    }

    fn frame_source_debug_labels(&self) -> BTreeMap<String, String> {
        self.frame_source_debug_labels_for(self.frame_source, self.frame_queue_label)
    }

    fn frame_source_debug_labels_for(
        &self,
        source: FrameSourceKind,
        queue_label: &'static str,
    ) -> BTreeMap<String, String> {
        let mut labels = self.source_labels_for(source);
        labels.insert("queue".to_string(), queue_label.to_string());
        labels
    }

    fn frame_source_reason_labels(&self, reason: &str) -> BTreeMap<String, String> {
        let mut labels = self.frame_source_labels();
        labels.insert("reason".to_string(), reason.to_string());
        labels
    }

    fn dropped_invalid_total(&self) -> u64 {
        self.dropped_zero_total
            .saturating_add(self.dropped_invalid_sentinel_total)
            .saturating_add(self.dropped_unsupported_version_total)
            .saturating_add(self.dropped_too_short_total)
    }

    fn set_frame_source(&mut self, source: FrameSourceKind, queue_label: &'static str) {
        if self.frame_source != source || self.frame_queue_label != queue_label {
            self.frame_source = source;
            self.frame_queue_label = queue_label;
            self.clear_windows();
            self.source_up = false;
            self.clear_frame_sample_state();
        }
        self.source_health
            .entry(source)
            .or_insert_with(|| FrameSourceHealth::unavailable(source))
            .queue_label = queue_label;
    }

    fn record_source_health(
        &mut self,
        source: FrameSourceKind,
        supported: bool,
        up: bool,
        queue_label: &'static str,
    ) {
        self.source_health.insert(
            source,
            FrameSourceHealth {
                supported,
                up,
                queue_label,
            },
        );
    }

    fn record_selected_source_health(&mut self) {
        self.record_source_health(
            self.frame_source,
            self.source_supported,
            self.source_up,
            self.frame_queue_label,
        );
    }

    fn monotonic_nanos(&self) -> u64 {
        self.start
            .elapsed()
            .as_nanos()
            .try_into()
            .unwrap_or(u64::MAX)
    }

    fn refresh_frame_source_status(&mut self) {
        for source in configured_frame_sources(&self.config) {
            match source {
                FrameSourceKind::GamescopeWayland => {
                    self.set_frame_source(
                        FrameSourceKind::GamescopeWayland,
                        FRAME_SOURCE_QUEUE_NOT_APPLICABLE,
                    );
                    self.try_open_wayland_reader();
                }
                FrameSourceKind::MangoHudLog => {
                    self.set_frame_source(
                        FrameSourceKind::MangoHudLog,
                        FRAME_SOURCE_QUEUE_NOT_APPLICABLE,
                    );
                    self.source_supported =
                        discover_mangohud_log_path(&self.config.mangohud_log).is_some();
                    self.source_up = false;
                    self.record_selected_source_health();
                }
                FrameSourceKind::GamescopeMangoapp => {
                    self.set_frame_source(
                        FrameSourceKind::GamescopeMangoapp,
                        FRAME_SOURCE_QUEUE_UNAVAILABLE,
                    );
                    self.try_open_mangoapp_reader();
                }
            }

            if let Some(health) = self.source_health.get(&source).copied() {
                if health.supported {
                    self.set_frame_source(source, health.queue_label);
                    self.source_supported = health.supported;
                    self.source_up = false;
                    self.record_selected_source_health();
                    return;
                }
            }
        }

        self.source_supported = false;
        self.source_up = false;
        self.record_source_health(
            FrameSourceKind::GamescopeWayland,
            false,
            false,
            FRAME_SOURCE_QUEUE_NOT_APPLICABLE,
        );
    }

    fn read_frames(&mut self) -> Vec<FrameInput> {
        for source in configured_frame_sources(&self.config) {
            self.set_frame_source(source, source.unavailable_queue_label());
            let frames = match source {
                FrameSourceKind::GamescopeWayland => self.read_wayland_frames(),
                FrameSourceKind::MangoHudLog => self.read_mangohud_log_frames(),
                FrameSourceKind::GamescopeMangoapp => self.read_mangoapp_frames(),
            };

            if let Some(health) = self.source_health.get(&source).copied() {
                if health.supported {
                    self.set_frame_source(source, health.queue_label);
                    self.source_supported = health.supported;
                    self.source_up = health.up;
                    return frames;
                }
            }
        }

        self.source_supported = false;
        self.source_up = false;
        Vec::new()
    }

    fn read_wayland_frames(&mut self) -> Vec<FrameInput> {
        #[cfg(target_os = "linux")]
        {
            self.try_open_wayland_reader();
            let Some(app_id) = self.current_app_id else {
                self.source_up = false;
                self.record_source_health(
                    FrameSourceKind::GamescopeWayland,
                    self.source_supported,
                    false,
                    FRAME_SOURCE_QUEUE_NOT_APPLICABLE,
                );
                return Vec::new();
            };
            let Some(reader) = &mut self.wayland_reader else {
                self.source_up = false;
                return Vec::new();
            };

            let max_frame_time_ns = self
                .config
                .max_frame_time_milliseconds
                .saturating_mul(1_000_000);
            let max_messages = self
                .config
                .max_messages_per_poll
                .try_into()
                .unwrap_or(usize::MAX);
            let mut samples = Vec::new();
            match reader.read_available(app_id, max_frame_time_ns, max_messages, &mut samples) {
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
                    self.dropped_wrong_session_total = self
                        .dropped_wrong_session_total
                        .saturating_add(result.dropped_wrong_session);
                    let now = Instant::now();
                    if result.samples_read > 0 {
                        self.record_accepted_sample_timestamp(now);
                        self.last_sample_payload_bytes = None;
                        self.last_sample_output_width = None;
                        self.last_sample_output_height = None;
                    }
                    self.refresh_source_up(now, self.last_state.is_game_running());
                    self.record_source_health(
                        FrameSourceKind::GamescopeWayland,
                        self.source_supported,
                        self.source_up,
                        FRAME_SOURCE_QUEUE_NOT_APPLICABLE,
                    );
                    samples
                        .into_iter()
                        .map(|sample| FrameInput {
                            visible_frametime_ns: sample.visible_frametime_ns,
                        })
                        .collect()
                }
                Err(error) => {
                    warn!(%error, source = GAMESCOPE_WAYLAND_SOURCE, "Gamescope Wayland FPS source read failed");
                    self.wayland_reader = None;
                    self.set_frame_source(
                        FrameSourceKind::GamescopeWayland,
                        FRAME_SOURCE_QUEUE_NOT_APPLICABLE,
                    );
                    self.source_supported = false;
                    self.source_up = false;
                    self.record_selected_source_health();
                    Vec::new()
                }
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            self.source_supported = false;
            self.source_up = false;
            Vec::new()
        }
    }

    fn read_mangohud_log_frames(&mut self) -> Vec<FrameInput> {
        let max_frame_time_ns = self
            .config
            .max_frame_time_milliseconds
            .saturating_mul(1_000_000);
        let max_rows = self
            .config
            .max_messages_per_poll
            .try_into()
            .unwrap_or(usize::MAX);

        match self.mangohud_log_tail.read_available(
            &self.config.mangohud_log,
            max_frame_time_ns,
            max_rows,
        ) {
            Ok(result) => {
                self.source_supported = result.supported;
                self.dropped_zero_total =
                    self.dropped_zero_total.saturating_add(result.dropped_zero);
                self.dropped_too_large_total = self
                    .dropped_too_large_total
                    .saturating_add(result.dropped_too_large);
                self.samples_total = self
                    .samples_total
                    .saturating_add(result.frames.len() as u64);
                let now = Instant::now();
                if !result.frames.is_empty() {
                    self.record_accepted_sample_timestamp(now);
                    self.last_sample_payload_bytes = None;
                    self.last_sample_output_width = None;
                    self.last_sample_output_height = None;
                }
                self.refresh_source_up(now, self.last_state.is_game_running());
                self.record_source_health(
                    FrameSourceKind::MangoHudLog,
                    self.source_supported,
                    self.source_up,
                    FRAME_SOURCE_QUEUE_NOT_APPLICABLE,
                );
                result.frames
            }
            Err(error) => {
                warn!(%error, source = MANGOHUD_LOG_SOURCE, "game frame log read failed");
                self.source_supported = false;
                self.source_up = false;
                self.record_source_health(
                    FrameSourceKind::MangoHudLog,
                    false,
                    false,
                    FRAME_SOURCE_QUEUE_NOT_APPLICABLE,
                );
                Vec::new()
            }
        }
    }

    fn read_mangoapp_frames(&mut self) -> Vec<FrameInput> {
        #[cfg(target_os = "linux")]
        {
            self.try_open_mangoapp_reader();
            let Some(reader) = &mut self.reader else {
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
                    let now = Instant::now();
                    if result.samples_read > 0 {
                        self.record_accepted_sample_timestamp(now);
                        if let Some(sample) = samples.last() {
                            self.last_sample_payload_bytes = Some(sample.payload_len);
                            self.last_sample_output_width = sample.output_width;
                            self.last_sample_output_height = sample.output_height;
                        }
                    }
                    self.refresh_source_up(now, self.last_state.is_game_running());
                    self.record_selected_source_health();
                    samples
                        .into_iter()
                        .map(|sample| FrameInput {
                            visible_frametime_ns: sample.visible_frametime_ns,
                        })
                        .collect()
                }
                Err(error) => {
                    let queue = reader.queue_label();
                    warn!(%error, source = GAMESCOPE_FRAME_SOURCE, queue, "game frame source read failed");
                    self.reader = None;
                    self.set_frame_source(
                        FrameSourceKind::GamescopeMangoapp,
                        FRAME_SOURCE_QUEUE_UNAVAILABLE,
                    );
                    self.source_supported = false;
                    self.source_up = false;
                    self.record_selected_source_health();
                    Vec::new()
                }
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            self.source_supported = false;
            self.source_up = false;
            Vec::new()
        }
    }

    #[cfg(target_os = "linux")]
    fn try_open_wayland_reader(&mut self) {
        if !self.config.gamescope_wayland.enabled {
            self.wayland_reader = None;
            self.source_supported = false;
            self.source_up = false;
            self.record_source_health(
                FrameSourceKind::GamescopeWayland,
                false,
                false,
                FRAME_SOURCE_QUEUE_NOT_APPLICABLE,
            );
            return;
        }

        let now = Instant::now();
        if self.wayland_reader.is_some() {
            self.set_frame_source(
                FrameSourceKind::GamescopeWayland,
                FRAME_SOURCE_QUEUE_NOT_APPLICABLE,
            );
            self.source_supported = true;
            self.refresh_source_up(now, self.last_state.is_game_running());
            self.record_selected_source_health();
            return;
        }

        if self
            .last_wayland_open_attempt
            .is_some_and(|last| now.duration_since(last) < Duration::from_secs(1))
        {
            return;
        }
        self.last_wayland_open_attempt = Some(now);

        match GamescopeWaylandFrameReader::open(&self.config.gamescope_wayland) {
            Ok(reader) => {
                self.wayland_reader = Some(reader);
                self.set_frame_source(
                    FrameSourceKind::GamescopeWayland,
                    FRAME_SOURCE_QUEUE_NOT_APPLICABLE,
                );
                self.source_supported = true;
                self.refresh_source_up(now, self.last_state.is_game_running());
                self.record_selected_source_health();
                debug!(
                    source = GAMESCOPE_WAYLAND_SOURCE,
                    "Gamescope Wayland FPS source opened"
                );
            }
            Err(error) => {
                self.set_frame_source(
                    FrameSourceKind::GamescopeWayland,
                    FRAME_SOURCE_QUEUE_NOT_APPLICABLE,
                );
                self.source_supported = false;
                self.source_up = false;
                self.record_selected_source_health();
                debug!(%error, source = GAMESCOPE_WAYLAND_SOURCE, "Gamescope Wayland FPS source unavailable");
            }
        }
    }

    #[cfg(not(target_os = "linux"))]
    fn try_open_wayland_reader(&mut self) {
        self.source_supported = false;
        self.source_up = false;
        self.record_source_health(
            FrameSourceKind::GamescopeWayland,
            false,
            false,
            FRAME_SOURCE_QUEUE_NOT_APPLICABLE,
        );
    }

    #[cfg(target_os = "linux")]
    fn try_open_mangoapp_reader(&mut self) {
        if !self.config.gamescope_mangoapp.enabled {
            self.reader = None;
            self.source_supported = false;
            self.source_up = false;
            self.record_source_health(
                FrameSourceKind::GamescopeMangoapp,
                false,
                false,
                FRAME_SOURCE_QUEUE_UNAVAILABLE,
            );
            return;
        }

        if !self.config.gamescope_mangoapp.allow_destructive_read {
            self.reader = None;
            self.set_frame_source(
                FrameSourceKind::GamescopeMangoapp,
                FRAME_SOURCE_QUEUE_DESTRUCTIVE_READ_DISABLED,
            );
            self.source_supported = false;
            self.source_up = false;
            self.record_source_health(
                FrameSourceKind::GamescopeMangoapp,
                false,
                false,
                FRAME_SOURCE_QUEUE_DESTRUCTIVE_READ_DISABLED,
            );
            debug!(
                source = GAMESCOPE_FRAME_SOURCE,
                queue = FRAME_SOURCE_QUEUE_DESTRUCTIVE_READ_DISABLED,
                "game frame source blocked because destructive queue reads are disabled"
            );
            return;
        }

        if mangoapp_competing_consumer_running() {
            self.reader = None;
            self.set_frame_source(
                FrameSourceKind::GamescopeMangoapp,
                FRAME_SOURCE_QUEUE_BLOCKED,
            );
            self.source_supported = false;
            self.source_up = false;
            self.record_source_health(
                FrameSourceKind::GamescopeMangoapp,
                false,
                false,
                FRAME_SOURCE_QUEUE_BLOCKED,
            );
            debug!(
                source = GAMESCOPE_FRAME_SOURCE,
                queue = FRAME_SOURCE_QUEUE_BLOCKED,
                "game frame source blocked because MangoHud/mangoapp appears to be running"
            );
            return;
        }

        let now = Instant::now();
        let queue_label = if let Some(reader) = &mut self.reader {
            reader.refresh_sources(&self.config.gamescope_mangoapp);
            Some(reader.queue_label())
        } else {
            None
        };
        if let Some(queue_label) = queue_label {
            self.set_frame_source(FrameSourceKind::GamescopeMangoapp, queue_label);
            self.source_supported = true;
            self.refresh_source_up(now, self.last_state.is_game_running());
            self.record_selected_source_health();
            return;
        }

        if self
            .last_mangoapp_open_attempt
            .is_some_and(|last| now.duration_since(last) < Duration::from_secs(1))
        {
            return;
        }
        self.last_mangoapp_open_attempt = Some(now);
        let max_frame_time_ns = self
            .config
            .max_frame_time_milliseconds
            .saturating_mul(1_000_000);
        match MangoAppFrameReader::open(&self.config.gamescope_mangoapp, max_frame_time_ns) {
            Ok(reader) => {
                let queue_label = reader.queue_label();
                self.reader = Some(reader);
                self.set_frame_source(FrameSourceKind::GamescopeMangoapp, queue_label);
                self.source_supported = true;
                self.refresh_source_up(now, self.last_state.is_game_running());
                self.record_selected_source_health();
                debug!(
                    source = GAMESCOPE_FRAME_SOURCE,
                    queue = queue_label,
                    "game frame source opened"
                );
            }
            Err(error) => {
                self.set_frame_source(
                    FrameSourceKind::GamescopeMangoapp,
                    FRAME_SOURCE_QUEUE_UNAVAILABLE,
                );
                self.source_supported = false;
                self.source_up = false;
                self.record_selected_source_health();
                debug!(%error, source = GAMESCOPE_FRAME_SOURCE, "game frame source unavailable");
            }
        }
    }

    #[cfg(not(target_os = "linux"))]
    fn try_open_mangoapp_reader(&mut self) {
        self.source_supported = false;
        self.source_up = false;
        self.record_source_health(
            FrameSourceKind::GamescopeMangoapp,
            false,
            false,
            FRAME_SOURCE_QUEUE_UNAVAILABLE,
        );
    }

    fn refresh_source_up(&mut self, now: Instant, active: bool) {
        self.source_up = active
            && self
                .last_sample_instant
                .is_some_and(|last| now.duration_since(last) <= FRAME_SOURCE_UP_STALE_AFTER);
    }
}

fn initial_frame_source(config: &SteamDeckFpsConfig) -> FrameSourceKind {
    configured_frame_sources(config)
        .into_iter()
        .next()
        .unwrap_or(FrameSourceKind::GamescopeWayland)
}

fn configured_frame_sources(config: &SteamDeckFpsConfig) -> Vec<FrameSourceKind> {
    let mut sources = Vec::new();
    for source in &config.source_preference {
        let Some(source) = FrameSourceKind::from_config_name(source) else {
            continue;
        };
        if source_enabled(config, source) && !sources.contains(&source) {
            sources.push(source);
        }
    }

    for source in [
        FrameSourceKind::GamescopeWayland,
        FrameSourceKind::MangoHudLog,
        FrameSourceKind::GamescopeMangoapp,
    ] {
        if source_enabled(config, source) && !sources.contains(&source) {
            sources.push(source);
        }
    }

    sources
}

fn source_enabled(config: &SteamDeckFpsConfig, source: FrameSourceKind) -> bool {
    match source {
        FrameSourceKind::GamescopeWayland => config.gamescope_wayland.enabled,
        FrameSourceKind::MangoHudLog => config.mangohud_log.enabled,
        FrameSourceKind::GamescopeMangoapp => config.gamescope_mangoapp.enabled,
    }
}

fn discover_mangohud_log_path(config: &MangoHudLogConfig) -> Option<PathBuf> {
    let mut best: Option<(SystemTime, PathBuf)> = None;
    for path in &config.paths {
        collect_mangohud_log_candidates(path, &mut best);
    }

    if config.auto_discover {
        for dir in mangohud_auto_discovery_dirs() {
            collect_mangohud_log_candidates(&dir, &mut best);
        }
    }

    best.map(|(_, path)| path)
}

fn collect_mangohud_log_candidates(path: &Path, best: &mut Option<(SystemTime, PathBuf)>) {
    if path.is_file() {
        maybe_update_mangohud_log_candidate(path, best);
        return;
    }

    if !path.is_dir() {
        return;
    }

    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let candidate = entry.path();
        if candidate.is_file() {
            maybe_update_mangohud_log_candidate(&candidate, best);
        }
    }
}

fn maybe_update_mangohud_log_candidate(path: &Path, best: &mut Option<(SystemTime, PathBuf)>) {
    if !is_mangohud_log_file(path) {
        return;
    }
    let Ok(metadata) = fs::metadata(path) else {
        return;
    };
    let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
    if best.as_ref().is_none_or(|(current, _)| modified > *current) {
        *best = Some((modified, path.to_path_buf()));
    }
}

fn is_mangohud_log_file(path: &Path) -> bool {
    let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
        return false;
    };
    if !extension.eq_ignore_ascii_case("csv") {
        return false;
    }
    let Some(file_name) = path.file_name().and_then(|value| value.to_str()) else {
        return false;
    };
    !file_name.ends_with("_summary.csv")
}

fn mangohud_auto_discovery_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        dirs.push(home.clone());
        dirs.push(home.join("mangologs"));
        dirs.push(home.join("MangoHud"));
        dirs.push(home.join("Documents"));
        dirs.push(home.join("Desktop"));
    }
    if let Some(state_home) = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from) {
        dirs.push(state_home.join("MangoHud"));
    }
    if let Some(data_home) = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from) {
        dirs.push(data_home.join("MangoHud"));
    }
    dirs
}

fn parse_mangohud_log_header(line: &str) -> Option<MangoHudLogHeader> {
    let columns: Vec<String> = split_mangohud_csv_line(line)
        .into_iter()
        .map(|value| value.trim().to_ascii_lowercase())
        .collect();
    let fps_index = columns.iter().position(|value| value == "fps");
    let frametime_index = columns.iter().position(|value| value == "frametime");
    if fps_index.is_some() || frametime_index.is_some() {
        Some(MangoHudLogHeader {
            fps_index,
            frametime_index,
        })
    } else {
        None
    }
}

fn parse_mangohud_log_frame(
    line: &str,
    header: MangoHudLogHeader,
    max_frame_time_ns: u64,
) -> ParsedMangoHudLogFrame {
    if parse_mangohud_log_header(line).is_some() || line.trim().is_empty() {
        return ParsedMangoHudLogFrame::Ignore;
    }

    let columns = split_mangohud_csv_line(line);
    let mut frame_time_ns = header
        .frametime_index
        .and_then(|index| columns.get(index))
        .and_then(|value| parse_non_negative_float(value))
        .map(|milliseconds| (milliseconds * 1_000_000.0).round() as u64);

    if frame_time_ns.is_none() {
        frame_time_ns = header
            .fps_index
            .and_then(|index| columns.get(index))
            .and_then(|value| parse_positive_float(value))
            .map(|fps| (1_000_000_000.0 / fps).round() as u64);
    }

    let Some(frame_time_ns) = frame_time_ns else {
        return ParsedMangoHudLogFrame::Ignore;
    };
    if frame_time_ns == 0 {
        return ParsedMangoHudLogFrame::Zero;
    }
    if frame_time_ns > max_frame_time_ns {
        return ParsedMangoHudLogFrame::TooLarge;
    }

    ParsedMangoHudLogFrame::Frame(FrameInput {
        visible_frametime_ns: frame_time_ns,
    })
}

fn parse_non_negative_float(value: &str) -> Option<f64> {
    let value = value.trim();
    let parsed: f64 = value.parse().ok()?;
    parsed
        .is_finite()
        .then_some(parsed)
        .filter(|value| *value >= 0.0)
}

fn parse_positive_float(value: &str) -> Option<f64> {
    parse_non_negative_float(value).filter(|value| *value > 0.0)
}

fn split_mangohud_csv_line(line: &str) -> Vec<&str> {
    line.trim_end_matches('\r').split(',').collect()
}

#[cfg(target_os = "linux")]
fn mangoapp_competing_consumer_running() -> bool {
    let Ok(entries) = fs::read_dir("/proc") else {
        return false;
    };
    let current_pid = std::process::id();
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let Some(pid_text) = file_name.to_str() else {
            continue;
        };
        let Ok(pid) = pid_text.parse::<u32>() else {
            continue;
        };
        if pid == current_pid {
            continue;
        }
        let proc_dir = entry.path();
        if fs::read_to_string(proc_dir.join("comm"))
            .ok()
            .is_some_and(|name| is_mangoapp_consumer_name(name.trim()))
        {
            return true;
        }
        if fs::read(proc_dir.join("cmdline"))
            .ok()
            .and_then(|bytes| first_cmdline_arg(&bytes).map(str::to_string))
            .is_some_and(|arg| is_mangoapp_consumer_name(command_basename(&arg)))
        {
            return true;
        }
    }
    false
}

#[cfg(target_os = "linux")]
fn first_cmdline_arg(bytes: &[u8]) -> Option<&str> {
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    std::str::from_utf8(&bytes[..end])
        .ok()
        .filter(|arg| !arg.is_empty())
}

#[cfg(target_os = "linux")]
fn command_basename(value: &str) -> &str {
    Path::new(value)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(value)
}

#[cfg(target_os = "linux")]
fn is_mangoapp_consumer_name(value: &str) -> bool {
    let value = value.to_ascii_lowercase();
    value == "mangoapp" || value == "mangohud" || value.starts_with("mangohud.")
}

fn summary_metrics(
    base_labels: BTreeMap<String, String>,
    summary: &telemon_collectors::game::capture::CaptureSummary,
) -> Vec<MetricSample> {
    let mut metrics = Vec::new();
    push_game_metric(
        &mut metrics,
        names::GAME_FRAME_SAMPLES,
        "Game frames observed in a rolling window.",
        base_labels.clone(),
        summary.frame_count as f64,
    );
    push_game_stat_metric(
        &mut metrics,
        names::GAME_FPS,
        "Game frame rate in frames per second by statistic.",
        &base_labels,
        "avg",
        summary.average_fps,
    );
    push_game_stat_metric(
        &mut metrics,
        names::GAME_FPS,
        "Game frame rate in frames per second by statistic.",
        &base_labels,
        "low_1pct",
        summary.low_1pct_fps_average,
    );
    push_game_stat_metric(
        &mut metrics,
        names::GAME_FPS,
        "Game frame rate in frames per second by statistic.",
        &base_labels,
        "low_0_1pct",
        summary.low_01pct_fps_average,
    );

    for (stat, value) in [
        ("avg", summary.average_frame_time_seconds),
        ("min", summary.min_frame_time_seconds),
        ("max", summary.max_frame_time_seconds),
        ("p50", summary.p50_frame_time_seconds),
        ("p95", summary.p95_frame_time_seconds),
        ("p99", summary.p99_frame_time_seconds),
    ] {
        push_game_stat_metric(
            &mut metrics,
            names::GAME_FRAME_MS,
            "Game frame time in milliseconds by statistic.",
            &base_labels,
            stat,
            value * 1_000.0,
        );
    }

    if let Some(jitter) = summary.jitter {
        for (stat, value) in [
            ("avg", jitter.average_seconds),
            ("p95", jitter.p95_seconds),
            ("p99", jitter.p99_seconds),
            ("max", jitter.max_seconds),
        ] {
            push_game_stat_metric(
                &mut metrics,
                names::GAME_JITTER_MS,
                "Adjacent frame-time delta in milliseconds by statistic.",
                &base_labels,
                stat,
                value * 1_000.0,
            );
        }
    }

    metrics
}

fn push_game_metric(
    metrics: &mut Vec<MetricSample>,
    name: &str,
    help: &str,
    base_labels: BTreeMap<String, String>,
    value: f64,
) {
    metrics.push(MetricSample::gauge(name, help, base_labels, value));
}

fn push_game_stat_metric(
    metrics: &mut Vec<MetricSample>,
    name: &str,
    help: &str,
    base_labels: &BTreeMap<String, String>,
    stat: &str,
    value: f64,
) {
    let mut labels = base_labels.clone();
    labels.insert("stat".to_string(), stat.to_string());
    metrics.push(MetricSample::gauge(name, help, labels, value));
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

    fn metric_value(metrics: &[MetricSample], name: &str, labels: &[(&str, &str)]) -> Option<f64> {
        metrics
            .iter()
            .find(|metric| {
                metric.name == name
                    && labels.iter().all(|(key, value)| {
                        metric
                            .labels
                            .get(*key)
                            .is_some_and(|actual| actual == value)
                    })
            })
            .map(|metric| metric.value)
    }

    #[test]
    fn summary_metrics_include_appid_and_title_labels() {
        let summary = CaptureStats::from_frame_times_ns([16_000_000, 17_000_000, 33_000_000])
            .summary()
            .unwrap();
        let mut labels = BTreeMap::new();
        labels.insert("source".to_string(), GAMESCOPE_FRAME_SOURCE.to_string());
        labels.insert("window".to_string(), "1s".to_string());
        labels.insert("appid".to_string(), "1234".to_string());
        labels.insert("title".to_string(), "Example Game".to_string());

        let metrics = summary_metrics(labels, &summary);

        assert!(metrics
            .iter()
            .any(|metric| metric.name == names::GAME_FPS_AVG));
        assert!(metrics.iter().all(|metric| metric
            .labels
            .get("appid")
            .is_some_and(|value| value == "1234")));
        assert!(metrics.iter().all(|metric| metric
            .labels
            .get("title")
            .is_some_and(|value| value == "Example Game")));
    }

    #[test]
    fn mangohud_log_parser_accepts_versioned_header_and_rows() {
        let header = parse_mangohud_log_header(
            "fps,frametime,cpu_load,cpu_power,gpu_load,cpu_temp,gpu_temp,gpu_core_clock,gpu_mem_clock,gpu_vram_used,gpu_power,ram_used,swap_used,process_rss,cpu_mhz,elapsed",
        )
        .unwrap();

        let parsed = parse_mangohud_log_frame(
            "60,16.667,4,0,99,60,70,800,1600,1024,10,8000,0,100,3200,123456",
            header,
            1_000_000_000,
        );
        assert!(matches!(
            parsed,
            ParsedMangoHudLogFrame::Frame(FrameInput {
                visible_frametime_ns: 16_667_000
            })
        ));

        let parsed = parse_mangohud_log_frame(
            "120,,4,0,99,60,70,800,1600,1024,10,8000,0,100,3200,123456",
            header,
            1_000_000_000,
        );
        assert!(matches!(
            parsed,
            ParsedMangoHudLogFrame::Frame(FrameInput {
                visible_frametime_ns: 8_333_333
            })
        ));

        assert_eq!(
            parse_mangohud_log_header("--------------------FRAME METRICS--------------------"),
            None
        );
        assert_eq!(
            parse_mangohud_log_frame(
                "60,0,4,0,99,60,70,800,1600,1024,10,8000,0,100,3200,123456",
                header,
                1_000_000_000,
            ),
            ParsedMangoHudLogFrame::Zero
        );
        assert_eq!(
            parse_mangohud_log_frame(
                "60,1001,4,0,99,60,70,800,1600,1024,10,8000,0,100,3200,123456",
                header,
                1_000_000_000,
            ),
            ParsedMangoHudLogFrame::TooLarge
        );
    }

    #[test]
    fn mangohud_log_tail_discovers_latest_csv_and_reads_appended_rows() {
        use std::io::Write;

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "telemon-mangohud-log-test-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        let log_path = dir.join("Sekiro_2026-06-04_21-00-00.csv");
        fs::write(
            &log_path,
            "v1\n0.8.1\n---------------------SYSTEM INFO---------------------\nos,cpu,gpu,ram,kernel,driver,cpuscheduler\nlinux,cpu,gpu,ram,kernel,driver,sched\n--------------------FRAME METRICS--------------------\nfps,frametime,cpu_load,cpu_power,gpu_load,cpu_temp,gpu_temp,gpu_core_clock,gpu_mem_clock,gpu_vram_used,gpu_power,ram_used,swap_used,process_rss,cpu_mhz,elapsed\n",
        )
        .unwrap();

        let config = MangoHudLogConfig {
            enabled: true,
            paths: vec![dir.clone()],
            auto_discover: false,
        };
        let mut tail = MangoHudLogTail::default();
        let initial = tail.read_available(&config, 1_000_000_000, 512).unwrap();
        assert!(initial.supported);
        assert!(initial.frames.is_empty());

        let mut file = fs::OpenOptions::new().append(true).open(&log_path).unwrap();
        writeln!(
            file,
            "60,16.667,4,0,99,60,70,800,1600,1024,10,8000,0,100,3200,123456"
        )
        .unwrap();
        drop(file);

        let update = tail.read_available(&config, 1_000_000_000, 512).unwrap();
        assert!(update.supported);
        assert_eq!(update.frames.len(), 1);
        assert_eq!(update.frames[0].visible_frametime_ns, 16_667_000);

        fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn mangoapp_consumer_name_matching_is_specific() {
        assert!(is_mangoapp_consumer_name("mangoapp"));
        assert!(is_mangoapp_consumer_name("MangoHud"));
        assert!(is_mangoapp_consumer_name("mangohud.x86_64"));
        assert!(!is_mangoapp_consumer_name("telemon-exporter"));
        assert!(!is_mangoapp_consumer_name("steam"));
    }

    #[test]
    fn passive_mangohud_log_config_selects_log_source_without_direct_queue() {
        let config = SteamDeckFpsConfig {
            enabled: true,
            mangohud_log: MangoHudLogConfig {
                enabled: true,
                paths: Vec::new(),
                auto_discover: false,
            },
            ..Default::default()
        };
        let runtime = GameFpsRuntime::new(config, SteamDeckGameStateConfig::default());
        let metrics = runtime.metrics();

        assert_eq!(
            metric_value(
                &metrics,
                names::GAME_FPS_SOURCE_SELECTED,
                &[("source", MANGOHUD_LOG_SOURCE)]
            ),
            Some(1.0)
        );
        assert_eq!(
            metric_value(
                &metrics,
                names::GAME_FPS_SOURCE_AVAILABLE,
                &[("source", MANGOHUD_LOG_SOURCE)]
            ),
            Some(0.0)
        );
        assert_eq!(
            metric_value(
                &metrics,
                names::GAME_FPS_SOURCE_SELECTED,
                &[("source", GAMESCOPE_FRAME_SOURCE)]
            ),
            None
        );
    }

    #[test]
    fn passive_mangohud_log_stays_selected_with_stale_direct_queue_config() {
        let gamescope_mangoapp = telemon_core::config::GamescopeMangoappConfig {
            enabled: true,
            allow_destructive_read: false,
            ..Default::default()
        };
        let config = SteamDeckFpsConfig {
            enabled: true,
            source_preference: vec![
                GAMESCOPE_FRAME_SOURCE.to_string(),
                MANGOHUD_LOG_SOURCE.to_string(),
            ],
            mangohud_log: MangoHudLogConfig {
                enabled: true,
                paths: Vec::new(),
                auto_discover: false,
            },
            gamescope_mangoapp,
            ..Default::default()
        };
        let mut runtime = GameFpsRuntime::new(config, SteamDeckGameStateConfig::default());

        assert!(runtime.read_frames().is_empty());
        let metrics = runtime.metrics();

        assert_eq!(
            metric_value(
                &metrics,
                names::GAME_FPS_SOURCE_SELECTED,
                &[("source", MANGOHUD_LOG_SOURCE)]
            ),
            Some(1.0)
        );
        assert_eq!(
            metric_value(
                &metrics,
                names::GAME_FPS_SOURCE_SELECTED,
                &[("source", GAMESCOPE_FRAME_SOURCE)]
            ),
            Some(0.0)
        );
        assert_eq!(
            metric_value(
                &metrics,
                names::GAME_FPS_SOURCE_AVAILABLE,
                &[("source", MANGOHUD_LOG_SOURCE)]
            ),
            Some(0.0)
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn direct_queue_reports_disabled_when_destructive_reads_are_not_allowed() {
        let gamescope_mangoapp = telemon_core::config::GamescopeMangoappConfig {
            enabled: true,
            allow_destructive_read: false,
            ..Default::default()
        };
        let config = SteamDeckFpsConfig {
            enabled: true,
            mangohud_log: Default::default(),
            gamescope_mangoapp,
            ..Default::default()
        };
        let mut runtime = GameFpsRuntime::new(config, SteamDeckGameStateConfig::default());

        assert!(runtime.read_frames().is_empty());
        let metrics = runtime.metrics();

        assert_eq!(
            metric_value(
                &metrics,
                names::GAME_FPS_SOURCE_SELECTED,
                &[("source", GAMESCOPE_FRAME_SOURCE)]
            ),
            Some(1.0)
        );
        assert_eq!(
            metric_value(
                &metrics,
                names::GAME_FPS_SOURCE_AVAILABLE,
                &[("source", GAMESCOPE_FRAME_SOURCE)]
            ),
            Some(0.0)
        );
        assert_eq!(
            metric_value(
                &metrics,
                names::GAME_FPS_SOURCE_BACKEND_INFO,
                &[
                    ("source", GAMESCOPE_FRAME_SOURCE),
                    ("queue", FRAME_SOURCE_QUEUE_DESTRUCTIVE_READ_DISABLED)
                ]
            ),
            Some(1.0)
        );
    }

    #[test]
    fn configured_frame_sources_ignore_disabled_direct_queue() {
        let config = SteamDeckFpsConfig {
            enabled: true,
            mangohud_log: MangoHudLogConfig {
                enabled: true,
                paths: Vec::new(),
                auto_discover: false,
            },
            gamescope_mangoapp: Default::default(),
            ..Default::default()
        };

        assert_eq!(
            configured_frame_sources(&config),
            vec![FrameSourceKind::MangoHudLog]
        );
    }

    #[test]
    fn gamescope_wayland_source_is_preferred_when_enabled() {
        let config = SteamDeckFpsConfig {
            enabled: true,
            gamescope_wayland: telemon_core::config::GamescopeWaylandConfig {
                enabled: true,
                ..Default::default()
            },
            mangohud_log: MangoHudLogConfig {
                enabled: true,
                paths: Vec::new(),
                auto_discover: false,
            },
            ..Default::default()
        };
        let runtime = GameFpsRuntime::new(config.clone(), SteamDeckGameStateConfig::default());

        assert_eq!(
            configured_frame_sources(&config),
            vec![
                FrameSourceKind::GamescopeWayland,
                FrameSourceKind::MangoHudLog
            ]
        );
        let metrics = runtime.metrics();
        assert_eq!(
            metric_value(
                &metrics,
                names::GAME_FPS_SOURCE_SELECTED,
                &[("source", GAMESCOPE_WAYLAND_SOURCE)]
            ),
            Some(1.0)
        );
        assert_eq!(
            metric_value(
                &metrics,
                names::GAME_FPS_SOURCE_SELECTED,
                &[("source", MANGOHUD_LOG_SOURCE)]
            ),
            Some(0.0)
        );
    }

    #[test]
    fn unavailable_wayland_source_falls_through_to_mangohud_log() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "telemon-wayland-fallback-test-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        let log_path = dir.join("Spyro_2026-06-05_21-00-00.csv");
        fs::write(
            &log_path,
            "--------------------FRAME METRICS--------------------\nfps,frametime,elapsed\n",
        )
        .unwrap();

        let config = SteamDeckFpsConfig {
            enabled: true,
            source_preference: vec![
                GAMESCOPE_WAYLAND_SOURCE.to_string(),
                MANGOHUD_LOG_SOURCE.to_string(),
            ],
            gamescope_wayland: telemon_core::config::GamescopeWaylandConfig {
                enabled: true,
                display: dir.join("missing-gamescope-socket").display().to_string(),
            },
            mangohud_log: MangoHudLogConfig {
                enabled: true,
                paths: vec![dir.clone()],
                auto_discover: false,
            },
            ..Default::default()
        };
        let mut runtime = GameFpsRuntime::new(config, SteamDeckGameStateConfig::default());
        runtime.last_state = DeckGameState::GameFocusedVisible;
        runtime.set_current_game(Some(996_580));

        assert!(runtime.read_frames().is_empty());
        let metrics = runtime.metrics();
        assert_eq!(
            metric_value(
                &metrics,
                names::GAME_FPS_SOURCE_SELECTED,
                &[("source", MANGOHUD_LOG_SOURCE)]
            ),
            Some(1.0)
        );
        assert_eq!(
            metric_value(
                &metrics,
                names::GAME_FPS_SOURCE_AVAILABLE,
                &[("source", MANGOHUD_LOG_SOURCE)]
            ),
            Some(1.0)
        );
        assert_eq!(
            metric_value(
                &metrics,
                names::GAME_FPS_SOURCE_AVAILABLE,
                &[("source", GAMESCOPE_WAYLAND_SOURCE)]
            ),
            Some(0.0)
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn source_diagnostics_include_last_sample_metadata() {
        let config = SteamDeckFpsConfig {
            enabled: true,
            gamescope_mangoapp: telemon_core::config::GamescopeMangoappConfig {
                enabled: true,
                allow_destructive_read: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut runtime = GameFpsRuntime::new(config, SteamDeckGameStateConfig::default());
        runtime.set_frame_source(FrameSourceKind::GamescopeMangoapp, "configured_ftok");
        runtime.last_state = DeckGameState::GameFocusedVisible;
        runtime.source_supported = true;
        runtime.source_up = true;
        runtime.last_sample_timestamp_seconds = Some(12_345);
        runtime.last_sample_instant = Some(Instant::now() - Duration::from_millis(1_500));
        runtime.last_sample_interval = Some(Duration::from_millis(125));
        runtime.last_sample_payload_bytes = Some(85);
        runtime.last_sample_output_width = Some(1920);
        runtime.last_sample_output_height = Some(1080);
        runtime.record_selected_source_health();

        let metrics = runtime.metrics();
        let base_labels = &[("source", GAMESCOPE_FRAME_SOURCE)];
        let debug_labels = &[
            ("source", GAMESCOPE_FRAME_SOURCE),
            ("queue", "configured_ftok"),
        ];

        assert_eq!(
            metric_value(
                &metrics,
                names::GAME_FPS_SOURCE_SAMPLE_LAST_TS_S,
                base_labels,
            ),
            Some(12_345.0)
        );
        let age = metric_value(&metrics, names::GAME_FPS_SOURCE_SAMPLE_AGE_S, base_labels).unwrap();
        assert!((1.0..3.0).contains(&age), "unexpected sample age: {age}");
        assert_eq!(
            metric_value(
                &metrics,
                names::GAME_FPS_SOURCE_SAMPLE_INTERVAL_MS,
                base_labels,
            ),
            Some(125.0)
        );
        assert_eq!(
            metric_value(
                &metrics,
                names::GAME_FPS_SOURCE_SAMPLE_PAYLOAD_BYTES,
                debug_labels,
            ),
            Some(85.0)
        );
        assert_eq!(
            metric_value(
                &metrics,
                names::GAME_FPS_SOURCE_OUTPUT_PIXELS,
                &[
                    ("source", GAMESCOPE_FRAME_SOURCE),
                    ("queue", "configured_ftok"),
                    ("axis", "width")
                ],
            ),
            Some(1920.0)
        );
        assert_eq!(
            metric_value(
                &metrics,
                names::GAME_FPS_SOURCE_OUTPUT_PIXELS,
                &[
                    ("source", GAMESCOPE_FRAME_SOURCE),
                    ("queue", "configured_ftok"),
                    ("axis", "height")
                ],
            ),
            Some(1080.0)
        );
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
            .any(|metric| metric.name == names::GAME_SESSION_ACTIVE));
        assert!(!metrics
            .iter()
            .any(|metric| metric.name == names::GAME_FRAME_SAMPLES));
    }

    #[test]
    fn clean_metrics_filters_debug_only_fps_metadata() {
        let clean = MetricSample::gauge(
            names::GAME_FPS_SOURCE_SELECTED,
            "Selected.",
            labels(&[("source", GAMESCOPE_FRAME_SOURCE)]),
            1.0,
        );
        let debug = MetricSample::gauge(
            names::GAME_FPS_SOURCE_BACKEND_INFO,
            "Backend.",
            labels(&[
                ("source", GAMESCOPE_FRAME_SOURCE),
                ("queue", "configured_ftok"),
            ]),
            1.0,
        );

        assert_eq!(clean_metrics(vec![clean.clone(), debug]), vec![clean]);
    }
}
