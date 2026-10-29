//! Per-user locations. Everything the app writes lives in
//! `%LOCALAPPDATA%\agent-usage-bar`.

use std::path::PathBuf;

pub fn data_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join("AppData").join("Local"));
    base.join("agent-usage-bar")
}

pub fn home() -> PathBuf {
    std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

pub fn config_file() -> PathBuf {
    data_dir().join("config.json")
}

pub fn status_file() -> PathBuf {
    data_dir().join("status.json")
}

pub fn log_file() -> PathBuf {
    data_dir().join("agent-usage-bar.log")
}

/// Our own Claude sign-in, encrypted with DPAPI for the current user.
pub fn claude_token_file() -> PathBuf {
    data_dir().join("claude-token.bin")
}

/// Writes through a temporary file and a rename so readers (wdock) never see
/// a half-written file.
pub fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}
