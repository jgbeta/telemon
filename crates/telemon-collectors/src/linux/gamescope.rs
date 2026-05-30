use std::process::Command;

use serde::Serialize;

use telemon_core::config::SteamDeckGameStateConfig;

pub const STEAM_UI_APP_ID: u32 = 769;

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
pub struct GamescopeFocusSnapshot {
    pub focused_app_id: Option<u32>,
    pub steam_focused: Option<bool>,
    pub focusable_app_ids: Vec<u32>,
    pub focused_pid: Option<u32>,
}

impl GamescopeFocusSnapshot {
    pub fn classify(&self) -> DeckGameState {
        if let Some(app_id) = self.focused_app_id {
            if is_probable_game_app_id(app_id) {
                return DeckGameState::GameFocusedVisible;
            }

            if app_id == STEAM_UI_APP_ID || self.steam_focused == Some(true) {
                if self
                    .focusable_app_ids
                    .iter()
                    .any(|&id| is_probable_game_app_id(id))
                {
                    return DeckGameState::GameRunningBackground;
                }
                return DeckGameState::SteamUiOnly;
            }
        }

        if self
            .focusable_app_ids
            .iter()
            .any(|&id| is_probable_game_app_id(id))
        {
            return DeckGameState::GameRunningBackground;
        }

        if self.focused_pid.is_some() {
            return DeckGameState::UnknownGameProcess;
        }

        DeckGameState::Idle
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

    let mut command = Command::new(&config.xprop_path);
    command.args([
        "-root",
        "GAMESCOPE_FOCUSED_APP",
        "GAMESCOPE_FOCUSED_APP_GFX",
        "GAMESCOPE_FOCUSABLE_APPS",
        "GAMESCOPE_FOCUSED_PID",
    ]);
    if !config.display.trim().is_empty() {
        command.env("DISPLAY", &config.display);
    }

    let output = match command.output() {
        Ok(output) => output,
        Err(error) => {
            return GamescopeDetection {
                enabled: true,
                supported: false,
                state: DeckGameState::Idle,
                snapshot: GamescopeFocusSnapshot::default(),
                messages: vec![format!("failed to run xprop: {error}")],
            };
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let snapshot = parse_xprop_output(&stdout);
    let state = snapshot.classify();
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

    let supported = snapshot.focused_app_id.is_some()
        || !snapshot.focusable_app_ids.is_empty()
        || snapshot.focused_pid.is_some();

    GamescopeDetection {
        enabled: true,
        supported,
        state,
        snapshot,
        messages,
    }
}

fn parse_xprop_output(output: &str) -> GamescopeFocusSnapshot {
    let mut snapshot = GamescopeFocusSnapshot::default();
    for line in output.lines() {
        if line.starts_with("GAMESCOPE_FOCUSED_APP_GFX") {
            if snapshot.focused_app_id.is_none() {
                snapshot.focused_app_id = first_u32_after_equals(line);
            }
        } else if line.starts_with("GAMESCOPE_FOCUSED_APP") {
            snapshot.focused_app_id = first_u32_after_equals(line);
        } else if line.starts_with("GAMESCOPE_FOCUSABLE_APPS") {
            snapshot.focusable_app_ids = all_u32_after_equals(line);
        } else if line.starts_with("GAMESCOPE_FOCUSED_PID") {
            snapshot.focused_pid = first_u32_after_equals(line);
        }
    }
    snapshot
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focused_non_steam_app_is_game_visible() {
        let snapshot = GamescopeFocusSnapshot {
            focused_app_id: Some(1234),
            ..Default::default()
        };
        assert_eq!(snapshot.classify(), DeckGameState::GameFocusedVisible);
    }

    #[test]
    fn steam_focused_with_focusable_game_is_background_game() {
        let snapshot = GamescopeFocusSnapshot {
            focused_app_id: Some(STEAM_UI_APP_ID),
            focusable_app_ids: vec![STEAM_UI_APP_ID, 1234],
            ..Default::default()
        };
        assert_eq!(snapshot.classify(), DeckGameState::GameRunningBackground);
    }

    #[test]
    fn steam_ui_only_is_not_game_running() {
        let snapshot = GamescopeFocusSnapshot {
            focused_app_id: Some(STEAM_UI_APP_ID),
            focusable_app_ids: vec![STEAM_UI_APP_ID],
            ..Default::default()
        };
        assert_eq!(snapshot.classify(), DeckGameState::SteamUiOnly);
        assert!(!snapshot.classify().is_game_running());
    }

    #[test]
    fn parses_xprop_output() {
        let snapshot = parse_xprop_output(
            "GAMESCOPE_FOCUSED_APP(CARDINAL) = 769\nGAMESCOPE_FOCUSABLE_APPS(CARDINAL) = 769, 1234\n",
        );
        assert_eq!(snapshot.focused_app_id, Some(769));
        assert_eq!(snapshot.focusable_app_ids, vec![769, 1234]);
        assert_eq!(snapshot.classify(), DeckGameState::GameRunningBackground);
    }
}
