//! Claude: the OAuth usage endpoint that Claude Code's `/usage` reads.
//!
//! Tokens, in order of preference:
//! 1. our own sign-in (PKCE in the browser, the same flow claude-usage-bar
//!    uses), stored DPAPI-encrypted in `%LOCALAPPDATA%\agent-usage-bar` and
//!    refreshed by us;
//! 2. Claude Code's `~/.claude/.credentials.json`, read-only. We never
//!    refresh that token: refreshing rotates the refresh token and would sign
//!    Claude Code out. When it has expired, using Claude Code renews it.

use crate::model::{Failure, Status, Usage, Window, clamp_percent, parse_time};
use crate::providers::Provider;
use base64::Engine;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Claude Code's public OAuth client, as used by Claude Code itself and by
/// Blimp-Labs/claude-usage-bar.
const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const AUTHORIZE_URL: &str = "https://claude.ai/oauth/authorize";
const TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const BETA: &str = "oauth-2025-04-20";
const SCOPES: &str = "user:profile user:inference";
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(600);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OwnToken {
    pub access_token: String,
    pub refresh_token: Option<String>,
    /// Unix seconds.
    pub expires_at: i64,
    #[serde(default)]
    pub scopes: Vec<String>,
}

impl OwnToken {
    fn expires_soon(&self, now: Timestamp) -> bool {
        self.expires_at - now.as_second() < 300
    }
}

pub struct ClaudeProvider {
    agent: Option<ureq::Agent>,
}

impl ClaudeProvider {
    pub fn new() -> Self {
        ClaudeProvider { agent: None }
    }

    fn agent(&mut self) -> ureq::Agent {
        self.agent.get_or_insert_with(crate::http::public).clone()
    }
}

impl Provider for ClaudeProvider {
    fn id(&self) -> crate::model::AgentId {
        crate::model::AgentId::Claude
    }

    fn uses_network(&self) -> bool {
        true
    }

    fn fetch(&mut self) -> Result<Usage, Failure> {
        let agent = self.agent();
        let now = Timestamp::now();

        if let Some(token) = load_own_token()
            && let Some(result) = fetch_with_own_token(&agent, token, now)
        {
            return result;
        }

        match read_claude_code_credentials() {
            Some(c) if c.expires_at > now.as_second() + 60 => {
                if !c.scopes.is_empty() && !c.scopes.iter().any(|s| s == "user:profile") {
                    return Err(Failure::new(
                        Status::SignInRequired,
                        "Claude Code's token cannot read usage; sign in here",
                    ));
                }
                let windows = get_usage(&agent, &c.access_token).map_err(|e| match e {
                    UsageError::Unauthorized => {
                        Failure::new(Status::SignInRequired, "Claude Code's sign-in was rejected; sign in here")
                    }
                    e => e.failure(),
                })?;
                Ok(Usage { plan: c.plan, source: "claude-code", windows })
            }
            Some(_) => Err(Failure::new(
                Status::SignInRequired,
                "Claude Code's sign-in expired; use Claude Code once or sign in here",
            )),
            None => Err(Failure::new(Status::SignInRequired, "Sign in to show Claude usage")),
        }
    }
}

/// `None` when the server no longer accepts our sign-in; it is then
/// discarded and Claude Code's credentials are tried instead.
fn fetch_with_own_token(agent: &ureq::Agent, mut token: OwnToken, now: Timestamp) -> Option<Result<Usage, Failure>> {
    let discard = || {
        crate::log::warn("claude: own sign-in rejected, discarding it");
        delete_own_token();
        None
    };
    let usage = |windows| Usage { plan: None, source: "sign-in", windows };
    let mut refreshed = false;
    if token.expires_soon(now) {
        match refresh(agent, &token) {
            Ok(t) => {
                token = t;
                refreshed = true;
            }
            Err(RefreshError::Rejected) => return discard(),
            Err(RefreshError::Transient(m)) => return Some(Err(Failure::error(m))),
        }
    }
    match get_usage(agent, &token.access_token) {
        Ok(w) => Some(Ok(usage(w))),
        Err(UsageError::Unauthorized) if !refreshed => match refresh(agent, &token) {
            Ok(t) => Some(get_usage(agent, &t.access_token).map(usage).map_err(UsageError::failure)),
            Err(RefreshError::Transient(m)) => Some(Err(Failure::error(m))),
            Err(RefreshError::Rejected) => discard(),
        },
        Err(UsageError::Unauthorized) => discard(),
        Err(e) => Some(Err(e.failure())),
    }
}

// ---- usage endpoint ----

enum UsageError {
    Unauthorized,
    Other(String),
}

