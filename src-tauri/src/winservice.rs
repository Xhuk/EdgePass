//! OpenBao Windows service bridge (NSSM service name: OpenBao).

use serde::Serialize;
use std::process::Command;
use std::time::{Duration, Instant};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

const SERVICE_NAME: &str = "OpenBao";

/// `sc.exe` es un binario de consola y edgePass corre como app GUI
/// (`windows_subsystem = "windows"`, sin consola padre): sin este flag cada
/// spawn le asigna Windows una **consola nueva y visible**. El poll de estado
/// de la UI (cada 5s) hacía parpadear una ventana de cmd en bucle.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// TTL de la caché de estado usada por el poll de la UI. El servicio OpenBao
/// vive por su cuenta; la pastilla tolera 15s de latencia y así no se lanza un
/// proceso por tick.
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
                    "Servicio OpenBao en ejecución".into()
                } else {
                    format!("Servicio OpenBao: {state}")
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
    // `sc start` devuelve estado no-cero a veces incluso arrancando bien: el
    // estado real manda, lo re-consultamos abajo.
    let _ = sc(&["start", SERVICE_NAME]);
    // Espera acotada (~5s) sondeando cada 1s en lugar de 20 spawns seguidos.
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
            "No arrancó el servicio OpenBao (estado={}). ¿Necesitas Admin?",
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

/// Estado con caché corta: lo usa el poll de la UI (5s). No spawnea `sc.exe` en
/// cada tick y, como OpenBao corre por su cuenta, 15s de latencia en la pastilla
/// no cambian ninguna decisión (el auto-unseal va por HTTP, no por `sc.exe`).
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
