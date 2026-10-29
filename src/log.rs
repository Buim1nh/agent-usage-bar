//! A tiny append-only log. Never pass tokens or response bodies that could
//! hold them; messages are status codes and short reasons.

use std::io::Write;

const MAX_BYTES: u64 = 256 * 1024;

fn write(level: &str, msg: &str) {
    let path = crate::paths::log_file();
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_BYTES) {
        let _ = std::fs::rename(&path, path.with_extension("log.old"));
    }
    let _ = std::fs::create_dir_all(crate::paths::data_dir());
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "{} {level} {msg}", jiff::Timestamp::now());
    }
}

pub fn info(msg: &str) {
    write("INFO", msg);
}

pub fn warn(msg: &str) {
    write("WARN", msg);
}
