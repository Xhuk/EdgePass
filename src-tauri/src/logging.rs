//! File logger.
//!
//! Release builds run with `windows_subsystem = "windows"`, `panic = "abort"`
//! and `strip = true`, so `eprintln!` goes nowhere and a failure leaves no
//! trace at all. Every decision the widget makes (seal/unseal, sync, IPC ops,
//! refusals) goes through here and lands in
//! `%APPDATA%\edgePass\logs\edgepass.log`, which is what the Diagnostics panel
//! shows. No external crates: one file, one rotation, one panic hook.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

/// Rotate at this size, keeping one previous file.
///
/// The check used to run only when the file was opened, so a widget left
/// running for days never rotated: one session produced a 14 MB file. The
/// counter below rotates mid-session too, which is the case that actually
/// happens — this widget is meant to stay up for weeks.
const MAX_LOG_BYTES: u64 = 4 * 1024 * 1024;
const PREV_FILE: &str = "edgepass.1.log";

/// Bytes in the current file, so rotation does not need a `metadata` call per
/// line. Seeded from the file's real size when it is opened.
static WRITTEN: AtomicU64 = AtomicU64::new(0);

static LOGGER: OnceLock<Mutex<Option<File>>> = OnceLock::new();

pub fn log_dir() -> PathBuf {
    // Local, not Roaming: a 4 MB log does not belong in the synced profile.
    crate::config::local_data_dir()
        .or_else(|_| crate::config::data_dir())
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
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .ok()?;
    // Continue counting from what is already on disk, so a restart does not
    // postpone the next rotation by another full file.
    let size = file.metadata().map(|m| m.len()).unwrap_or(0);
    WRITTEN.store(size, Ordering::Relaxed);
    Some(file)
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
    let mut must_rotate = false;
    if let Some(file) = guard.as_mut() {
        if file.write_all(entry.as_bytes()).is_ok() {
            let total = WRITTEN.fetch_add(entry.len() as u64, Ordering::Relaxed);
            must_rotate = total + entry.len() as u64 >= MAX_LOG_BYTES;
        }
        let _ = file.flush();
    }
    if must_rotate {
        // Rotating under the same lock keeps it race-free: no other thread can
        // be holding the file open while it is renamed.
        let path = log_path();
        let prev = path.with_file_name(PREV_FILE);
        let _ = fs::remove_file(&prev);
        let _ = fs::rename(&path, &prev);
        *guard = OpenOptions::new().create(true).append(true).open(&path).ok();
        WRITTEN.store(0, Ordering::Relaxed);
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
            "edgePass {} started (os={}, exe={})",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            exe
        ),
    );
    line("INFO", &format!("log at {}", log_path().display()));
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
            "panic without a message".into()
        };
        line("PANIC", &format!("{what} ({where_})"));
    }));
}

/// Last `n` lines of the current log, for the Diagnostics panel.
pub fn tail(n: usize) -> String {
    let raw = fs::read_to_string(log_path()).unwrap_or_default();
    let lines: Vec<&str> = raw.lines().rev().take(n).collect();
    lines.into_iter().rev().collect::<Vec<_>>().join("\n")
}

/// Log-file path as a string, for the diagnostics report.
pub fn log_path_string() -> String {
    log_path().display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("edgepass-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    #[test]
    fn rotation_moves_the_current_file_aside() {
        let dir = tmp_dir("rot");
        let path = dir.join("edgepass.log");
        if fs::write(&path, vec![b'x'; (MAX_LOG_BYTES + 1) as usize]).is_err() {
            return; // no space in the temp dir: not this test's business
        }
        rotate_if_needed(&path);
        assert!(!path.exists(), "the current file should have been moved");
        assert!(dir.join(PREV_FILE).exists(), "the previous file was not left behind");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_small_file_is_left_alone() {
        let dir = tmp_dir("rot-small");
        let path = dir.join("edgepass.log");
        fs::write(&path, b"hello\n").expect("test file");
        rotate_if_needed(&path);
        assert!(path.exists(), "a small file is left alone");
        assert!(!dir.join(PREV_FILE).exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
