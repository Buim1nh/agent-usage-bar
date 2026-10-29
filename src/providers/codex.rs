//! Codex: the rate limits Codex CLI records in its own session logs.
//!
//! Every turn, Codex appends a `token_count` event with the server's
//! `rate_limits` (primary ≈ 5 hours, secondary ≈ weekly) to
//! `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl` (or `$CODEX_HOME`). We read
//! the newest one; no network and no credentials. The numbers are as fresh
//! as the last Codex turn, so a window whose reset has passed reads 0%.
//! Same approach as xiangz19/codex-ratelimit.

use crate::model::{AgentId, Failure, Status, Usage, Window, clamp_percent, parse_time};
use crate::providers::{Provider, window_label};
use jiff::{SignedDuration, Timestamp};
use serde_json::Value;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// Only the tail of a big session file is read.
const TAIL_BYTES: u64 = 4 * 1024 * 1024;
/// Newest session files to look into before giving up.
const MAX_FILES: usize = 8;

pub struct CodexProvider;

impl CodexProvider {
    pub fn new() -> Self {
        CodexProvider
    }
}

impl Provider for CodexProvider {
    fn id(&self) -> AgentId {
        AgentId::Codex
    }

    fn uses_network(&self) -> bool {
        false
    }

    fn fetch(&mut self) -> Result<Usage, Failure> {
        let sessions = codex_home().join("sessions");
        if !sessions.is_dir() {
            return Err(Failure::new(Status::NotFound, "No Codex sessions on this machine"));
        }
        let now = Timestamp::now();
        for file in newest_session_files(&sessions, MAX_FILES) {
            let Ok(text) = read_tail(&file) else { continue };
            if let Some(found) = latest_rate_limits(&text, now) {
                return Ok(found);
            }
        }
        Err(Failure::new(Status::NotFound, "No rate-limit data in recent Codex sessions yet"))
    }
}

fn codex_home() -> PathBuf {
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::paths::home().join(".codex"))
}

/// Session files under `sessions/YYYY/MM/DD`, newest first by modification
/// time, looking only at the most recent day directories.
fn newest_session_files(root: &Path, limit: usize) -> Vec<PathBuf> {
    fn dirs_desc(p: &Path) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = std::fs::read_dir(p)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        v.sort();
        v.reverse();
        v
    }
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    let mut days = 0;
    'outer: for y in dirs_desc(root) {
        for m in dirs_desc(&y) {
            for d in dirs_desc(&m) {
                for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                    let p = e.path();
                    if p.extension().is_some_and(|x| x == "jsonl") {
                        let t = e.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
                        files.push((t, p));
                    }
                }
                days += 1;
                if days >= 7 && files.len() >= limit {
                    break 'outer;
                }
            }
        }
    }
    files.sort_by_key(|f| std::cmp::Reverse(f.0));
    files.into_iter().take(limit).map(|(_, p)| p).collect()
}