impl UsageError {
    fn failure(self) -> Failure {
        match self {
            UsageError::Unauthorized => Failure::new(Status::SignInRequired, "Sign-in rejected; sign in again"),
            UsageError::Other(m) => Failure::error(m),
        }
    }
}

fn get_usage(agent: &ureq::Agent, access_token: &str) -> Result<Vec<Window>, UsageError> {
    let resp = agent
        .get(USAGE_URL)
        .set("Authorization", &format!("Bearer {access_token}"))
        .set("anthropic-beta", BETA)
        .set("Accept", "application/json")
        .call();
    match resp {
        Ok(r) => {
            let v: Value = r.into_json().map_err(|_| UsageError::Other("unreadable usage response".into()))?;
            let windows = parse_usage(&v);
            if windows.is_empty() {
                Err(UsageError::Other("usage response had no limits".into()))
            } else {
                Ok(windows)
            }
        }
        Err(ureq::Error::Status(401, _)) => Err(UsageError::Unauthorized),
        Err(ureq::Error::Status(429, _)) => Err(UsageError::Other("rate limited by the usage endpoint".into())),
        Err(e) => Err(UsageError::Other(crate::http::describe(&e))),
    }
}

/// Maps the usage response to windows: 5-hour first, then the weekly limits.
pub fn parse_usage(v: &Value) -> Vec<Window> {
    let mut out: Vec<Window> = Vec::new();
    for (key, label) in [
        ("five_hour", "5h"),
        ("seven_day", "7d"),
        ("seven_day_opus", "Opus 7d"),
        ("seven_day_sonnet", "Sonnet 7d"),
    ] {
        let b = &v[key];
        if let Some(u) = b["utilization"].as_f64() {
            out.push(Window {
                id: key.into(),
                label: label.into(),
                used_percent: clamp_percent(u),
                resets_at: parse_time(&b["resets_at"]),
            });
        }
    }
    // Newer accounts list per-model weekly windows under `limits`.
    if let Some(limits) = v["limits"].as_array() {
        for l in limits {
            if l["kind"].as_str() != Some("weekly_scoped") {
                continue;
            }
            let (Some(model), Some(p)) = (l["scope"]["model"]["display_name"].as_str(), l["percent"].as_f64()) else {
                continue;
            };
            let label = format!("{model} 7d");
            if out.iter().any(|w| w.label.eq_ignore_ascii_case(&label)) {
                continue;
            }
            out.push(Window {
                id: format!("weekly_{}", model.to_ascii_lowercase().replace(' ', "_")),
                label,
                used_percent: clamp_percent(p),
                resets_at: parse_time(&l["resets_at"]),
            });
        }
    }
    out
}

// ---- Claude Code's credentials (read-only) ----

#[derive(Debug, PartialEq)]
pub struct CliCredentials {
    pub access_token: String,
    /// Unix seconds.
    pub expires_at: i64,
    pub scopes: Vec<String>,
    pub plan: Option<String>,
}

fn claude_code_credentials_path() -> PathBuf {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::paths::home().join(".claude"))
        .join(".credentials.json")
}

fn read_claude_code_credentials() -> Option<CliCredentials> {
    parse_cli_credentials(&std::fs::read(claude_code_credentials_path()).ok()?)
}

pub fn parse_cli_credentials(bytes: &[u8]) -> Option<CliCredentials> {
    let v: Value = serde_json::from_slice(bytes).ok()?;
    let o = &v["claudeAiOauth"];
    let access_token = o["accessToken"].as_str()?.to_string();
    let expires_at = parse_time(&o["expiresAt"]).map(|t| t.as_second()).unwrap_or(0);
    let scopes = o["scopes"]
        .as_array()
        .map(|a| a.iter().filter_map(|s| s.as_str().map(String::from)).collect())
        .unwrap_or_default();
    let plan = o["subscriptionType"].as_str().map(String::from);
    Some(CliCredentials { access_token, expires_at, scopes, plan })
}

// ---- our own sign-in ----

pub fn has_own_token() -> bool {
    crate::paths::claude_token_file().exists()
}

fn load_own_token() -> Option<OwnToken> {
    let sealed = std::fs::read(crate::paths::claude_token_file()).ok()?;
    serde_json::from_slice(&crate::secret::unprotect(&sealed)?).ok()
}

fn save_own_token(t: &OwnToken) -> Result<(), String> {
    let json = serde_json::to_vec(t).map_err(|e| e.to_string())?;
    let sealed = crate::secret::protect(&json).ok_or("encrypting the token failed")?;
    crate::paths::write_atomic(&crate::paths::claude_token_file(), &sealed).map_err(|e| e.to_string())
}

pub fn delete_own_token() {
    let _ = std::fs::remove_file(crate::paths::claude_token_file());
}

enum RefreshError {
    /// The server refused the refresh token; sign in again.
    Rejected,
    Transient(String),
}

