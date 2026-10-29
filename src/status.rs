//! `%LOCALAPPDATA%\agent-usage-bar\status.json`, the file other programs
//! (wdock's top bar) read. The schema is documented in docs/status-json.md;
//! bump `SCHEMA` on any breaking change.

use crate::config::Config;
use crate::github::GithubState;
use crate::model::{AgentId, AgentState, Status, Window};
use jiff::Timestamp;
use serde::Serialize;

pub const SCHEMA: u32 = 1;

#[derive(Serialize)]
pub struct StatusFile<'a> {
    pub schema: u32,
    pub updated_at: Timestamp,
    pub refresh_minutes: u32,
    pub paused: bool,
    pub agents: Vec<AgentEntry<'a>>,
    pub github: GithubEntry<'a>,
}

#[derive(Serialize)]
pub struct GithubEntry<'a> {
    pub shown: bool,
    pub status: Status,
    pub message: Option<&'a str>,
    pub fetched_at: Option<Timestamp>,
    pub login: Option<&'a str>,
    pub total: Option<u32>,
    pub days: &'a [crate::github::Day],
}

#[derive(Serialize)]
pub struct AgentEntry<'a> {
    pub id: AgentId,
    pub name: &'static str,
    pub shown: bool,
    pub status: Status,
    pub message: Option<&'a str>,
    pub plan: Option<&'a str>,
    pub source: Option<&'static str>,
    pub fetched_at: Option<Timestamp>,
    pub windows: &'a [Window],
}

pub fn build<'a>(states: &'a [AgentState], github: &'a GithubState, config: &Config, paused: bool, now: Timestamp) -> StatusFile<'a> {
    StatusFile {
        schema: SCHEMA,
        updated_at: now,
        refresh_minutes: config.refresh_minutes,
        paused,
        agents: states
            .iter()
            .map(|s| AgentEntry {
                id: s.id,
                name: s.id.name(),
                shown: config.is_shown(s.id),
                status: s.status,
                message: s.message.as_deref(),
                plan: s.usage.as_ref().and_then(|u| u.plan.as_deref()),
                source: s.usage.as_ref().map(|u| u.source),
                fetched_at: s.fetched_at,
                windows: s.usage.as_ref().map(|u| u.windows.as_slice()).unwrap_or(&[]),
            })
            .collect(),
        github: GithubEntry {
            shown: config.show_github,
            status: github.status,
            message: github.message.as_deref(),
            fetched_at: github.fetched_at,
            login: github.data.as_ref().map(|c| c.login.as_str()),
            total: github.data.as_ref().map(|c| c.total),
            days: github.data.as_ref().map(|c| c.days.as_slice()).unwrap_or(&[]),
        },
    }
}

pub fn write(states: &[AgentState], github: &GithubState, config: &Config, paused: bool) {
    let file = build(states, github, config, paused, Timestamp::now());
    match serde_json::to_vec_pretty(&file) {
        Ok(json) => {
            if let Err(e) = crate::paths::write_atomic(&crate::paths::status_file(), &json) {
                crate::log::warn(&format!("writing status.json failed: {e}"));
            }
        }
        Err(e) => crate::log::warn(&format!("serializing status failed: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Usage;

    #[test]
    fn serializes_documented_shape() {
        let now: Timestamp = "2026-10-29T10:00:00Z".parse().unwrap();
        let mut claude = AgentState::new(AgentId::Claude);
        claude.apply(
            Ok(Usage {
                plan: Some("pro".into()),
                source: "oauth",
                windows: vec![Window {
                    id: "five_hour".into(),
                    label: "5h".into(),
                    used_percent: 42.5,
                    resets_at: Some("2026-10-29T12:00:00Z".parse().unwrap()),
                }],
            }),
            now,
        );
        let codex = AgentState::new(AgentId::Codex);
        let states = [claude, codex];
        let v = serde_json::to_value(build(&states, &GithubState::default(), &Config::default(), false, now)).unwrap();
        assert_eq!(v["schema"], 1);
        assert_eq!(v["updated_at"], "2026-10-29T10:00:00Z");
        let a = &v["agents"][0];
        assert_eq!(a["id"], "claude");
        assert_eq!(a["shown"], true);
        assert_eq!(a["status"], "ok");
        assert_eq!(a["plan"], "pro");
        assert_eq!(a["windows"][0]["label"], "5h");
        assert_eq!(a["windows"][0]["used_percent"], 42.5);
        assert_eq!(a["windows"][0]["resets_at"], "2026-10-29T12:00:00Z");
        assert_eq!(v["agents"][1]["status"], "pending");
        assert_eq!(v["agents"][1]["shown"], false);
        assert_eq!(v["agents"][1]["windows"], serde_json::json!([]));
        assert_eq!(v["github"]["shown"], true);
        assert_eq!(v["github"]["status"], "pending");
    }
}
