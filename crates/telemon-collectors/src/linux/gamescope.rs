use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use serde::Serialize;

use telemon_core::config::SteamDeckGameStateConfig;

pub const STEAM_UI_APP_ID: u32 = 769;

const PROC_ROOT: &str = "/proc";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeckGameState {
    Idle,
    SteamUiOnly,
    GameRunningBackground,
    GameFocusedVisible,
    UnknownGameProcess,
}

impl DeckGameState {
    pub fn as_str(self) -> &'static str {
        match self {
            DeckGameState::Idle => "idle",
            DeckGameState::SteamUiOnly => "steam_ui_only",
            DeckGameState::GameRunningBackground => "game_running_background",
            DeckGameState::GameFocusedVisible => "game_focused_visible",
            DeckGameState::UnknownGameProcess => "unknown_game_process",
        }
    }

    pub fn is_game_running(self) -> bool {
        matches!(
            self,
            DeckGameState::GameRunningBackground | DeckGameState::GameFocusedVisible
        )
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct GamescopeFocusableWindow {
    pub window_id: u32,
    pub app_id: u32,
    pub pid: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct GamescopeFocusSnapshot {
    pub focused_app_id: Option<u32>,
    pub steam_focused: Option<bool>,
    pub focusable_app_ids: Vec<u32>,
    pub focused_pid: Option<u32>,
    pub focused_graphics_app_id: Option<u32>,
    pub focused_input_app_id: Option<u32>,
    pub focused_window_id: Option<u32>,
    pub steam_games_running: Option<u32>,
    pub focusable_windows: Vec<GamescopeFocusableWindow>,
    pub active_game_app_id: Option<u32>,
    pub active_game_pid: Option<u32>,
    pub detection_source: Option<String>,
}

impl GamescopeFocusSnapshot {
    pub fn classify(&self) -> DeckGameState {
        if let Some(app_id) = self.active_game_app_id {
            if !is_probable_game_app_id(app_id) {
                return DeckGameState::SteamUiOnly;
            }

            if self.active_game_pid.is_none() {
                return DeckGameState::UnknownGameProcess;
            }

            if self.focused_graphics_app_id == Some(app_id)
                || self.focused_input_app_id == Some(app_id)
                || self
                    .focused_window_id
                    .is_some_and(|window_id| self.window_has_game(window_id, app_id))
                || self.detection_source.as_deref() == Some("desktop_active_window")
            {
                return DeckGameState::GameFocusedVisible;
            }

            return DeckGameState::GameRunningBackground;
        }

        if self
            .focused_graphics_app_id
            .is_some_and(is_probable_game_app_id)
            || self
                .focused_input_app_id
                .is_some_and(is_probable_game_app_id)
            || self.focused_app_id.is_some_and(is_probable_game_app_id)
            || self
                .focusable_app_ids
                .iter()
                .any(|&id| is_probable_game_app_id(id))
        {
            return DeckGameState::UnknownGameProcess;
        }

        if self.focused_app_id == Some(STEAM_UI_APP_ID)
            || self.focused_input_app_id == Some(STEAM_UI_APP_ID)
            || self.focused_graphics_app_id == Some(STEAM_UI_APP_ID)
            || self.steam_focused == Some(true)
            || self.steam_games_running == Some(0)
        {
            return DeckGameState::SteamUiOnly;
        }

        if self.focused_pid.is_some() {
            return DeckGameState::UnknownGameProcess;
        }

        DeckGameState::Idle
    }

    fn derive_active_game<F>(&mut self, pid_exists: F)
    where
        F: Fn(u32) -> bool,
    {
        self.active_game_app_id = None;
        self.active_game_pid = None;
        self.detection_source = None;

        if let Some(window_id) = self.focused_window_id {
            if let Some(window) = self.focusable_windows.iter().find(|window| {
                window.window_id == window_id && is_probable_game_app_id(window.app_id)
            }) {
                self.active_game_app_id = Some(window.app_id);
                if window.pid != 0 && pid_exists(window.pid) {
                    self.active_game_pid = Some(window.pid);
                }
                self.detection_source = Some("gamescope_focused_window".to_string());
                return;
            }
        }

        if let Some(app_id) = self.focused_graphics_app_id {
            if is_probable_game_app_id(app_id) {
                self.active_game_app_id = Some(app_id);
                if let Some(pid) = self.pid_for_app_id(app_id, &pid_exists) {
                    self.active_game_pid = Some(pid);
                }
                self.detection_source = Some("gamescope_focused_app_gfx".to_string());
                return;
            }
        }

        if let Some(app_id) = self.focused_input_app_id {
            if is_probable_game_app_id(app_id) {
                self.active_game_app_id = Some(app_id);
                if let Some(pid) = self.pid_for_app_id(app_id, &pid_exists) {
                    self.active_game_pid = Some(pid);
                }
                self.detection_source = Some("gamescope_focused_app".to_string());
                return;
            }
        }

        if self.steam_games_running.unwrap_or_default() > 0 {
            if let Some(window) = self
                .focusable_windows
                .iter()
                .find(|window| is_probable_game_app_id(window.app_id) && pid_exists(window.pid))
            {
                self.active_game_app_id = Some(window.app_id);
                self.active_game_pid = Some(window.pid);
                self.detection_source = Some("gamescope_focusable_window".to_string());
                return;
            }
        }

        if let Some(app_id) = self
            .focusable_app_ids
            .iter()
            .copied()
            .find(|&id| is_probable_game_app_id(id))
        {
            self.active_game_app_id = Some(app_id);
            self.detection_source = Some("gamescope_focusable_app_legacy".to_string());
        }
    }

    fn has_gamescope_signal(&self) -> bool {
        self.focused_app_id.is_some()
            || self.focused_graphics_app_id.is_some()
            || self.focused_input_app_id.is_some()
            || self.focused_window_id.is_some()
            || self.steam_games_running.is_some()
            || !self.focusable_windows.is_empty()
            || !self.focusable_app_ids.is_empty()
            || self.focused_pid.is_some()
    }

    fn window_has_game(&self, window_id: u32, app_id: u32) -> bool {
        self.focusable_windows
            .iter()
            .any(|window| window.window_id == window_id && window.app_id == app_id)
    }

    fn pid_for_app_id<F>(&self, app_id: u32, pid_exists: F) -> Option<u32>
    where
        F: Fn(u32) -> bool,
    {
        self.focusable_windows
            .iter()
            .find(|window| window.app_id == app_id && window.pid != 0 && pid_exists(window.pid))
            .map(|window| window.pid)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct GamescopeDetection {
    pub enabled: bool,
    pub supported: bool,
    pub state: DeckGameState,
    pub snapshot: GamescopeFocusSnapshot,
    pub messages: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SteamDeckGameStateInspection {
    pub enabled: bool,
    pub xprop_path: String,
    pub display: String,
    pub supported: bool,
    pub state: DeckGameState,
    pub snapshot: GamescopeFocusSnapshot,
    pub messages: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct GamescopeDetector {
    config: SteamDeckGameStateConfig,
}

impl GamescopeDetector {
    pub fn new(config: SteamDeckGameStateConfig) -> Self {
        Self { config }
    }

    pub fn detect(&self) -> GamescopeDetection {
        detect_gamescope_state(&self.config)
    }
}

pub fn inspect_hardware(config: &SteamDeckGameStateConfig) -> SteamDeckGameStateInspection {
    let detection = detect_gamescope_state(config);
    SteamDeckGameStateInspection {
        enabled: config.enabled,
        xprop_path: config.xprop_path.clone(),
        display: config.display.clone(),
        supported: detection.supported,
        state: detection.state,
        snapshot: detection.snapshot,
        messages: detection.messages,
    }
}

pub fn is_probable_game_app_id(app_id: u32) -> bool {
    app_id != 0 && app_id != STEAM_UI_APP_ID
}

fn detect_gamescope_state(config: &SteamDeckGameStateConfig) -> GamescopeDetection {
    if !config.enabled {
        return GamescopeDetection {
            enabled: false,
            supported: false,
            state: DeckGameState::Idle,
            snapshot: GamescopeFocusSnapshot::default(),
            messages: vec!["disabled".to_string()],
        };
    }

    let mut messages = Vec::new();
    let initial_env = configured_xprop_environment(config);
    let mut snapshot = GamescopeFocusSnapshot::default();
    let mut supported = false;
    let mut used_env = initial_env.clone();

    match run_gamescope_xprop(config, &initial_env) {
        Ok(attempt) => {
            messages.extend(attempt.messages);
            supported = attempt.snapshot.has_gamescope_signal();
            snapshot = attempt.snapshot;
        }
        Err(error) => {
            messages.push(format!("failed to run xprop: {error}"));
        }
    }

    let should_retry_with_steam_env =
        config.auto_discover_steam_display && (!supported || config.display.trim().is_empty());
    if should_retry_with_steam_env {
        if let Some(steam_env) = discover_steam_xprop_environment(Path::new(PROC_ROOT)) {
            if steam_env != initial_env {
                match run_gamescope_xprop(config, &steam_env) {
                    Ok(attempt) => {
                        let retry_supported = attempt.snapshot.has_gamescope_signal();
                        if retry_supported || !supported {
                            snapshot = attempt.snapshot;
                            supported = retry_supported;
                            used_env = steam_env;
                        }
                        messages.extend(attempt.messages);
                    }
                    Err(error) => {
                        messages.push(format!("failed to run xprop with Steam display: {error}"));
                    }
                }
            }
        }
    }

    snapshot.derive_active_game(pid_exists);
    let mut state = snapshot.classify();

    if !state.is_game_running() && config.desktop_fallback_enabled {
        if let Some(candidate) =
            detect_desktop_active_window(config, &used_env, Path::new(PROC_ROOT))
        {
            apply_game_candidate(&mut snapshot, &candidate);
            supported = true;
            state = candidate.state;
            messages.push(format!(
                "game state detected by {}",
                candidate.detection_source
            ));
        }
    }

    if !state.is_game_running() && config.process_fallback_enabled {
        if let Some(candidate) = detect_steam_process_tree(Path::new(PROC_ROOT)) {
            apply_game_candidate(&mut snapshot, &candidate);
            supported = true;
            state = candidate.state;
            messages.push(format!(
                "game state detected by {}",
                candidate.detection_source
            ));
        }
    }

    GamescopeDetection {
        enabled: true,
        supported,
        state,
        snapshot,
        messages,
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct XpropEnvironment {
    display: Option<String>,
    xauthority: Option<String>,
}

#[derive(Debug)]
struct XpropAttempt {
    snapshot: GamescopeFocusSnapshot,
    messages: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GameCandidate {
    app_id: u32,
    pid: u32,
    state: DeckGameState,
    detection_source: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProcessInfo {
    pid: u32,
    ppid: u32,
    comm: String,
    cmdline: String,
}

fn configured_xprop_environment(config: &SteamDeckGameStateConfig) -> XpropEnvironment {
    XpropEnvironment {
        display: non_empty_string(&config.display),
        xauthority: None,
    }
}

fn run_gamescope_xprop(
    config: &SteamDeckGameStateConfig,
    env: &XpropEnvironment,
) -> std::io::Result<XpropAttempt> {
    let output = run_xprop(
        config,
        env,
        &[
            "-root",
            "GAMESCOPE_FOCUSED_APP_GFX",
            "GAMESCOPE_FOCUSED_APP",
            "GAMESCOPE_FOCUSED_WINDOW",
            "GAMESCOPE_FOCUSABLE_WINDOWS",
            "STEAM_GAMES_RUNNING",
            "GAMESCOPE_FOCUSABLE_APPS",
            "GAMESCOPE_FOCUSED_PID",
        ],
    )?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let snapshot = parse_xprop_output(&stdout);
    let mut messages = Vec::new();
    if !output.status.success() {
        messages.push(format!("xprop exited with status {}", output.status));
    }
    for line in stdout.lines().chain(stderr.lines()) {
        if line.contains("not found") || line.contains("no such atom") {
            messages.push(line.trim().to_string());
        }
    }
    if !stderr.trim().is_empty() && messages.is_empty() {
        messages.push(stderr.trim().to_string());
    }

    Ok(XpropAttempt { snapshot, messages })
}

fn run_xprop(
    config: &SteamDeckGameStateConfig,
    env: &XpropEnvironment,
    args: &[&str],
) -> std::io::Result<Output> {
    let mut command = Command::new(&config.xprop_path);
    command.args(args);
    if let Some(display) = &env.display {
        command.env("DISPLAY", display);
    }
    if let Some(xauthority) = &env.xauthority {
        command.env("XAUTHORITY", xauthority);
    }
    command.output()
}

fn discover_steam_xprop_environment(proc_root: &Path) -> Option<XpropEnvironment> {
    let current_uid = read_process_uid(&proc_root.join("self"))?;
    numeric_proc_dirs(proc_root)
        .into_iter()
        .filter_map(|pid| {
            let process_dir = proc_root.join(pid.to_string());
            let comm = read_trimmed(process_dir.join("comm")).ok()?;
            if comm.trim() != "steam" {
                return None;
            }
            if read_process_uid(&process_dir)? != current_uid {
                return None;
            }
            let env = read_process_environ_map(&process_dir).ok()?;
            let display = non_empty_string(env.get("DISPLAY")?)?;
            Some((
                pid,
                XpropEnvironment {
                    display: Some(display),
                    xauthority: env
                        .get("XAUTHORITY")
                        .and_then(|value| non_empty_string(value)),
                },
            ))
        })
        .max_by_key(|(pid, _)| *pid)
        .map(|(_, env)| env)
}

fn detect_desktop_active_window(
    config: &SteamDeckGameStateConfig,
    env: &XpropEnvironment,
    proc_root: &Path,
) -> Option<GameCandidate> {
    let active_output = run_xprop(config, env, &["-root", "_NET_ACTIVE_WINDOW"]).ok()?;
    if !active_output.status.success() {
        return None;
    }
    let active_stdout = String::from_utf8_lossy(&active_output.stdout);
    let window_id = parse_active_window_id(&active_stdout)?;
    if window_id == 0 {
        return None;
    }

    let window_arg = format!("0x{window_id:x}");
    let window_output = run_xprop(
        config,
        env,
        &[
            "-id",
            &window_arg,
            "_NET_WM_PID",
            "WM_CLASS",
            "_NET_WM_NAME",
            "WM_NAME",
        ],
    )
    .ok()?;
    if !window_output.status.success() {
        return None;
    }
    let window_stdout = String::from_utf8_lossy(&window_output.stdout);
    let pid = parse_wm_pid(&window_stdout)?;
    if !pid_exists_in(proc_root, pid) || is_ignored_game_process(proc_root, pid) {
        return None;
    }

    let process_dir = proc_root.join(pid.to_string());
    let app_id = steam_app_id_from_process_env(&process_dir)
        .or_else(|| steam_app_id_from_text(&window_stdout))?;
    if !is_probable_game_app_id(app_id) {
        return None;
    }

    Some(GameCandidate {
        app_id,
        pid,
        state: DeckGameState::GameFocusedVisible,
        detection_source: "desktop_active_window".to_string(),
    })
}

fn detect_steam_process_tree(proc_root: &Path) -> Option<GameCandidate> {
    let processes = read_process_infos(proc_root);
    find_steam_process_tree_candidate(&processes)
}

fn find_steam_process_tree_candidate(processes: &[ProcessInfo]) -> Option<GameCandidate> {
    let by_pid: HashMap<u32, &ProcessInfo> = processes
        .iter()
        .map(|process| (process.pid, process))
        .collect();
    let mut children_by_ppid: HashMap<u32, Vec<u32>> = HashMap::new();
    for process in processes {
        children_by_ppid
            .entry(process.ppid)
            .or_default()
            .push(process.pid);
    }

    processes
        .iter()
        .filter_map(|process| {
            let app_id = steam_launch_app_id(&process.cmdline)?;
            if !is_probable_game_app_id(app_id) {
                return None;
            }
            let descendant_pid =
                first_non_steam_descendant(process.pid, &by_pid, &children_by_ppid)?;
            Some(GameCandidate {
                app_id,
                pid: descendant_pid,
                state: DeckGameState::GameRunningBackground,
                detection_source: "steam_reaper_process".to_string(),
            })
        })
        .next()
}

fn first_non_steam_descendant(
    root_pid: u32,
    by_pid: &HashMap<u32, &ProcessInfo>,
    children_by_ppid: &HashMap<u32, Vec<u32>>,
) -> Option<u32> {
    let mut seen = HashSet::new();
    let mut queue = VecDeque::new();
    queue.extend(
        children_by_ppid
            .get(&root_pid)
            .into_iter()
            .flatten()
            .copied(),
    );
    while let Some(pid) = queue.pop_front() {
        if !seen.insert(pid) {
            continue;
        }
        let Some(process) = by_pid.get(&pid) else {
            continue;
        };
        if !is_ignored_game_process_text(&process.comm, &process.cmdline) {
            return Some(pid);
        }
        queue.extend(children_by_ppid.get(&pid).into_iter().flatten().copied());
    }
    None
}

fn apply_game_candidate(snapshot: &mut GamescopeFocusSnapshot, candidate: &GameCandidate) {
    snapshot.active_game_app_id = Some(candidate.app_id);
    snapshot.active_game_pid = Some(candidate.pid);
    snapshot.detection_source = Some(candidate.detection_source.clone());
}

fn parse_xprop_output(output: &str) -> GamescopeFocusSnapshot {
    let mut snapshot = GamescopeFocusSnapshot::default();
    for line in output.lines() {
        if line.starts_with("GAMESCOPE_FOCUSED_APP_GFX") {
            snapshot.focused_graphics_app_id = first_u32_after_equals(line);
        } else if line.starts_with("GAMESCOPE_FOCUSED_APP") {
            snapshot.focused_input_app_id = first_u32_after_equals(line);
        } else if line.starts_with("GAMESCOPE_FOCUSED_WINDOW") {
            snapshot.focused_window_id = first_u32_after_equals(line);
        } else if line.starts_with("GAMESCOPE_FOCUSABLE_WINDOWS") {
            snapshot.focusable_windows = parse_focusable_windows(line);
            snapshot.focusable_app_ids = snapshot
                .focusable_windows
                .iter()
                .map(|window| window.app_id)
                .collect();
        } else if line.starts_with("STEAM_GAMES_RUNNING") {
            snapshot.steam_games_running = first_u32_after_equals(line);
        } else if line.starts_with("GAMESCOPE_FOCUSABLE_APPS") {
            snapshot.focusable_app_ids = all_u32_after_equals(line);
        } else if line.starts_with("GAMESCOPE_FOCUSED_PID") {
            snapshot.focused_pid = first_u32_after_equals(line);
        }
    }
    snapshot.focused_app_id = snapshot
        .focused_input_app_id
        .or(snapshot.focused_graphics_app_id);
    snapshot.steam_focused = snapshot
        .focused_input_app_id
        .or(snapshot.focused_app_id)
        .map(|app_id| app_id == STEAM_UI_APP_ID);
    snapshot
}

fn parse_focusable_windows(line: &str) -> Vec<GamescopeFocusableWindow> {
    all_u32_after_equals(line)
        .chunks_exact(3)
        .map(|chunk| GamescopeFocusableWindow {
            window_id: chunk[0],
            app_id: chunk[1],
            pid: chunk[2],
        })
        .collect()
}

fn parse_active_window_id(output: &str) -> Option<u32> {
    output
        .lines()
        .find(|line| line.starts_with("_NET_ACTIVE_WINDOW"))
        .and_then(|line| line.split_once('#').map(|(_, rhs)| rhs.trim()))
        .and_then(parse_window_id)
}

fn parse_window_id(value: &str) -> Option<u32> {
    let token = value
        .split(|c: char| c.is_whitespace() || c == ',')
        .find(|token| !token.is_empty())?;
    if let Some(hex) = token.strip_prefix("0x") {
        u32::from_str_radix(hex, 16).ok()
    } else {
        token.parse::<u32>().ok()
    }
}

fn parse_wm_pid(output: &str) -> Option<u32> {
    output
        .lines()
        .find(|line| line.starts_with("_NET_WM_PID"))
        .and_then(first_u32_after_equals)
}

fn first_u32_after_equals(line: &str) -> Option<u32> {
    all_u32_after_equals(line).into_iter().next()
}

fn all_u32_after_equals(line: &str) -> Vec<u32> {
    let Some((_, rhs)) = line.split_once('=') else {
        return Vec::new();
    };
    rhs.split(|c: char| !(c.is_ascii_digit()))
        .filter(|value| !value.is_empty())
        .filter_map(|value| value.parse::<u32>().ok())
        .collect()
}

fn steam_app_id_from_process_env(process_dir: &Path) -> Option<u32> {
    let env = read_process_environ_map(process_dir).ok()?;
    ["SteamAppId", "SteamGameId"]
        .iter()
        .filter_map(|key| env.get(*key))
        .filter_map(|value| value.parse::<u32>().ok())
        .find(|&app_id| is_probable_game_app_id(app_id))
}

fn steam_app_id_from_text(value: &str) -> Option<u32> {
    let needle = "steam_app_";
    value
        .match_indices(needle)
        .filter_map(|(index, _)| {
            let start = index + needle.len();
            value[start..]
                .chars()
                .take_while(|character| character.is_ascii_digit())
                .collect::<String>()
                .parse::<u32>()
                .ok()
        })
        .find(|&app_id| is_probable_game_app_id(app_id))
}

fn steam_launch_app_id(cmdline: &str) -> Option<u32> {
    if !cmdline.contains("SteamLaunch") {
        return None;
    }
    let index = cmdline.find("AppId=")? + "AppId=".len();
    cmdline[index..]
        .chars()
        .take_while(|character| character.is_ascii_digit())
        .collect::<String>()
        .parse::<u32>()
        .ok()
}

fn pid_exists(pid: u32) -> bool {
    pid_exists_in(Path::new(PROC_ROOT), pid)
}

fn pid_exists_in(proc_root: &Path, pid: u32) -> bool {
    pid != 0 && proc_root.join(pid.to_string()).exists()
}

fn is_ignored_game_process(proc_root: &Path, pid: u32) -> bool {
    let process_dir = proc_root.join(pid.to_string());
    let comm = read_trimmed(process_dir.join("comm")).unwrap_or_default();
    let cmdline = read_cmdline(process_dir.join("cmdline")).unwrap_or_default();
    is_ignored_game_process_text(&comm, &cmdline)
}

fn is_ignored_game_process_text(comm: &str, cmdline: &str) -> bool {
    let comm = comm.trim().to_ascii_lowercase();
    let executable = cmdline
        .split_whitespace()
        .next()
        .and_then(|value| Path::new(value).file_name())
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let name = if comm.is_empty() { &executable } else { &comm };
    matches!(
        name.as_str(),
        "steam"
            | "steamwebhelper"
            | "vrwebhelper"
            | "gamescope"
            | "reaper"
            | "kwin"
            | "kwin_x11"
            | "kwin_wayland"
            | "plasmashell"
            | "steam-runtime-supervisor"
    ) || name.starts_with("steamwebhelper")
        || name.starts_with("steam-runtime")
}

fn read_process_infos(proc_root: &Path) -> Vec<ProcessInfo> {
    numeric_proc_dirs(proc_root)
        .into_iter()
        .filter_map(|pid| {
            let process_dir = proc_root.join(pid.to_string());
            Some(ProcessInfo {
                pid,
                ppid: read_process_ppid(&process_dir)?,
                comm: read_trimmed(process_dir.join("comm")).unwrap_or_default(),
                cmdline: read_cmdline(process_dir.join("cmdline")).unwrap_or_default(),
            })
        })
        .collect()
}

fn read_process_ppid(process_dir: &Path) -> Option<u32> {
    let stat = fs::read_to_string(process_dir.join("stat")).ok()?;
    let close = stat.rfind(") ")?;
    let fields: Vec<_> = stat[close + 2..].split_whitespace().collect();
    fields.get(1)?.parse::<u32>().ok()
}

fn read_process_uid(process_dir: &Path) -> Option<u32> {
    let status = fs::read_to_string(process_dir.join("status")).ok()?;
    status
        .lines()
        .find(|line| line.starts_with("Uid:"))
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse::<u32>().ok())
}

fn read_process_environ_map(process_dir: &Path) -> std::io::Result<HashMap<String, String>> {
    let bytes = fs::read(process_dir.join("environ"))?;
    Ok(bytes
        .split(|byte| *byte == 0)
        .filter_map(|entry| std::str::from_utf8(entry).ok())
        .filter_map(|entry| entry.split_once('='))
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect())
}

fn read_cmdline(path: PathBuf) -> std::io::Result<String> {
    let bytes = fs::read(path)?;
    let value = bytes
        .split(|byte| *byte == 0)
        .filter_map(|entry| std::str::from_utf8(entry).ok())
        .filter(|entry| !entry.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    Ok(value)
}

fn read_trimmed(path: PathBuf) -> std::io::Result<String> {
    fs::read_to_string(path).map(|value| value.trim().to_string())
}

fn numeric_proc_dirs(proc_root: &Path) -> Vec<u32> {
    fs::read_dir(proc_root)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| entry.file_name().to_str()?.parse::<u32>().ok())
        .collect()
}

fn non_empty_string(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focused_non_steam_app_without_pid_is_unknown() {
        let mut snapshot = GamescopeFocusSnapshot {
            focused_graphics_app_id: Some(1234),
            focused_app_id: Some(1234),
            ..Default::default()
        };
        snapshot.derive_active_game(|_| false);
        assert_eq!(snapshot.classify(), DeckGameState::UnknownGameProcess);
        assert!(!snapshot.classify().is_game_running());
    }

    #[test]
    fn steam_focused_with_focusable_game_pid_is_background_game() {
        let mut snapshot = GamescopeFocusSnapshot {
            focused_input_app_id: Some(STEAM_UI_APP_ID),
            focused_app_id: Some(STEAM_UI_APP_ID),
            steam_games_running: Some(1),
            focusable_windows: vec![
                GamescopeFocusableWindow {
                    window_id: 10,
                    app_id: STEAM_UI_APP_ID,
                    pid: 20,
                },
                GamescopeFocusableWindow {
                    window_id: 11,
                    app_id: 1234,
                    pid: 30,
                },
            ],
            ..Default::default()
        };
        snapshot.derive_active_game(|pid| pid == 30);
        assert_eq!(snapshot.classify(), DeckGameState::GameRunningBackground);
        assert_eq!(snapshot.active_game_app_id, Some(1234));
        assert_eq!(snapshot.active_game_pid, Some(30));
    }

    #[test]
    fn steam_ui_only_is_not_game_running() {
        let mut snapshot = GamescopeFocusSnapshot {
            focused_graphics_app_id: Some(STEAM_UI_APP_ID),
            focused_input_app_id: Some(STEAM_UI_APP_ID),
            focused_app_id: Some(STEAM_UI_APP_ID),
            steam_games_running: Some(0),
            focusable_app_ids: vec![STEAM_UI_APP_ID],
            ..Default::default()
        };
        snapshot.derive_active_game(|_| false);
        assert_eq!(snapshot.classify(), DeckGameState::SteamUiOnly);
        assert!(!snapshot.classify().is_game_running());
    }

    #[test]
    fn parses_gamescope_focusable_window_triples() {
        let mut snapshot = parse_xprop_output(
            "GAMESCOPE_FOCUSED_APP_GFX(CARDINAL) = 1234\n\
             GAMESCOPE_FOCUSED_APP(CARDINAL) = 769\n\
             GAMESCOPE_FOCUSED_WINDOW(CARDINAL) = 41943067\n\
             GAMESCOPE_FOCUSABLE_WINDOWS(CARDINAL) = 41943067, 1234, 18342, 41943070, 769, 1540\n\
             STEAM_GAMES_RUNNING(CARDINAL) = 1\n",
        );
        snapshot.derive_active_game(|pid| pid == 18342);
        assert_eq!(snapshot.focused_graphics_app_id, Some(1234));
        assert_eq!(snapshot.focused_input_app_id, Some(769));
        assert_eq!(snapshot.focused_window_id, Some(41943067));
        assert_eq!(snapshot.focusable_windows.len(), 2);
        assert_eq!(snapshot.active_game_app_id, Some(1234));
        assert_eq!(snapshot.active_game_pid, Some(18342));
        assert_eq!(snapshot.classify(), DeckGameState::GameFocusedVisible);
    }

    #[test]
    fn overlay_focus_keeps_graphics_game_visible() {
        let mut snapshot = parse_xprop_output(
            "GAMESCOPE_FOCUSED_APP_GFX(CARDINAL) = 1245620\n\
             GAMESCOPE_FOCUSED_APP(CARDINAL) = 769\n\
             GAMESCOPE_FOCUSABLE_WINDOWS(CARDINAL) = 99, 1245620, 2000, 100, 769, 3000\n\
             STEAM_GAMES_RUNNING(CARDINAL) = 1\n",
        );
        snapshot.derive_active_game(|pid| pid == 2000);
        assert_eq!(snapshot.active_game_app_id, Some(1245620));
        assert_eq!(snapshot.active_game_pid, Some(2000));
        assert_eq!(snapshot.classify(), DeckGameState::GameFocusedVisible);
    }

    #[test]
    fn legacy_focusable_apps_without_pid_are_unknown() {
        let mut snapshot = parse_xprop_output(
            "GAMESCOPE_FOCUSED_APP(CARDINAL) = 769\nGAMESCOPE_FOCUSABLE_APPS(CARDINAL) = 769, 1234\n",
        );
        snapshot.derive_active_game(|_| false);
        assert_eq!(snapshot.focused_app_id, Some(769));
        assert_eq!(snapshot.focusable_app_ids, vec![769, 1234]);
        assert_eq!(snapshot.classify(), DeckGameState::UnknownGameProcess);
    }

    #[test]
    fn parses_desktop_active_window_id_and_pid() {
        assert_eq!(
            parse_active_window_id("_NET_ACTIVE_WINDOW(WINDOW): window id # 0x3e00007\n"),
            Some(0x3e00007)
        );
        assert_eq!(parse_wm_pid("_NET_WM_PID(CARDINAL) = 12345\n"), Some(12345));
    }

    #[test]
    fn parses_steam_app_ids() {
        assert_eq!(
            steam_app_id_from_text(r#"WM_CLASS(STRING) = "steam_app_1245620""#),
            Some(1245620)
        );
        assert_eq!(
            steam_launch_app_id("/usr/bin/reaper SteamLaunch AppId=730 -- /game"),
            Some(730)
        );
    }

    #[test]
    fn steam_process_tree_requires_non_steam_descendant() {
        let processes = vec![
            ProcessInfo {
                pid: 10,
                ppid: 1,
                comm: "reaper".to_string(),
                cmdline: "/usr/bin/reaper SteamLaunch AppId=730 --".to_string(),
            },
            ProcessInfo {
                pid: 11,
                ppid: 10,
                comm: "steamwebhelper".to_string(),
                cmdline: "steamwebhelper".to_string(),
            },
            ProcessInfo {
                pid: 12,
                ppid: 11,
                comm: "proton".to_string(),
                cmdline: "proton waitforexitandrun game.exe".to_string(),
            },
        ];
        let candidate = find_steam_process_tree_candidate(&processes).unwrap();
        assert_eq!(candidate.app_id, 730);
        assert_eq!(candidate.pid, 12);
        assert_eq!(candidate.state, DeckGameState::GameRunningBackground);
    }
}