fn refresh(agent: &ureq::Agent, t: &OwnToken) -> Result<OwnToken, RefreshError> {
    let Some(refresh_token) = &t.refresh_token else {
        return Err(RefreshError::Rejected);
    };
    let mut body = serde_json::json!({
        "grant_type": "refresh_token",
        "refresh_token": refresh_token,
        "client_id": CLIENT_ID,
    });
    if !t.scopes.is_empty() {
        body["scope"] = Value::from(t.scopes.join(" "));
    }
    match agent.post(TOKEN_URL).send_json(body) {
        Ok(r) => {
            let v: Value = r.into_json().map_err(|_| RefreshError::Transient("unreadable token response".into()))?;
            let new = token_from_response(&v, Some(t), Timestamp::now())
                .ok_or(RefreshError::Transient("token response had no access token".into()))?;
            save_own_token(&new).map_err(RefreshError::Transient)?;
            Ok(new)
        }
        Err(ureq::Error::Status(code, _)) if (400..500).contains(&code) && code != 429 => Err(RefreshError::Rejected),
        Err(e) => Err(RefreshError::Transient(crate::http::describe(&e))),
    }
}

pub fn token_from_response(v: &Value, previous: Option<&OwnToken>, now: Timestamp) -> Option<OwnToken> {
    let access_token = v["access_token"].as_str()?.to_string();
    let refresh_token = v["refresh_token"]
        .as_str()
        .map(String::from)
        .or_else(|| previous.and_then(|p| p.refresh_token.clone()));
    let expires_in = v["expires_in"].as_i64().unwrap_or(3600);
    let scopes = match v["scope"].as_str() {
        Some(s) => s.split_whitespace().map(String::from).collect(),
        None => previous
            .map(|p| p.scopes.clone())
            .unwrap_or_else(|| SCOPES.split(' ').map(String::from).collect()),
    };
    Some(OwnToken { access_token, refresh_token, expires_at: now.as_second() + expires_in, scopes })
}

/// Runs the browser sign-in: opens claude.ai, waits for the redirect on a
/// loopback port and stores the resulting token. Blocks for up to ten minutes.
pub fn sign_in() -> Result<(), String> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let redirect_uri = format!("http://localhost:{port}/callback");
    let verifier = random_token();
    let state = random_token();
    let challenge = b64(&<sha2::Sha256 as sha2::Digest>::digest(verifier.as_bytes()));
    let url = format!(
        "{AUTHORIZE_URL}?code=true&client_id={CLIENT_ID}&response_type=code&redirect_uri={}&scope={}&code_challenge={challenge}&code_challenge_method=S256&state={state}",
        percent_encode(&redirect_uri),
        percent_encode(SCOPES),
    );
    crate::shell::open(&url);

    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let deadline = Instant::now() + SIGN_IN_TIMEOUT;
    let code = loop {
        if Instant::now() > deadline {
            return Err("sign-in timed out".into());
        }
        let stream = match listener.accept() {
            Ok((s, _)) => s,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(250));
                continue;
            }
            Err(e) => return Err(e.to_string()),
        };
        let _ = stream.set_nonblocking(false);
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        let mut line = String::new();
        let _ = BufReader::new(&stream).read_line(&mut line);
        let query = callback_query(&line);
        let got_state = query.as_ref().and_then(|q| param(q, "state"));
        let got_code = query.as_ref().and_then(|q| param(q, "code"));
        let ok = got_state.as_deref() == Some(state.as_str()) && got_code.is_some();
        // Ignore stray requests (favicon, other tabs) and keep waiting.
        if query.is_none() || got_state.as_deref() != Some(state.as_str()) {
            respond(stream, 404, "Not found");
            continue;
        }
        if !ok {
            respond(stream, 400, "Sign-in was cancelled. You can close this tab.");
            return Err("sign-in was cancelled".into());
        }
        respond(stream, 200, "Signed in to agent-usage-bar. You can close this tab.");
        break got_code.unwrap_or_default();
    };
    // claude.ai may append "#state" to the code.
    let code = code.split('#').next().unwrap_or_default().to_string();

    let body = serde_json::json!({
        "grant_type": "authorization_code",
        "code": code,
        "state": state,
        "client_id": CLIENT_ID,
        "redirect_uri": redirect_uri,
        "code_verifier": verifier,
    });
    let agent = crate::http::public();
    let v: Value = agent
        .post(TOKEN_URL)
        .send_json(body)
        .map_err(|e| format!("token exchange failed: {}", crate::http::describe(&e)))?
        .into_json()
        .map_err(|_| "unreadable token response".to_string())?;
    let token = token_from_response(&v, None, Timestamp::now()).ok_or("token response had no access token")?;
    save_own_token(&token)
}

