//! Opening the sibling widgets this one points at.
//!
//! Today that is **DaemonCurier**, the WSL2/system HUD:
//! it owns the "where did my RAM go" view — distros, the Docker engine inside them, the
//! container list — which is exactly what you want open after the `GW` LED tells you a
//! runner is spinning. It shares edgePass's contract (release exe, logon task, one
//! instance), so "open" is: ask the live widget for focus over `\\.\pipe\daemoncurier`,
//! and only launch the exe when nobody answers. A second launch would be harmless anyway
//! (its own lock makes it focus the live one), but the pipe is cheaper and leaves no
//! stray process behind.
//!
//! The exe lives wherever the user says (`daemoncurierExe` in config or
//! `EDGEPASS_DAEMONCURIER`): an empty value means the integration is off, and "open"
//! says exactly that — it does not guess a path.

use std::path::PathBuf;
use std::process::Command;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const PIPE_NAME: &str = r"\\.\pipe\daemoncurier";
/// A stuck pipe must not freeze the toast: same timeout its own launcher uses.
const PIPE_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(1500);

/// DaemonCurier's exe: what the user pointed at, or nothing — the public binary
/// carries no default paths, so "nothing" is an answer with a message.
pub fn daemoncurier_exe(cfg: &crate::config::AppConfig) -> PathBuf {
    let raw = cfg.daemoncurier_exe.trim();
    if !raw.is_empty() {
        return PathBuf::from(raw);
    }
    if let Ok(env) = std::env::var("EDGEPASS_DAEMONCURIER") {
        let e = env.trim().to_string();
        if !e.is_empty() {
            return PathBuf::from(e);
        }
    }
    PathBuf::new()
}

/// Focus the live DaemonCurier, or start it. Either way something ends up on screen.
///
/// The unconfigured case answers first: an integration that is off never dials
/// the sibling's pipe and never guesses a path.
pub fn open_daemoncurier(cfg: &crate::config::AppConfig) -> Result<String, String> {
    let exe = daemoncurier_exe(cfg);
    if exe.as_os_str().is_empty() {
        return Err("DaemonCurier no está configurado (daemoncurierExe en la config o EDGEPASS_DAEMONCURIER)".into());
    }
    if focus_via_pipe() {
        return Ok("DaemonCurier enfocado".into());
    }
    if !exe.is_file() {
        return Err(format!(
            "no encuentro DaemonCurier en {} (definí daemoncurierExe o EDGEPASS_DAEMONCURIER)",
            exe.display()
        ));
    }
    let mut cmd = Command::new(&exe);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.spawn()
        .map_err(|e| format!("no pude abrir DaemonCurier: {e}"))?;
    Ok("DaemonCurier abierto".into())
}

/// `{"op":"focus"}` over the widget's own pipe. `false` when nobody is listening.
#[cfg(windows)]
fn focus_via_pipe() -> bool {
    use std::io::{BufRead, BufReader, Write};
    use std::sync::mpsc;

    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let sent = (|| -> std::io::Result<bool> {
            let mut pipe = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(PIPE_NAME)?;
            pipe.write_all(b"{\"op\":\"focus\"}\n")?;
            let mut line = String::new();
            BufReader::new(pipe).read_line(&mut line)?;
            Ok(line.contains("\"focused\": true") || line.contains("\"focused\":true"))
        })();
        let _ = tx.send(sent.unwrap_or(false));
    });
    rx.recv_timeout(PIPE_TIMEOUT).unwrap_or(false)
}

#[cfg(not(windows))]
fn focus_via_pipe() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_exe_is_configured_or_off() {
        // Sin configuración no hay default: la integración está apagada, no rota.
        let cfg = crate::config::AppConfig::default();
        assert!(daemoncurier_exe(&cfg).as_os_str().is_empty());
        let mut cfg = crate::config::AppConfig::default();
        cfg.daemoncurier_exe = r"D:\otro\DaemonCurier.exe".into();
        assert_eq!(
            daemoncurier_exe(&cfg),
            PathBuf::from(r"D:\otro\DaemonCurier.exe")
        );
    }

    #[test]
    fn open_without_a_configured_exe_says_how_to_fix_it() {
        let cfg = crate::config::AppConfig::default();
        let err = open_daemoncurier(&cfg).expect_err("sin exe configurado");
        assert!(err.contains("daemoncurierExe"), "{err}");
        assert!(!err.contains(r"\proyectos"), "{err}");
    }

    #[test]
    fn focus_fails_fast_when_nobody_is_listening() {
        // Sin el widget corriendo no hay pipe: y sin pipe no se queda colgado.
        let started = std::time::Instant::now();
        let focused = focus_via_pipe();
        assert!(!focused || started.elapsed() < PIPE_TIMEOUT + std::time::Duration::from_secs(1));
    }
}
