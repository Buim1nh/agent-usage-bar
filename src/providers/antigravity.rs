//! Antigravity: quota from the IDE's local language server.
//!
//! The Antigravity IDE (and the `agy` CLI) runs `language_server*.exe` with
//! `--csrf_token <token>` on its command line and serves a Connect API on a
//! loopback port with a self-signed certificate. We find the process, read
//! its command line and listening ports, and POST to
//! `RetrieveUserQuotaSummary` (newer) or `GetUserStatus` (older). The API is
//! internal and undocumented; the protocol follows what
//! steipete/CodexBar (MIT) and Henrik-3/AntigravityQuota (MIT) document.
//! The token stays in memory and is only sent to 127.0.0.1.

use crate::model::{AgentId, Failure, Status, Usage, Window, clamp_percent, parse_time};
use crate::providers::Provider;
use serde_json::Value;

const SERVICE: &str = "exa.language_server_pb.LanguageServerService";

#[derive(Clone, Debug, PartialEq)]
pub struct ServerInfo {
    pub pid: u32,
    pub csrf_token: String,
    pub extension_port: Option<u16>,
}

pub struct AntigravityProvider {
    agent: Option<ureq::Agent>,
    /// Last working (pid, port, https).
    cached: Option<(u32, u16, bool)>,
}

impl AntigravityProvider {
    pub fn new() -> Self {
        AntigravityProvider { agent: None, cached: None }
    }
}

impl Provider for AntigravityProvider {
    fn id(&self) -> AgentId {
        AgentId::Antigravity
    }

    fn uses_network(&self) -> bool {
        // Loopback only.
        false
    }

    fn fetch(&mut self) -> Result<Usage, Failure> {
        let servers = find_servers();
        if servers.is_empty() {
            return Err(Failure::new(Status::NotRunning, "Antigravity is not running; open it to read quota"));
        }
        let agent = self.agent.get_or_insert_with(crate::http::loopback).clone();
        let mut last_err = String::from("no responding port");
        for server in &servers {
            let mut candidates: Vec<(u16, bool)> = Vec::new();
            if let Some((pid, port, tls)) = self.cached
                && pid == server.pid
            {
                candidates.push((port, tls));
            }
            for port in listening_ports(server.pid) {
                candidates.push((port, true));
            }
            if let Some(p) = server.extension_port {
                candidates.push((p, false));
            }
            candidates.dedup();
            for (port, tls) in candidates {
                match query(&agent, server, port, tls) {
                    Ok(usage) => {
                        self.cached = Some((server.pid, port, tls));
                        return Ok(usage);
                    }
                    Err(e) => last_err = e,
                }
            }
        }
        self.cached = None;
        Err(Failure::error(format!("Antigravity language server did not answer ({last_err})")))
    }
}

fn post(agent: &ureq::Agent, server: &ServerInfo, port: u16, tls: bool, method: &str) -> Result<Value, String> {
    let scheme = if tls { "https" } else { "http" };
    let body = serde_json::json!({"metadata": {
        "ideName": "antigravity", "extensionName": "antigravity", "locale": "en", "ideVersion": "unknown"
    }});
    agent
        .post(&format!("{scheme}://127.0.0.1:{port}/{SERVICE}/{method}"))
        .set("Connect-Protocol-Version", "1")
        .set("X-Codeium-Csrf-Token", &server.csrf_token)
        .send_json(body)
        .map_err(|e| crate::http::describe(&e))?
        .into_json()
        .map_err(|_| "not JSON".to_string())
}

fn query(agent: &ureq::Agent, server: &ServerInfo, port: u16, tls: bool) -> Result<Usage, String> {
    let status = post(agent, server, port, tls, "GetUserStatus");
    if let Ok(summary) = post(agent, server, port, tls, "RetrieveUserQuotaSummary") {
        let windows = parse_quota_summary(&summary);
        if !windows.is_empty() {
            let plan = status.as_ref().ok().and_then(plan_name);
            return Ok(Usage { plan, source: "language-server", windows });
        }
    }
    let status = status?;
    let windows = parse_user_status(&status);
    if windows.is_empty() {
        return Err("no quota in response".into());
    }
    Ok(Usage { plan: plan_name(&status), source: "language-server", windows })
}

