//! `config.json`: which agents are shown and how often to refresh.
//! Written as UTF-8 without a BOM; unknown or missing fields fall back to
//! defaults so a hand-edited file never stops the app.

use crate::model::AgentId;
use serde::{Deserialize, Serialize};

pub const REFRESH_CHOICES: [u32; 5] = [5, 10, 15, 30, 60];
const MIN_REFRESH: u32 = 5;
const MAX_REFRESH: u32 = 240;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Minutes between refreshes.
    pub refresh_minutes: u32,
    /// Agents drawn in the tray and fetched. Hidden agents are not fetched.
    pub shown: Vec<AgentId>,
    /// Skip network refreshes while a full-screen app (game, video) is in front.
    pub pause_when_fullscreen: bool,
    /// Show the shared GitHub contributions section (needs the GitHub CLI).
    pub show_github: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config { refresh_minutes: 10, shown: vec![AgentId::Claude], pause_when_fullscreen: true, show_github: true }
    }
}

impl Config {
    pub fn load() -> Config {
        std::fs::read(crate::paths::config_file())
            .ok()
            .map(|b| Self::parse(&b))
            .unwrap_or_default()
    }

    pub fn parse(bytes: &[u8]) -> Config {
        let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
        let mut c: Config = serde_json::from_slice(bytes).unwrap_or_default();
        c.refresh_minutes = c.refresh_minutes.clamp(MIN_REFRESH, MAX_REFRESH);
        let mut seen = Vec::new();
        c.shown.retain(|a| {
            let new = !seen.contains(a);
            seen.push(*a);
            new
        });
        c
    }

    pub fn save(&self) {
        if let Ok(json) = serde_json::to_vec_pretty(self)
            && let Err(e) = crate::paths::write_atomic(&crate::paths::config_file(), &json) {
                crate::log::warn(&format!("saving config failed: {e}"));
            }
    }

    pub fn is_shown(&self, id: AgentId) -> bool {
        self.shown.contains(&id)
    }

    pub fn toggle(&mut self, id: AgentId) {
        if self.is_shown(id) {
            self.shown.retain(|a| *a != id);
        } else {
            self.shown.push(id);
            // Keep the tray order stable: Claude, Codex, Antigravity.
            self.shown.sort_by_key(|a| AgentId::ALL.iter().position(|b| b == a));
        }
    }

    /// The next refresh choice after the current one, wrapping around.
    pub fn next_refresh(&self) -> u32 {
        REFRESH_CHOICES
            .iter()
            .copied()
            .find(|m| *m > self.refresh_minutes)
            .unwrap_or(REFRESH_CHOICES[0])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_clamping() {
        assert_eq!(Config::parse(b"not json"), Config::default());
        let c = Config::parse(b"\xEF\xBB\xBF{\"refresh_minutes\": 1, \"shown\": [\"codex\", \"codex\"]}");
        assert_eq!(c.refresh_minutes, 5);
        assert_eq!(c.shown, vec![AgentId::Codex]);
        assert!(c.pause_when_fullscreen);
    }

    #[test]
    fn toggles_keep_order() {
        let mut c = Config { shown: vec![AgentId::Antigravity], ..Default::default() };
        c.toggle(AgentId::Claude);
        assert_eq!(c.shown, vec![AgentId::Claude, AgentId::Antigravity]);
        c.toggle(AgentId::Antigravity);
        assert_eq!(c.shown, vec![AgentId::Claude]);
    }

    #[test]
    fn refresh_cycles() {
        let mut c = Config::default();
        assert_eq!(c.next_refresh(), 15);
        c.refresh_minutes = 60;
        assert_eq!(c.next_refresh(), 5);
    }
}
