//! Lightweight Q-ring (qringd) probe + on/off/restart for the header quick action.
//!
//! Does not talk HTTP/:7437. Status = `qringd.exe` alive (and optional host pid
//! files). Start/stop shells out to `python qring.py enable|disable`, which
//! already dials this widget's named pipe for vault hold/unseal.
//!
//! **Where process control meets policy.** `off` is the one state whose policy is a
//! lie — a stopped gateway that still says "the rings may run" — so every stop pulls
//! `qring_governed` with it ([`policy_after_stop`], written from the `off` arm of
//! [`control`]). The other direction (policy ON over a stopped gateway) is the LED
//! click's job, not this module's: see `toggles::govern_action`.

use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// The pack only exists where the user points at it (`qringPack` in config or
/// `EDGEPASS_QRING_PACK`). An empty value means the integration is off; the
/// LED says so instead of guessing a path that belongs to another machine.
fn unconfigured_msg() -> String {
    "pack de Q-ring no configurado (qringPack en la config o EDGEPASS_QRING_PACK)".into()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QringStatusDto {
    pub running: bool,
    pub pack: String,
    pub host: String,
    pub pack_ok: bool,
    pub pid: Option<u32>,
    pub message: String,
    /// Policy: may rings RUN. Deliberately separate from `running`, which is process control.
    pub governed: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QringGovernDto {
    pub ok: bool,
    pub enabled: bool,
    pub source: String,
}

/// Governance, not process control: whether rings may RUN at all. Persisted so the pack reads
/// it over the pipe (`{"op":"qring"}`) and refuses to fire — and to spend — while it is false.
pub fn govern(enabled: bool) -> Result<QringGovernDto, String> {
    let mut cfg = crate::config::load_config()?;
    cfg.qring_governed = enabled;
    crate::config::save_config(&cfg)?;
    Ok(QringGovernDto { ok: true, enabled, source: "edgepass:config".into() })
}

pub fn governed() -> bool {
    crate::config::load_config().map(|c| c.qring_governed).unwrap_or(true)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QringControlDto {
    pub ok: bool,
    pub action: String,
    pub running: bool,
    pub message: String,
}

pub fn paths_from_config(pack: &str, host: &str) -> (PathBuf, PathBuf) {
    let pack = resolve_pack(pack);
    let host = resolve_host(host, &pack);
    (pack, host)
}

fn resolve_pack(raw: &str) -> PathBuf {
    let trimmed = raw.trim();
    if !trimmed.is_empty() {
        return PathBuf::from(trimmed);
    }
    if let Ok(env) = std::env::var("EDGEPASS_QRING_PACK") {
        let e = env.trim();
        if !e.is_empty() {
            return PathBuf::from(e);
        }
    }
    PathBuf::new()
}

fn resolve_host(raw: &str, pack: &Path) -> PathBuf {
    let trimmed = raw.trim();
    if !trimmed.is_empty() {
        return PathBuf::from(trimmed);
    }
    if let Ok(env) = std::env::var("EDGEPASS_QRING_HOST") {
        let e = env.trim();
        if !e.is_empty() {
            return PathBuf::from(e);
        }
    }
    if let Ok(env) = std::env::var("QRING_HOST") {
        let e = env.trim();
        if !e.is_empty() {
            return PathBuf::from(e);
        }
    }
    // Match Python gate_host: prefer pack parent when it holds edge-operator.
    let parent = pack.parent().map(Path::to_path_buf).unwrap_or_else(|| pack.to_path_buf());
    if parent.join(".qring").join("edge-operator.json").is_file() {
        return parent;
    }
    parent
}

pub fn status(pack_cfg: &str, host_cfg: &str) -> QringStatusDto {
    let (pack, host) = paths_from_config(pack_cfg, host_cfg);
    let pack_ok = pack.join("qring.py").is_file();
    let pid = read_runtime_pid(&host).filter(|p| pid_alive(*p));
    let running = qringd_process_alive() || pid.is_some();
    let message = if pack.as_os_str().is_empty() {
        unconfigured_msg()
    } else if !pack_ok {
        format!("pack missing: {}", pack.display())
    } else if running {
        match pid {
            Some(p) => format!("qringd up · pid {p}"),
            None => "qringd up".into(),
        }
    } else {
        "qringd down".into()
    };
    QringStatusDto {
        running,
        pack: pack.display().to_string(),
        host: host.display().to_string(),
        pack_ok,
        pid,
        message,
        governed: governed(),
    }
}

pub fn control(pack_cfg: &str, host_cfg: &str, action: &str) -> Result<QringControlDto, String> {
    let action = action.trim().to_ascii_lowercase();
    let (pack, host) = paths_from_config(pack_cfg, host_cfg);
    if pack.as_os_str().is_empty() {
        return Err(unconfigured_msg());
    }
    if !pack.join("qring.py").is_file() {
        return Err(format!("Q-ring pack not found: {}", pack.display()));
    }

    match action.as_str() {
        "on" | "enable" | "start" => {
            let msg = enable(&pack, &host)?;
            let st = status(pack_cfg, host_cfg);
            Ok(QringControlDto {
                ok: true,
                action: "on".into(),
                running: st.running,
                message: msg,
            })
        }
        "off" | "disable" | "stop" => {
            let msg = disable(&pack, &host)?;
            // Brief settle so tasklist reflects the kill.
            std::thread::sleep(Duration::from_millis(400));
            let st = status(pack_cfg, host_cfg);
            Ok(QringControlDto {
                ok: true,
                action: "off".into(),
                running: st.running,
                message: with_policy_off(msg),
            })
        }
        "restart" | "rerun" | "re-run" => {
            let _ = disable(&pack, &host);
            std::thread::sleep(Duration::from_millis(600));
            let msg = enable(&pack, &host)?;
            let st = status(pack_cfg, host_cfg);
            Ok(QringControlDto {
                ok: true,
                action: "restart".into(),
                running: st.running,
                message: format!("restarted · {msg}"),
            })
        }
        "toggle" => {
            if status(pack_cfg, host_cfg).running {
                control(pack_cfg, host_cfg, "off")
            } else {
                control(pack_cfg, host_cfg, "on")
            }
        }
        other => Err(format!("unknown qring action: {other}")),
    }
}

/// The config a stop leaves behind, or `None` when there is nothing to write.
///
/// Pure on purpose: the coupling is a contract ("no gateway ⇒ no policy"), and it
/// is the only half of it that can be tested without spawning python or touching
/// the real `config.json`.
pub fn policy_after_stop(cfg: &crate::config::AppConfig) -> Option<crate::config::AppConfig> {
    if !cfg.qring_governed {
        return None;
    }
    let mut next = cfg.clone();
    next.qring_governed = false;
    Some(next)
}

/// Writes that config change, folding the outcome into the CLI message.
///
/// Best effort *on purpose*: the gateway is already down by the time this runs, so
/// a config that cannot be written is reported in the message instead of turning a
/// stop that did happen into an error.
fn with_policy_off(msg: String) -> String {
    let cfg = match crate::config::load_config() {
        Ok(cfg) => cfg,
        Err(e) => return format!("{msg} · no pude leer la config para apagar la política: {e}"),
    };
    match policy_after_stop(&cfg) {
        Some(next) => match crate::config::save_config(&next) {
            Ok(()) => format!("{msg} · política apagada"),
            Err(e) => format!("{msg} · no pude apagar la política: {e}"),
        },
        None => msg,
    }
}

fn enable(pack: &Path, host: &Path) -> Result<String, String> {
    let out = run_cli(pack, host, &["enable", "--host", &host.display().to_string()], 90)?;
    let msg = summarize_cli("enable", &out);
    if out.code == 0 {
        Ok(msg)
    } else if out.code == 2 {
        Err(msg)
    } else {
        Err(msg)
    }
}

fn disable(pack: &Path, host: &Path) -> Result<String, String> {
    let out = run_cli(pack, host, &["disable", "--host", &host.display().to_string()], 20)?;
    // Belt-and-suspenders: stop any leftover qringd for this pack.
    let _ = kill_qringd_for_pack(pack);
    let msg = summarize_cli("disable", &out);
    if out.code == 0 {
        Ok(msg)
    } else {
        // Still killed processes; treat as soft success if qringd is gone.
        if !qringd_process_alive() {
            Ok(format!("{msg} · qringd stopped"))
        } else {
            Err(msg)
        }
    }
}

fn summarize_cli(op: &str, out: &CliOut) -> String {
    let last = out
        .stdout
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    if out.code == 0 {
        if last.is_empty() {
            format!("{op} ok")
        } else {
            last.to_string()
        }
    } else if out.code == 2 {
        // Token / more setup needed — surface the last QRING_* line.
        if last.is_empty() {
            format!("{op} needs setup (exit 2)")
        } else {
            last.to_string()
        }
    } else if last.is_empty() {
        format!("{op} failed (exit {})", out.code)
    } else {
        format!("{op} failed · {last}")
    }
}

struct CliOut {
    code: i32,
    stdout: String,
}

fn run_cli(pack: &Path, host: &Path, args: &[&str], timeout_secs: u64) -> Result<CliOut, String> {
    let script = pack.join("qring.py");
    let py = python_bin()?;
    let program = py.program.clone();
    let mut cmd = Command::new(&program);
    for a in &py.prefix {
        cmd.arg(a);
    }
    cmd.arg(&script).args(args).current_dir(host);
    cmd.env("QRING_HOST", host.display().to_string());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("spawn qring ({program}): {e}"))?;

    let deadline = std::time::Instant::now() + Duration::from_secs(timeout_secs);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut stdout = String::new();
                if let Some(mut out) = child.stdout.take() {
                    use std::io::Read;
                    let _ = out.read_to_string(&mut stdout);
                }
                if let Some(mut err) = child.stderr.take() {
                    use std::io::Read;
                    let mut e = String::new();
                    let _ = err.read_to_string(&mut e);
                    if !e.trim().is_empty() {
                        if !stdout.is_empty() {
                            stdout.push('\n');
                        }
                        stdout.push_str(&e);
                    }
                }
                let code = status.code().unwrap_or(-1);
                crate::log_info!("qring {} exit={} · {}", args.first().unwrap_or(&"?"), code, stdout.lines().last().unwrap_or(""));
                return Ok(CliOut { code, stdout });
            }
            Ok(None) => {
                if std::time::Instant::now() > deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!(
                        "qring {} timed out after {timeout_secs}s",
                        args.first().unwrap_or(&"?")
                    ));
                }
                std::thread::sleep(Duration::from_millis(200));
            }
            Err(e) => return Err(format!("qring wait: {e}")),
        }
    }
}

