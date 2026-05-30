use std::time::{Duration, Instant};

use telemon_collectors::linux::gamescope::{DeckGameState, GamescopeDetector};
use telemon_core::config::{AppConfig, SteamDeckGameStateConfig};

use crate::scheduler::SamplingOverride;

pub fn sampling_override(config: &AppConfig) -> Option<Box<dyn SamplingOverride>> {
    config.collectors.steam_deck_game_state.enabled.then(|| {
        Box::new(SteamDeckGameStateSamplingOverride::new(
            config.collectors.steam_deck_game_state.clone(),
        )) as Box<dyn SamplingOverride>
    })
}

pub struct SteamDeckGameStateSamplingOverride {
    config: SteamDeckGameStateConfig,
    detector: GamescopeDetector,
    last_poll_at: Option<Instant>,
    last_state: DeckGameState,
    force_until: Option<Instant>,
}

impl SteamDeckGameStateSamplingOverride {
    pub fn new(config: SteamDeckGameStateConfig) -> Self {
        Self {
            detector: GamescopeDetector::new(config.clone()),
            config,
            last_poll_at: None,
            last_state: DeckGameState::Idle,
            force_until: None,
        }
    }

    fn update_state_if_due(&mut self, now: Instant) {
        let poll_interval = Duration::from_secs(self.config.poll_interval_seconds);
        if self
            .last_poll_at
            .is_some_and(|last_poll_at| now.duration_since(last_poll_at) < poll_interval)
        {
            return;
        }

        self.last_poll_at = Some(now);
        let detection = self.detector.detect();
        self.last_state = detection.state;
        if detection.state.is_game_running() {
            self.force_until = Some(now + Duration::from_secs(self.config.stop_debounce_seconds));
        }
    }

    fn forced_interval_seconds_at(&mut self, now: Instant) -> Option<u64> {
        self.update_state_if_due(now);
        if self.last_state.is_game_running() {
            return Some(1);
        }
        if self
            .force_until
            .is_some_and(|force_until| now < force_until)
        {
            return Some(1);
        }
        None
    }
}

impl SamplingOverride for SteamDeckGameStateSamplingOverride {
    fn forced_interval_seconds(&mut self) -> Option<u64> {
        self.forced_interval_seconds_at(Instant::now())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_config_does_not_create_override() {
        let mut config = AppConfig::default();
        config.collectors.steam_deck_game_state.enabled = false;

        assert!(sampling_override(&config).is_none());
    }
}
