//! Usage data shared by providers, the UI and status.json.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentId {
    Claude,
    Codex,
    Antigravity,
}

impl AgentId {
    pub const ALL: [AgentId; 3] = [AgentId::Claude, AgentId::Codex, AgentId::Antigravity];

    pub fn name(self) -> &'static str {
        match self {
            AgentId::Claude => "Claude",
            AgentId::Codex => "Codex",
            AgentId::Antigravity => "Antigravity",
        }
    }
}

/// One rate-limit window, e.g. Claude's 5-hour session or Codex's weekly limit.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Window {
    /// Stable id from the source: `five_hour`, `seven_day`, `primary`, a bucket id...
    pub id: String,
    /// Short label for the UI: `5h`, `7d`, `Opus 7d`, a model name...
    pub label: String,
    /// 0..=100, share of the window already used.
    pub used_percent: f64,
    pub resets_at: Option<Timestamp>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Usage {
    pub plan: Option<String>,
    /// Where the numbers came from, e.g. `oauth`, `claude-code-credentials`, `session-log`.
    pub source: &'static str,
    /// Ordered: the first two are what the tray icon draws.
    pub windows: Vec<Window>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Not fetched yet.
    Pending,
    Ok,
    /// Needs the user to sign in (or the agent's own sign-in expired).
    SignInRequired,
    /// No local data for this agent on this machine.
    NotFound,
    /// The agent's local server is not running.
    NotRunning,
    Error,
}

/// Why a provider could not produce usage.
#[derive(Clone, Debug, PartialEq)]
pub struct Failure {
    pub status: Status,
    pub message: String,
}

impl Failure {
    pub fn new(status: Status, message: impl Into<String>) -> Self {
        Failure { status, message: message.into() }
    }
    pub fn error(message: impl Into<String>) -> Self {
        Failure::new(Status::Error, message)
    }
}

/// What the app knows about one agent right now.
#[derive(Clone, Debug)]
pub struct AgentState {
    pub id: AgentId,
    pub status: Status,
    pub message: Option<String>,
    /// Last good usage; kept through transient errors so the bars do not blank.
    pub usage: Option<Usage>,
    pub fetched_at: Option<Timestamp>,
}

impl AgentState {
    pub fn new(id: AgentId) -> Self {
        AgentState { id, status: Status::Pending, message: None, usage: None, fetched_at: None }
    }

    pub fn apply(&mut self, result: Result<Usage, Failure>, now: Timestamp) {
        match result {
            Ok(usage) => {
                self.status = Status::Ok;
                self.message = None;
                self.usage = Some(usage);
                self.fetched_at = Some(now);
            }
            Err(f) => {
                // Keep stale numbers only for transient errors; a missing
                // sign-in or source means the old numbers no longer apply.
                if f.status != Status::Error {
                    self.usage = None;
                }
                self.status = f.status;
                self.message = Some(f.message);
            }
        }
    }
}

/// Clamps a percentage into 0..=100, mapping NaN to 0.
pub fn clamp_percent(p: f64) -> f64 {
    if p.is_nan() { 0.0 } else { p.clamp(0.0, 100.0) }
}

/// "2h 05m", "3d 4h", "12m"; `None` once the reset has passed.
pub fn format_until(reset: Timestamp, now: Timestamp) -> Option<String> {
    let secs = reset.as_second() - now.as_second();
    if secs <= 0 {
        return None;
    }
    let mins = (secs + 59) / 60;
    let (d, h, m) = (mins / 1440, (mins % 1440) / 60, mins % 60);
    Some(if d > 0 {
        format!("{d}d {h}h")
    } else if h > 0 {
        format!("{h}h {m:02}m")
    } else {
        format!("{m}m")
    })
}

/// Parses an RFC 3339 timestamp, epoch seconds or epoch milliseconds.
pub fn parse_time(v: &serde_json::Value) -> Option<Timestamp> {
    match v {
        serde_json::Value::String(s) => s
            .parse::<Timestamp>()
            .ok()
            .or_else(|| s.parse::<f64>().ok().and_then(epoch)),
        serde_json::Value::Number(n) => n.as_f64().and_then(epoch),
        _ => None,
    }
}

fn epoch(n: f64) -> Option<Timestamp> {
    // Values past year 33658 in seconds are milliseconds.
    let secs = if n > 1e12 { n / 1000.0 } else { n };
    Timestamp::from_second(secs as i64).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_durations() {
        let now: Timestamp = "2026-10-29T10:00:00Z".parse().unwrap();
        let at = |s: &str| s.parse::<Timestamp>().unwrap();
        assert_eq!(format_until(at("2026-10-29T10:12:00Z"), now).as_deref(), Some("12m"));
        assert_eq!(format_until(at("2026-10-29T12:05:00Z"), now).as_deref(), Some("2h 05m"));
        assert_eq!(format_until(at("2026-11-01T14:00:00Z"), now).as_deref(), Some("3d 4h"));
        assert_eq!(format_until(at("2026-10-29T09:00:00Z"), now), None);
    }

    #[test]
    fn parses_times() {
        let iso = parse_time(&serde_json::json!("2026-10-29T18:00:00.123456+00:00")).unwrap();
        assert_eq!(iso.as_second(), 1793296800);
        assert_eq!(parse_time(&serde_json::json!(1793296800)).unwrap(), Timestamp::from_second(1793296800).unwrap());
        assert_eq!(parse_time(&serde_json::json!(1793296800000u64)).unwrap().as_second(), 1793296800);
        assert_eq!(parse_time(&serde_json::json!("1793296800")).unwrap().as_second(), 1793296800);
        assert!(parse_time(&serde_json::json!(null)).is_none());
    }

    #[test]
    fn transient_errors_keep_usage() {
        let now = Timestamp::UNIX_EPOCH;
        let mut s = AgentState::new(AgentId::Claude);
        s.apply(Ok(Usage { source: "oauth", ..Default::default() }), now);
        s.apply(Err(Failure::error("HTTP 500")), now);
        assert!(s.usage.is_some());
        assert_eq!(s.status, Status::Error);
        s.apply(Err(Failure::new(Status::SignInRequired, "sign in")), now);
        assert!(s.usage.is_none());
    }
}