fn respond(mut stream: std::net::TcpStream, code: u16, text: &str) {
    let body = format!(
        "<!doctype html><meta charset=utf-8><title>agent-usage-bar</title><body style=\"font:16px system-ui;margin:3em\">{text}</body>"
    );
    let _ = write!(
        stream,
        "HTTP/1.1 {code} OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
}

/// The query string of `GET /callback?... HTTP/1.1`.
pub fn callback_query(request_line: &str) -> Option<String> {
    let target = request_line.strip_prefix("GET ")?.split(' ').next()?;
    let query = target.strip_prefix("/callback?")?;
    Some(query.to_string())
}

pub fn param(query: &str, name: &str) -> Option<String> {
    query.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        (k == name).then(|| percent_decode(v))
    })
}

fn random_token() -> String {
    let mut buf = [0u8; 32];
    getrandom::fill(&mut buf).expect("OS random source");
    b64(&buf)
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

pub fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => match std::str::from_utf8(&bytes[i + 1..i + 3])
                .ok()
                .and_then(|h| u8::from_str_radix(h, 16).ok())
            {
                Some(b) => {
                    out.push(b);
                    i += 2;
                }
                None => out.push(b'%'),
            },
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_usage_response() {
        let v = serde_json::json!({
            "five_hour": {"utilization": 20.0, "resets_at": "2026-10-29T18:00:00.123+00:00"},
            "seven_day": {"utilization": 63.5, "resets_at": "2026-11-03T00:00:00Z"},
            "seven_day_opus": {"utilization": null, "resets_at": null},
            "seven_day_sonnet": {"utilization": 140.0, "resets_at": null},
            "extra_usage": {"is_enabled": false},
            "limits": [
                {"kind": "weekly_scoped", "percent": 48.5, "resets_at": "2026-11-03T00:00:00Z",
                 "scope": {"model": {"display_name": "Fable 5"}}},
                {"kind": "weekly_scoped", "percent": 10.0, "scope": {"model": {"display_name": "Sonnet"}}},
                {"kind": "five_hour_scoped", "percent": 1.0}
            ]
        });
        let w = parse_usage(&v);
        let labels: Vec<_> = w.iter().map(|w| w.label.as_str()).collect();
        assert_eq!(labels, ["5h", "7d", "Sonnet 7d", "Fable 5 7d"]);
        assert_eq!(w[0].used_percent, 20.0);
        assert_eq!(w[0].resets_at.unwrap().as_second(), 1793296800);
        assert_eq!(w[2].used_percent, 100.0);
        assert_eq!(w[3].id, "weekly_fable_5");
        assert!(parse_usage(&serde_json::json!({})).is_empty());
    }

    #[test]
    fn parses_claude_code_credentials() {
        let c = parse_cli_credentials(
            br#"{"claudeAiOauth":{"accessToken":"sk-ant-oat-x","refreshToken":"r","expiresAt":1793144966901,
                "scopes":["user:inference","user:profile"],"subscriptionType":"pro"},"mcpOAuth":{}}"#,
        )
        .unwrap();
        assert_eq!(c.access_token, "sk-ant-oat-x");
        assert_eq!(c.expires_at, 1793144966);
        assert_eq!(c.plan.as_deref(), Some("pro"));
        assert!(c.scopes.contains(&"user:profile".to_string()));
        assert!(parse_cli_credentials(b"{}").is_none());
    }

    #[test]
    fn builds_tokens_from_responses() {
        let now = Timestamp::from_second(1000).unwrap();
        let t = token_from_response(
            &serde_json::json!({"access_token": "a", "refresh_token": "r", "expires_in": 28800, "scope": "user:profile user:inference"}),
            None,
            now,
        )
        .unwrap();
        assert_eq!(t.expires_at, 29800);
        assert_eq!(t.scopes, ["user:profile", "user:inference"]);
        let t2 = token_from_response(&serde_json::json!({"access_token": "b"}), Some(&t), now).unwrap();
        assert_eq!(t2.refresh_token.as_deref(), Some("r"));
        assert_eq!(t2.scopes, t.scopes);
    }

    #[test]
    fn parses_callback() {
        let q = callback_query("GET /callback?code=abc%23st&state=x-y_z HTTP/1.1\r\n").unwrap();
        assert_eq!(param(&q, "code").as_deref(), Some("abc#st"));
        assert_eq!(param(&q, "state").as_deref(), Some("x-y_z"));
        assert!(callback_query("GET /favicon.ico HTTP/1.1").is_none());
        assert_eq!(percent_encode("http://localhost:1/callback"), "http%3A%2F%2Flocalhost%3A1%2Fcallback");
        assert_eq!(percent_decode("100%"), "100%");
    }
}