fn plan_name(status: &Value) -> Option<String> {
    status["userStatus"]["planStatus"]["planInfo"]["planName"].as_str().map(String::from)
}

fn remaining_to_used(v: &Value) -> Option<f64> {
    v.as_f64().map(|f| clamp_percent((1.0 - f) * 100.0))
}

/// `response.groups[].buckets[]` from RetrieveUserQuotaSummary.
pub fn parse_quota_summary(v: &Value) -> Vec<Window> {
    let mut out = Vec::new();
    for g in v["response"]["groups"].as_array().into_iter().flatten() {
        let group = g["displayName"].as_str().unwrap_or_default();
        for b in g["buckets"].as_array().into_iter().flatten() {
            let id = b["bucketId"].as_str().unwrap_or("bucket").to_string();
            let remaining = &b["remaining"]["remainingFraction"];
            let remaining = if remaining.is_null() { &b["remainingFraction"] } else { remaining };
            let Some(used) = remaining_to_used(remaining) else { continue };
            let name = b["displayName"].as_str().unwrap_or(&id);
            let label = short_label(group, b["window"].as_str().unwrap_or_default(), &id, name);
            let reset = &b["remaining"]["resetTime"];
            let reset = if reset.is_null() { &b["resetTime"] } else { reset };
            out.push(Window { id, label, used_percent: used, resets_at: parse_time(reset) });
        }
    }
    out
}

/// "Gemini 7d" from group "Gemini Models", window "weekly"; "Claude/GPT 5h"
/// from "Claude and GPT models" and bucket id "3p-5h".
fn short_label(group: &str, window: &str, bucket_id: &str, display: &str) -> String {
    let lower = group.to_ascii_lowercase();
    let group = if lower.ends_with(" models") { &group[..group.len() - 7] } else { group };
    let group = group.replace(" and ", "/");
    let hint = format!("{window} {bucket_id}").to_ascii_lowercase();
    let window = if hint.contains("week") {
        "7d".to_string()
    } else if hint.contains("5h") || hint.contains("five") {
        "5h".to_string()
    } else if hint.contains("daily") || hint.contains("day") {
        "1d".to_string()
    } else {
        display.to_string()
    };
    if group.is_empty() { window } else { format!("{group} {window}") }
}

/// `userStatus.cascadeModelConfigData.clientModelConfigs[].quotaInfo` from
/// GetUserStatus: one window per model, most used first.
pub fn parse_user_status(v: &Value) -> Vec<Window> {
    let mut out: Vec<Window> = Vec::new();
    let models = &v["userStatus"]["cascadeModelConfigData"]["clientModelConfigs"];
    for m in models.as_array().into_iter().flatten() {
        let q = &m["quotaInfo"];
        let Some(used) = remaining_to_used(&q["remainingFraction"]) else { continue };
        let label = m["label"].as_str().unwrap_or("model").to_string();
        if out.iter().any(|w| w.label == label) {
            continue;
        }
        let id = m["modelOrAlias"]["model"].as_str().unwrap_or(&label).to_string();
        out.push(Window { id, label, used_percent: used, resets_at: parse_time(&q["resetTime"]) });
    }
    out.sort_by(|a, b| b.used_percent.total_cmp(&a.used_percent).then_with(|| a.label.cmp(&b.label)));
    out
}

/// Value of `--name value` or `--name=value` in a command line.
pub fn flag(cmdline: &str, name: &str) -> Option<String> {
    let mut words = split_words(cmdline).into_iter();
    while let Some(w) = words.next() {
        if let Some(rest) = w.strip_prefix(name) {
            if let Some(v) = rest.strip_prefix('=') {
                return Some(v.to_string());
            }
            if rest.is_empty() {
                return words.next();
            }
        }
    }
    None
}

