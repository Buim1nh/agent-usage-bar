//! agent-usage-bar: a tray icon with AI coding-agent usage limits.
//!
//! `agent-usage-bar.exe` runs the tray app (one instance per user session).
//! `agent-usage-bar.exe --once` fetches every agent once, writes status.json
//! and prints it to stdout, without a tray icon.

#![cfg_attr(not(test), windows_subsystem = "windows")]

mod app;
mod config;
mod github;
mod http;
mod icon;
mod log;
mod model;
mod paths;
mod popover;
mod providers;
mod secret;
mod shell;
mod status;
mod worker;

use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext};
use windows::core::w;

fn main() {
    if std::env::args().any(|a| a == "--once") {
        once();
        return;
    }
    unsafe {
        let _mutex = CreateMutexW(None, true, w!("Local\\agent-usage-bar"));
        if GetLastError() == ERROR_ALREADY_EXISTS {
            return;
        }
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    log::info(concat!("starting agent-usage-bar ", env!("CARGO_PKG_VERSION")));
    app::run();
    log::info("exiting");
}

fn once() {
    let (agents, gh) = worker::fetch_all_now();
    let now = jiff::Timestamp::now();
    let mut states: Vec<model::AgentState> = model::AgentId::ALL.iter().map(|a| model::AgentState::new(*a)).collect();
    for (id, r) in agents {
        if let Some(s) = states.iter_mut().find(|s| s.id == id) {
            s.apply(r, now);
        }
    }
    let mut github = github::GithubState::default();
    github.apply(gh, now);
    let config = config::Config::load();
    status::write(&states, &github, &config, false);
    let file = status::build(&states, &github, &config, false, now);
    if let Ok(json) = serde_json::to_string_pretty(&file) {
        use std::io::Write;
        let _ = writeln!(std::io::stdout(), "{json}");
    }
}