fn read_tail(path: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(path)?;
    let len = f.metadata()?.len();
    if len > TAIL_BYTES {
        f.seek(SeekFrom::Start(len - TAIL_BYTES))?;
    }
    let mut buf = Vec::new();
    f.read_to_end(&mut buf)?;
    // The first line may be cut by the seek; it fails to parse and is skipped.
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// The last `token_count` event with rate limits in a session log.
pub fn latest_rate_limits(text: &str, now: Timestamp) -> Option<Usage> {
    text.lines().rev().find_map(|line| {
        if !line.contains("rate_limits") {
            return None;
        }
        let v: Value = serde_json::from_str(line).ok()?;
        let payload = &v["payload"];
        if payload["type"].as_str() != Some("token_count") {
            return None;
        }
        let limits = &payload["rate_limits"];
        if !limits.is_object() {
            return None;
        }
        let recorded = parse_time(&v["timestamp"]);
        let windows: Vec<Window> = [("primary", 300.0), ("secondary", 10080.0)]
            .into_iter()
            .filter_map(|(key, default_minutes)| window(key, &limits[key], default_minutes, recorded, now))
            .collect();
        if windows.is_empty() {
            return None;
        }
        let plan = limits["plan_type"].as_str().map(String::from);
        Some(Usage { plan, source: "session-log", windows })
    })
}

fn window(key: &str, w: &Value, default_minutes: f64, recorded: Option<Timestamp>, now: Timestamp) -> Option<Window> {
    let used = w["used_percent"].as_f64()?;
    let minutes = w["window_minutes"].as_f64().unwrap_or(default_minutes);
    // Newer logs carry an absolute `resets_at`; older ones seconds from the record.
    let resets_at = parse_time(&w["resets_at"]).or_else(|| {
        let secs = w["resets_in_seconds"].as_f64()?;
        recorded?.checked_add(SignedDuration::from_secs(secs as i64)).ok()
    });
    let (used, resets_at) = match resets_at {
        Some(r) if r <= now => (0.0, None),
        r => (clamp_percent(used), r),
    };
    Some(Window { id: key.into(), label: window_label(minutes), used_percent: used, resets_at })
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOG: &str = r#"{"timestamp":"2026-10-29T09:00:00.000Z","type":"session_meta","payload":{"id":"x"}}
{"timestamp":"2026-10-29T09:01:00.000Z","type":"event_msg","payload":{"type":"token_count","info":null,"rate_limits":{"primary":{"used_percent":10.0,"window_minutes":299,"resets_in_seconds":3600},"secondary":{"used_percent":5.0,"window_minutes":10079,"resets_in_seconds":86400}}}}
{"timestamp":"2026-10-29T09:30:00.000Z","type":"response_item","payload":{"type":"message","content":"rate_limits are discussed here"}}
{"timestamp":"2026-10-29T09:40:00.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"total_tokens":1}},"rate_limits":{"limit_id":"codex","primary":{"used_percent":37.5,"window_minutes":300,"resets_at":1793383200},"secondary":{"used_percent":12.0,"window_minutes":10080,"resets_at":1793750400},"plan_type":"plus"}}}
"#;

    #[test]
    fn reads_latest_token_count() {
        let now: Timestamp = "2026-10-29T10:00:00Z".parse().unwrap();
        let u = latest_rate_limits(LOG, now).unwrap();
        assert_eq!(u.plan.as_deref(), Some("plus"));
        assert_eq!(u.windows.len(), 2);
        assert_eq!(u.windows[0].label, "5h");
        assert_eq!(u.windows[0].used_percent, 37.5);
        assert_eq!(u.windows[0].resets_at.unwrap().as_second(), 1793383200);
        assert_eq!(u.windows[1].label, "7d");
    }

    #[test]
    fn relative_resets_and_rollover() {
        let old = LOG.lines().take(2).collect::<Vec<_>>().join("\n");
        let before: Timestamp = "2026-10-29T09:30:00Z".parse().unwrap();
        let u = latest_rate_limits(&old, before).unwrap();
        assert_eq!(u.windows[0].used_percent, 10.0);
        assert_eq!(u.windows[0].resets_at.unwrap(), "2026-10-29T10:01:00Z".parse().unwrap());
        assert_eq!(u.windows[1].label, "7d");
        // The 5-hour window reset since the record was written.
        let after: Timestamp = "2026-10-29T11:00:00Z".parse().unwrap();
        let u = latest_rate_limits(&old, after).unwrap();
        assert_eq!(u.windows[0].used_percent, 0.0);
        assert_eq!(u.windows[0].resets_at, None);
        assert_eq!(u.windows[1].used_percent, 5.0);
    }

    #[test]
    fn nothing_without_rate_limits() {
        assert!(latest_rate_limits("{\"payload\":{\"type\":\"token_count\"}}\nnot json rate_limits", Timestamp::now()).is_none());
    }
}