fn split_words(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in s.chars() {
        match c {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// A language server belongs to Antigravity when its path or app data dir says so.
pub fn server_from_cmdline(pid: u32, cmdline: &str) -> Option<ServerInfo> {
    let lower = cmdline.to_ascii_lowercase();
    if !lower.contains("antigravity") && !lower.contains("agy") {
        return None;
    }
    let csrf_token = flag(cmdline, "--csrf_token")?;
    let extension_port = flag(cmdline, "--extension_server_port").and_then(|p| p.parse().ok());
    Some(ServerInfo { pid, csrf_token, extension_port })
}

fn find_servers() -> Vec<ServerInfo> {
    win::language_server_processes()
        .into_iter()
        .filter_map(|(pid, cmdline)| server_from_cmdline(pid, &cmdline))
        .collect()
}

fn listening_ports(pid: u32) -> Vec<u16> {
    win::listening_ports(pid)
}

mod win {
    use windows::Wdk::System::Threading::{NtQueryInformationProcess, PROCESSINFOCLASS};
    use windows::Win32::Foundation::{CloseHandle, UNICODE_STRING};
    use windows::Win32::NetworkManagement::IpHelper::{GetExtendedTcpTable, MIB_TCPTABLE_OWNER_PID, TCP_TABLE_OWNER_PID_LISTENER};
    use windows::Win32::Networking::WinSock::AF_INET;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
    };
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

    const PROCESS_COMMAND_LINE_INFORMATION: PROCESSINFOCLASS = PROCESSINFOCLASS(60);

    /// (pid, command line) of every running `language_server*` process we may read.
    pub fn language_server_processes() -> Vec<(u32, String)> {
        let mut out = Vec::new();
        unsafe {
            let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return out };
            let mut e = PROCESSENTRY32W { dwSize: size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
            let mut ok = Process32FirstW(snap, &mut e).is_ok();
            while ok {
                let len = e.szExeFile.iter().position(|c| *c == 0).unwrap_or(e.szExeFile.len());
                let name = String::from_utf16_lossy(&e.szExeFile[..len]).to_ascii_lowercase();
                if (name.starts_with("language_server") || name.starts_with("language-server"))
                    && let Some(cmd) = command_line(e.th32ProcessID) {
                        out.push((e.th32ProcessID, cmd));
                    }
                ok = Process32NextW(snap, &mut e).is_ok();
            }
            let _ = CloseHandle(snap);
        }
        out
    }

    fn command_line(pid: u32) -> Option<String> {
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
            let mut buf = vec![0u8; 64 * 1024];
            let mut ret = 0u32;
            let status = NtQueryInformationProcess(
                h,
                PROCESS_COMMAND_LINE_INFORMATION,
                buf.as_mut_ptr().cast(),
                buf.len() as u32,
                &mut ret,
            );
            let _ = CloseHandle(h);
            if status.is_err() {
                return None;
            }
            let us = &*(buf.as_ptr() as *const UNICODE_STRING);
            if us.Buffer.is_null() {
                return None;
            }
            let chars = std::slice::from_raw_parts(us.Buffer.0, us.Length as usize / 2);
            Some(String::from_utf16_lossy(chars))
        }
    }

    pub fn listening_ports(pid: u32) -> Vec<u16> {
        let mut size = 0u32;
        unsafe {
            let _ = GetExtendedTcpTable(None, &mut size, false, AF_INET.0 as u32, TCP_TABLE_OWNER_PID_LISTENER, 0);
            if size == 0 {
                return Vec::new();
            }
            let mut buf = vec![0u32; size as usize / 4 + 1];
            if GetExtendedTcpTable(Some(buf.as_mut_ptr().cast()), &mut size, false, AF_INET.0 as u32, TCP_TABLE_OWNER_PID_LISTENER, 0)
                != 0
            {
                return Vec::new();
            }
            let table = &*(buf.as_ptr() as *const MIB_TCPTABLE_OWNER_PID);
            let rows = std::slice::from_raw_parts(table.table.as_ptr(), table.dwNumEntries as usize);
            let mut ports: Vec<u16> = rows
                .iter()
                .filter(|r| r.dwOwningPid == pid)
                .map(|r| u16::from_be(r.dwLocalPort as u16))
                .collect();
            ports.sort_unstable();
            ports.dedup();
            ports
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_flags() {
        let cmd = r#""C:\Users\u\AppData\Local\Programs\Antigravity IDE\resources\app\extensions\antigravity\bin\language_server_windows_x64.exe" --enable_lsp --csrf_token 1a2b-3c --extension_server_port=51234 --app_data_dir antigravity-ide"#;
        let s = server_from_cmdline(42, cmd).unwrap();
        assert_eq!(s.csrf_token, "1a2b-3c");
        assert_eq!(s.extension_port, Some(51234));
        assert!(server_from_cmdline(1, r#"C:\other\language_server.exe --csrf_token x"#).is_none());
        assert!(server_from_cmdline(1, r#"C:\Antigravity\language_server.exe --port 1"#).is_none());
    }

    #[test]
    fn parses_quota_summary() {
        let v = serde_json::json!({"response": {"groups": [
            {"displayName": "Gemini Models", "buckets": [
                {"bucketId": "gemini-5h", "displayName": "5 Hour Limit Remaining", "remaining": {"remainingFraction": 0.25, "resetTime": "2026-10-29T18:00:00Z"}},
                {"bucketId": "gemini-weekly", "displayName": "Weekly Limit Remaining", "window": "weekly", "remainingFraction": 1, "resetTime": "2026-11-05T07:25:00Z"}
            ]},
            {"displayName": "Claude and GPT models", "buckets": [
                {"bucketId": "3p-weekly", "displayName": "Weekly Limit Remaining", "window": "weekly", "remainingFraction": 0.5},
                {"bucketId": "x", "displayName": "No data", "remaining": {}}
            ]}
        ]}});
        let w = parse_quota_summary(&v);
        assert_eq!(w.iter().map(|w| w.label.as_str()).collect::<Vec<_>>(), ["Gemini 5h", "Gemini 7d", "Claude/GPT 7d"]);
        assert_eq!(w[0].used_percent, 75.0);
        assert_eq!(w[0].resets_at.unwrap().as_second(), 1793296800);
        assert_eq!(w[1].used_percent, 0.0);
        assert_eq!(w[1].resets_at.unwrap(), "2026-11-05T07:25:00Z".parse().unwrap());
        assert_eq!(w[2].used_percent, 50.0);
    }

    #[test]
    fn parses_user_status() {
        let v = serde_json::json!({"userStatus": {
            "planStatus": {"planInfo": {"planName": "Pro"}},
            "cascadeModelConfigData": {"clientModelConfigs": [
                {"label": "Gemini 3 Pro", "modelOrAlias": {"model": "M1"}, "quotaInfo": {"remainingFraction": 0.9, "resetTime": "2026-10-29T18:00:00Z"}},
                {"label": "Claude Sonnet", "modelOrAlias": {"model": "M2"}, "quotaInfo": {"remainingFraction": 0.4}},
                {"label": "No quota", "modelOrAlias": {"model": "M3"}}
            ]}
        }});
        let w = parse_user_status(&v);
        assert_eq!(w.iter().map(|w| w.label.as_str()).collect::<Vec<_>>(), ["Claude Sonnet", "Gemini 3 Pro"]);
        assert!((w[0].used_percent - 60.0).abs() < 1e-9);
        assert_eq!(w[1].id, "M1");
        assert_eq!(plan_name(&v).as_deref(), Some("Pro"));
    }
}
