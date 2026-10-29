//! The shared GitHub section: your contribution calendar (commits, PRs,
//! issues, reviews per day), drawn like the graph on a GitHub profile.
//!
//! It asks the GitHub CLI (`gh api graphql`) so the app stores no GitHub
//! credentials: `gh` uses its own sign-in from `gh auth login`.

use crate::model::{Failure, Status};
use jiff::Timestamp;
use jiff::civil::Date;
use serde::Serialize;
use serde_json::Value;
use std::path::PathBuf;

const QUERY: &str = "query{viewer{login contributionsCollection{contributionCalendar{totalContributions weeks{contributionDays{date contributionCount contributionLevel}}}}}}";

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Day {
    pub date: Date,
    pub count: u32,
    /// 0 (none) to 4, GitHub's own quartile shading.
    pub level: u8,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Contributions {
    pub login: String,
    /// Contributions in the last year.
    pub total: u32,
    /// Oldest first; the first day is a Sunday, as on github.com.
    pub days: Vec<Day>,
}

#[derive(Clone, Debug)]
pub struct GithubState {
    pub status: Status,
    pub message: Option<String>,
    pub data: Option<Contributions>,
    pub fetched_at: Option<Timestamp>,
}

impl Default for GithubState {
    fn default() -> Self {
        GithubState { status: Status::Pending, message: None, data: None, fetched_at: None }
    }
}

impl GithubState {
    pub fn apply(&mut self, r: Result<Contributions, Failure>, now: Timestamp) {
        match r {
            Ok(c) => {
                *self = GithubState { status: Status::Ok, message: None, data: Some(c), fetched_at: Some(now) };
            }
            Err(f) => {
                if f.status != Status::Error {
                    self.data = None;
                }
                self.status = f.status;
                self.message = Some(f.message);
            }
        }
    }
}

pub fn fetch() -> Result<Contributions, Failure> {
    let Some(gh) = find_gh() else {
        return Err(Failure::new(Status::NotFound, "Install GitHub CLI (gh) and run gh auth login"));
    };
    let mut cmd = std::process::Command::new(gh);
    cmd.args(["api", "graphql", "-f", &format!("query={QUERY}")])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let out = cmd.output().map_err(|e| Failure::error(format!("running gh failed: {e}")))?;
    if !out.status.success() {
        return Err(Failure::new(Status::SignInRequired, "gh is not signed in; run gh auth login"));
    }
    let v: Value = serde_json::from_slice(&out.stdout).map_err(|_| Failure::error("unreadable gh output"))?;
    parse(&v).ok_or_else(|| Failure::error("gh returned no contribution calendar"))
}

pub fn parse(v: &Value) -> Option<Contributions> {
    let viewer = &v["data"]["viewer"];
    let cal = &viewer["contributionsCollection"]["contributionCalendar"];
    let mut days = Vec::new();
    for w in cal["weeks"].as_array()? {
        for d in w["contributionDays"].as_array().into_iter().flatten() {
            let Some(date) = d["date"].as_str().and_then(|s| s.parse::<Date>().ok()) else { continue };
            let level = match d["contributionLevel"].as_str() {
                Some("FIRST_QUARTILE") => 1,
                Some("SECOND_QUARTILE") => 2,
                Some("THIRD_QUARTILE") => 3,
                Some("FOURTH_QUARTILE") => 4,
                _ => 0,
            };
            days.push(Day { date, count: d["contributionCount"].as_u64().unwrap_or(0) as u32, level });
        }
    }
    Some(Contributions {
        login: viewer["login"].as_str().unwrap_or_default().to_string(),
        total: cal["totalContributions"].as_u64().unwrap_or(0) as u32,
        days,
    })
}

fn find_gh() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).map(|d| d.join("gh.exe")).collect())
        .unwrap_or_default();
    for (var, rel) in [("ProgramFiles", "GitHub CLI\\gh.exe"), ("LOCALAPPDATA", "Programs\\GitHub CLI\\gh.exe")] {
        if let Some(base) = std::env::var_os(var) {
            candidates.push(PathBuf::from(base).join(rel));
        }
    }
    candidates.into_iter().find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_calendar() {
        let v = serde_json::json!({"data": {"viewer": {"login": "octo", "contributionsCollection": {"contributionCalendar": {
            "totalContributions": 130,
            "weeks": [
                {"contributionDays": [
                    {"date": "2026-10-25", "contributionCount": 0, "contributionLevel": "NONE"},
                    {"date": "2026-10-26", "contributionCount": 5, "contributionLevel": "SECOND_QUARTILE"}
                ]},
                {"contributionDays": [{"date": "2026-10-27", "contributionCount": 12, "contributionLevel": "FOURTH_QUARTILE"}]}
            ]}}}}});
        let c = parse(&v).unwrap();
        assert_eq!(c.login, "octo");
        assert_eq!(c.total, 130);
        assert_eq!(c.days.len(), 3);
        assert_eq!(c.days[1], Day { date: "2026-10-26".parse().unwrap(), count: 5, level: 2 });
        assert_eq!(c.days[2].level, 4);
        assert!(parse(&serde_json::json!({"errors": []})).is_none());
    }
}