struct PythonBin {
    program: String,
    prefix: Vec<String>,
}

fn python_bin() -> Result<PythonBin, String> {
    // Prefer `python` when it works; fall back to the Windows `py -3` launcher.
    for (program, prefix) in [("python", vec![]), ("py", vec!["-3".into()])] {
        let mut cmd = Command::new(program);
        for a in &prefix {
            cmd.arg(a);
        }
        cmd.arg("-c").arg("import sys; print(sys.executable)");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        if let Ok(out) = cmd.output() {
            if out.status.success() {
                let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !path.is_empty() {
                    return Ok(PythonBin {
                        program: program.into(),
                        prefix,
                    });
                }
            }
        }
    }
    Err("python not on PATH".into())
}

fn read_runtime_pid(host: &Path) -> Option<u32> {
    let path = host.join(".qring").join("runtime.pid");
    let raw = std::fs::read_to_string(path).ok()?;
    raw.trim().parse().ok()
}

fn pid_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                return false;
            }
            CloseHandle(h);
            true
        }
    }
    #[cfg(not(windows))]
    {
        let _ = pid;
        false
    }
}

fn qringd_process_alive() -> bool {
    #[cfg(windows)]
    {
        let mut cmd = Command::new("tasklist");
        cmd.args(["/FI", "IMAGENAME eq qringd.exe", "/NH"]);
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
        match cmd.output() {
            Ok(out) => {
                let s = String::from_utf8_lossy(&out.stdout).to_ascii_lowercase();
                s.contains("qringd.exe")
            }
            Err(_) => false,
        }
    }
    #[cfg(not(windows))]
    {
        false
    }
}

