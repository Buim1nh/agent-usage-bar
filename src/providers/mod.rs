//! One provider per agent. Providers run on the worker thread and may block.

pub mod antigravity;
pub mod claude;
pub mod codex;

use crate::model::{AgentId, Failure, Usage};

pub trait Provider: Send {
    fn id(&self) -> AgentId;
    /// True when fetching contacts a remote server; those refreshes are
    /// skipped while a full-screen app is in front.
    fn uses_network(&self) -> bool;
    fn fetch(&mut self) -> Result<Usage, Failure>;
}

pub fn all() -> Vec<Box<dyn Provider>> {
    vec![
        Box::new(claude::ClaudeProvider::new()),
        Box::new(codex::CodexProvider::new()),
        Box::new(antigravity::AntigravityProvider::new()),
    ]
}

/// "5h", "7d", "36h", "2d" from a window length in minutes.
pub fn window_label(minutes: f64) -> String {
    let m = minutes.round() as i64;
    if m <= 0 {
        return "?".into();
    }
    // Sources report 10079 or 299 for 7 days and 5 hours; round to the unit.
    let days = (m + 720) / 1440;
    if days >= 1 && (days * 1440 - m).abs() <= 60 {
        format!("{days}d")
    } else if m >= 60 {
        format!("{}h", (m + 30) / 60)
    } else {
        format!("{m}m")
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn labels() {
        assert_eq!(super::window_label(300.0), "5h");
        assert_eq!(super::window_label(299.0), "5h");
        assert_eq!(super::window_label(10080.0), "7d");
        assert_eq!(super::window_label(10079.0), "7d");
        assert_eq!(super::window_label(1440.0), "1d");
        assert_eq!(super::window_label(45.0), "45m");
    }
}
