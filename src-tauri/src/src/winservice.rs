//! OpenBao Windows service bridge (NSSM service name: OpenBao).

use serde::Serialize;
use std::process::Command;
use std::time::{Duration, Instant};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

const SERVICE_NAME: &str = "OpenBao";

/// `sc.exe` is a console binary and edgePass runs as a GUI app
/// (`windows_subsystem = "windows"`, no parent console): without this flag every
/// spawn gets a **new, visible** console from Windows. The UI's status poll
/// (every 5s) made a cmd window flash in a loop.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Status cache TTL used by the UI poll. The OpenBao service runs on its own;
/// the pill tolerates 15s of latency and that way no process is launched per
/// tick.
const STATUS_CACHE_TTL: Duration = Duration::from_secs(15);

static STATUS_CACHE: parking_lot::Mutex<Option<(Instant, ServiceStatusDto)>> =
    parking_lot::Mutex::new(None);

/// Runs a console program without ever flashing a console window.
///
/// `sc.exe` and `reg.exe` are console binaries and edgePass is a GUI-subsystem
/// process with no console to inherit, so Windows allocates a fresh *visible*
/// console for every spawn unless this flag is set. Single point where any
/// hidden child process is created.
pub fn run_hidden(program: &str, args: &[&str]) -> Result<std::process::Output, String> {
    let mut cmd = Command::new(program);
    cmd.args(args);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd.output().map_err(|e| {
        format!(
            "{program} {} failed: {e}",
            args.first().copied().unwrap_or("")
        )
    })
}

fn sc(args: &[&str]) -> Result<std::process::Output, String> {
    run_hidden("sc.exe", args)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceStatusDto {
    pub name: String,
    pub installed: bool,
    pub running: bool,
    pub state: String,
    pub message: String,
}

fn sc_query() -> Result<String, String> {
    let out = sc(&["query", SERVICE_NAME])?;
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    if out.status.success() || text.contains("STATE") {
        Ok(text)
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        Err(format!("OpenBao service not found ({err})"))
    }
}

fn parse_state(query: &str) -> String {
    for line in query.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("STATE") {
            // STATE              : 4  RUNNING
            let upper = rest.to_uppercase();
            if upper.contains("RUNNING") {
                return "Running".into();
            }
            if upper.contains("STOPPED") {
                return "Stopped".into();
            }
            if upper.contains("START_PENDING") {
                return "StartPending".into();
            }
            if upper.contains("STOP_PENDING") {
                return "StopPending".into();
            }
            if upper.contains("PAUSED") {
                return "Paused".into();
            }
            return rest.trim().trim_start_matches(':').trim().to_string();
        }
    }
    "Unknown".into()
}

pub fn status() -> ServiceStatusDto {
    match sc_query() {
        Ok(q) => {
            let state = parse_state(&q);
            let running = state.eq_ignore_ascii_case("Running");
            ServiceStatusDto {
                name: SERVICE_NAME.into(),
                installed: true,
                running,
                state: state.clone(),
                message: if running {
                    "OpenBao service running".into()
                } else {
                    format!("OpenBao service: {state}")
                },
            }
        }
        Err(e) => ServiceStatusDto {
            name: SERVICE_NAME.into(),
            installed: false,
            running: false,
            state: "Missing".into(),
            message: e,
        },
    }
}

pub fn start() -> Result<ServiceStatusDto, String> {
    let cur = status();
    if cur.running {
        cache_store(&cur);
        return Ok(cur);
    }
    if !cur.installed {
        return Err(cur.message);
    }
    // `sc start` sometimes returns a non-zero state even when it starts fine:
    // the real state is what counts, so we re-query it below.
    let _ = sc(&["start", SERVICE_NAME]);
    // Bounded wait (~5s) probing every 1s instead of 20 spawns in a row.
    for tick in 0..20u32 {
        std::thread::sleep(Duration::from_millis(250));
        if tick % 4 != 3 {
            continue;
        }
        let s = status();
        if s.running {
            cache_store(&s);
            return Ok(s);
        }
    }
    let s = status();
    cache_store(&s);
    if s.running {
        Ok(s)
    } else {
        Err(format!(
            "The OpenBao service did not start (state={}). Do you need Admin?",
            s.state
        ))
    }
}

/// If service exists and is stopped, start it. Returns final status.
pub fn ensure_running() -> Result<ServiceStatusDto, String> {
    let s = status();
    if !s.installed {
        cache_store(&s);
        return Ok(s); // API-only mode; no service on this machine
    }
    if s.running {
        cache_store(&s);
        return Ok(s);
    }
    start()
}

/// Status with a short cache: used by the UI poll (5s). It does not spawn
/// `sc.exe` on every tick and, since OpenBao runs on its own, 15s of latency in
/// the pill changes no decision (auto-unseal goes over HTTP, not `sc.exe`).
pub fn status_cached() -> ServiceStatusDto {
    {
        let cache = STATUS_CACHE.lock();
        if let Some((at, dto)) = cache.as_ref() {
            if at.elapsed() < STATUS_CACHE_TTL {
                return dto.clone();
            }
        }
    }
    let fresh = status();
    cache_store(&fresh);
    fresh
}

fn cache_store(dto: &ServiceStatusDto) {
    *STATUS_CACHE.lock() = Some((Instant::now(), dto.clone()));
}