fn kill_qringd_for_pack(pack: &Path) -> bool {
    #[cfg(windows)]
    {
        let pack_norm = pack.display().to_string().replace('/', "\\").to_ascii_lowercase();
        let mut cmd = Command::new("powershell");
        cmd.args([
            "-NoProfile",
            "-Command",
            &format!(
                "$pack = '{}'; Get-CimInstance Win32_Process -Filter \"Name = 'qringd.exe'\" | Where-Object {{ $_.CommandLine -and $_.CommandLine.ToLower().Contains($pack.ToLower()) }} | ForEach-Object {{ Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }}",
                pack_norm.replace('\'', "''")
            ),
        ]);
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
        cmd.status().map(|s| s.success()).unwrap_or(false)
    }
    #[cfg(not(windows))]
    {
        let _ = pack;
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;

    #[test]
    fn stopping_the_gateway_pulls_the_policy() {
        let governed = AppConfig::default();
        assert!(governed.qring_governed, "el default permite correr");

        let next = policy_after_stop(&governed).expect("un stop con la política ON escribe");
        assert!(
            !next.qring_governed,
            "sin gateway no hay política que valga"
        );
        assert!(
            governed.qring_governed,
            "el helper no toca el original: quien llama decide si guarda"
        );

        let mut already_off = AppConfig::default();
        already_off.qring_governed = false;
        assert!(
            policy_after_stop(&already_off).is_none(),
            "sin nada que apagar no hay escritura"
        );
    }

    #[test]
    fn an_unconfigured_pack_refuses_with_a_useful_message() {
        // Sin pack declarado, ni el LED ni el click deben adivinar una ruta.
        let cfg = AppConfig::default();
        let st = status(&cfg.qring_pack, &cfg.qring_host);
        assert!(!st.pack_ok, "sin pack no puede estar ok");
        assert!(
            st.message.contains("no configurado"),
            "mensaje con el cómo: {}",
            st.message
        );
        assert!(
            !st.message.contains(r"\proyectos"),
            "nada del binario pierde una ruta de otra máquina: {}",
            st.message
        );
        let err = control(&cfg.qring_pack, &cfg.qring_host, "on").expect_err("sin pack");
        assert!(err.contains("no configurado"), "{err}");
    }
}
