//! The worker thread: runs providers (which block on disk and network) off
//! the UI thread and posts results back as window messages.

use crate::github::Contributions;
use crate::model::{AgentId, Failure, Usage};
use crate::providers::Provider;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};

pub enum Job {
    /// Fetch these agents (and GitHub). With `network: false`, providers that
    /// contact remote servers and GitHub are skipped.
    Refresh { agents: Vec<AgentId>, github: bool, network: bool },
    SignInClaude,
}

pub enum Event {
    Agent(AgentId, Result<Usage, Failure>),
    Github(Result<Contributions, Failure>),
    RefreshDone,
    SignInDone(Result<(), String>),
}

/// Results waiting for the UI thread, which drains them on `notify`.
pub type Inbox = Arc<Mutex<Vec<Event>>>;

pub fn spawn(inbox: Inbox, notify: impl Fn() + Send + Clone + 'static) -> Sender<Job> {
    let (tx, rx) = std::sync::mpsc::channel();
    let n = notify.clone();
    let ib = inbox.clone();
    std::thread::Builder::new()
        .name("refresh".into())
        .spawn(move || run(rx, ib, n))
        .expect("spawn worker");
    tx
}

fn run(rx: Receiver<Job>, inbox: Inbox, notify: impl Fn() + Clone + Send + 'static) {
    let mut providers = crate::providers::all();
    let push = |e: Event| {
        inbox.lock().unwrap().push(e);
        notify();
    };
    while let Ok(job) = rx.recv() {
        match job {
            Job::Refresh { agents, github, network } => {
                for p in providers.iter_mut() {
                    if agents.contains(&p.id()) && (network || !p.uses_network()) {
                        push(Event::Agent(p.id(), fetch_logged(p.as_mut())));
                    }
                }
                if github && network {
                    let r = crate::github::fetch();
                    if let Err(f) = &r {
                        crate::log::warn(&format!("github: {}", f.message));
                    }
                    push(Event::Github(r));
                }
                push(Event::RefreshDone);
            }
            Job::SignInClaude => {
                // On its own thread: it waits for the browser for minutes.
                let ib = inbox.clone();
                let n = notify.clone();
                std::thread::spawn(move || {
                    let r = crate::providers::claude::sign_in();
                    match &r {
                        Ok(()) => crate::log::info("claude: signed in"),
                        Err(e) => crate::log::warn(&format!("claude sign-in: {e}")),
                    }
                    ib.lock().unwrap().push(Event::SignInDone(r));
                    n();
                });
            }
        }
    }
}

fn fetch_logged(p: &mut dyn Provider) -> Result<Usage, Failure> {
    let r = p.fetch();
    if let Err(f) = &r {
        crate::log::info(&format!("{}: {:?}: {}", p.id().name(), f.status, f.message));
    }
    r
}

pub type Snapshot = (Vec<(AgentId, Result<Usage, Failure>)>, Result<Contributions, Failure>);

/// Fetches every agent once on the calling thread, for `--once`.
pub fn fetch_all_now() -> Snapshot {
    let agents = crate::providers::all().into_iter().map(|mut p| (p.id(), p.fetch())).collect();
    (agents, crate::github::fetch())
}
