//! File logger.
//!
//! Release builds run with `windows_subsystem = "windows"`, `panic = "abort"`
//! and `strip = true`, so `eprintln!` goes nowhere and a failure leaves no
//! trace at all. Every decision the widget makes (seal/unseal, sync, IPC ops,
//! refusals) goes through here and lands in
//! `%APPDATA%\edgePass\logs\edgepass.log`, which is what the Diagnóstico panel
//! shows. No external crates: one file, one rotation, one panic hook.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// Rotate at this size, keeping one previous file.
const MAX_LOG_BYTES: u64 = 512 * 1024;
const PREV_FILE: &str = "edgepass.1.log";

static LOGGER: OnceLock<Mutex<Option<File>>> = OnceLock::new();

pub fn log_dir() -> PathBuf {
    crate::config::data_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("logs")
}

pub fn log_path() -> PathBuf {
    log_dir().join("edgepass.log")
}

fn logger() -> &'static Mutex<Option<File>> {
    LOGGER.get_or_init(|| Mutex::new(open_file()))
}

fn open_file() -> Option<File> {
    let path = log_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    rotate_if_needed(&path);
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .ok()
}

/// `edgepass.log` -> `edgepass.1.log`, dropping whatever was there.
fn rotate_if_needed(path: &Path) {
    let too_big = fs::metadata(path)
        .map(|m| m.len() > MAX_LOG_BYTES)
        .unwrap_or(false);
    if !too_big {
        return;
    }
    let prev = path.with_file_name(PREV_FILE);
    let _ = fs::remove_file(&prev);
    let _ = fs::rename(path, prev);
}

/// Timestamped single line. Never pass secret values in here.
pub fn line(level: &str, msg: &str) {
    let stamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
    let entry = format!("{stamp} {level:<5} {msg}\n");
    // `logger()` is a `std::sync::Mutex`, so a poisoned lock is a real
    // possibility: a panicking thread must not lose the log line *about* the
    // panic. Recover the guard instead of unwrapping.
    let guard = logger().lock();
    let mut guard = match guard {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    if let Some(file) = guard.as_mut() {
        let _ = file.write_all(entry.as_bytes());
        let _ = file.flush();
    }
    #[cfg(debug_assertions)]
    eprint!("{entry}");
}

/// Create the log file, write the session banner and hook panics.
pub fn init() {
    let _ = logger();
    install_panic_hook();
    let exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "?".into());
    line(
        "----",
        &format!(
            "edgePass {} arrancó (os={}, exe={})",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            exe
        ),
    );
    line("INFO", &format!("log en {}", log_path().display()));
}

/// A panic in a release build otherwise dies silently: `panic = "abort"`
/// kills the process before anything can be reported.
pub fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let where_ = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "?".into());
        let what = if let Some(s) = info.payload().downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "panic sin mensaje".into()
        };
        line("PANIC", &format!("{what} ({where_})"));
    }));
}

/// Last `n` lines of the current log, for the Diagnóstico panel.
pub fn tail(n: usize) -> String {
    let raw = fs::read_to_string(log_path()).unwrap_or_default();
    let lines: Vec<&str> = raw.lines().rev().take(n).collect();
    lines.into_iter().rev().collect::<Vec<_>>().join("\n")
}

/// Log-file path as a string, for the diagnostics report.
pub fn log_path_string() -> String {
    log_path().display().to_string()
}
