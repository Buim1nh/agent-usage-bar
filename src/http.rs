//! HTTP agents. TLS goes through Windows SChannel (native-tls), so the
//! system certificate store and proxy-free defaults apply.

use std::sync::Arc;
use std::time::Duration;

pub const USER_AGENT: &str = concat!("agent-usage-bar/", env!("CARGO_PKG_VERSION"));

/// For public endpoints (api.anthropic.com, platform.claude.com).
pub fn public() -> ureq::Agent {
    let tls = native_tls::TlsConnector::new().expect("SChannel TLS");
    ureq::AgentBuilder::new()
        .tls_connector(Arc::new(tls))
        .timeout(Duration::from_secs(20))
        .user_agent(USER_AGENT)
        .build()
}

/// For agents' local servers on 127.0.0.1 only, which serve self-signed
/// certificates. Never use this for a remote host.
pub fn loopback() -> ureq::Agent {
    let tls = native_tls::TlsConnector::builder()
        .danger_accept_invalid_certs(true)
        .danger_accept_invalid_hostnames(true)
        .build()
        .expect("SChannel TLS");
    ureq::AgentBuilder::new()
        .tls_connector(Arc::new(tls))
        .timeout(Duration::from_secs(5))
        .user_agent(USER_AGENT)
        .build()
}

/// A short, token-free description of a failed request.
pub fn describe(err: &ureq::Error) -> String {
    match err {
        ureq::Error::Status(code, _) => format!("HTTP {code}"),
        ureq::Error::Transport(t) => format!("network error ({})", t.kind()),
    }
}
