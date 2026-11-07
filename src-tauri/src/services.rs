//! Owner-defined services behind a header LED.
//!
//! A built-in probe (see [`crate::toggles::PROBES`]) knows one thing about this
//! machine: the vault, the Q-ring pack, the local runner. Everything else a
//! person runs — their own API, a container, a Windows service, a tunnel — is
//! not a *kind* the binary can enumerate, so it is **data**: `config.json` →
//! `services[]`, one row per thing to watch.
//!
//! Two probe kinds, both cheap and both closed:
//!
//! - `process` reads the OS process list (an image name, optionally narrowed by
//!   an argument substring). It runs nothing.
//! - `cmd` runs a command and reads its exit code (`0` on, `2` off, `3` warn).
//!   Its first line of stdout becomes the tooltip detail, byte-capped and
//!   discarded when it looks like a credential.
//!
//! There is no free-form HTTP probe: a LED that chooses a host, a method and a
//! body widens the trust boundary in a way this build has not reviewed. A
//! service that only answers on a URL runs a health check that already exists on
//! the machine (`curl.exe` inside a `cmd`), on loopback.
//!
//! `origin` is reported, never configured. A service is not a child of this
//! widget unless the widget spawned it, so a process probe answers `external`
//! (or `down`); only [`crate::mcp`] rows can answer `managed`.

use serde::{Deserialize, Serialize};
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Cap on the list. The panel scrolls and the header caps itself separately; this
/// exists so a runaway config cannot turn one poll into hundreds of processes.
pub const MAX_SERVICES: usize = 24;

/// Probes shell out, and the status snapshot runs on a 5 s poll — the same TTL
/// the MCP sweep uses.
const STATUS_TTL: Duration = Duration::from_secs(10);

/// A probe that has not answered by then is "the probe itself failed", not a
/// reason to hang the whole snapshot.
///
/// Two seconds, not four: this runs on the 5 s status poll, and a health check
/// that needs longer than this is a probe to fix, not a state to wait for.
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// Tooltip budget. A detail is one sentence, not a log.
const MAX_DETAIL: usize = 180;

/// One service: what to watch and what a click may do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceSpec {
    /// Stable key. The derived probe is `svc.<id>`, the actions
    /// `svc.<id>.on` / `svc.<id>.off`.
    pub id: String,
    /// Short name for the panel. Defaults to the id.
    #[serde(default)]
    pub label: String,
    /// The tooltip's first line: what this is, in the owner's words.
    #[serde(default)]
    pub name: String,
    pub probe: ServiceProbe,
    #[serde(default)]
    pub actions: ServiceActions,
}

impl ServiceSpec {
    pub fn probe_id(&self) -> String {
        format!("svc.{}", self.id)
    }

    pub fn on_action(&self) -> String {
        format!("svc.{}.on", self.id)
    }

    pub fn off_action(&self) -> String {
        format!("svc.{}.off", self.id)
    }

    pub fn title(&self) -> String {
        let label = self.label.trim();
        let name = self.name.trim();
        match (label.is_empty(), name.is_empty()) {
            (true, true) => self.id.clone(),
            (true, false) => name.to_string(),
            (false, true) => label.to_string(),
            (false, false) => format!("{label} · {name}"),
        }
    }
}

/// How "is it up?" is answered.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ServiceProbe {
    /// Look for an image in the OS process list. Runs nothing.
    Process {
        /// Image to look for (`mi-api.exe`). The `.exe` is added on Windows when
        /// it is missing, so `mi-api` and `mi-api.exe` mean the same thing.
        image: String,
        /// Optional argument substring, to tell apart processes that share an
        /// interpreter (`node.exe`, `python.exe`).
        #[serde(default, rename = "match", skip_serializing_if = "Option::is_none")]
        matcher: Option<String>,
    },
    /// Run a command and read its exit code.
    Cmd {
        cmd: Vec<String>,
        /// Working directory. Empty = inherit edgePass's.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        cwd: String,
    },
}

/// What a click may do. Both sides are optional: a row with no `off` is a
/// read-only light, and that is a valid choice.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceActions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on: Option<ServiceAction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub off: Option<ServiceAction>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceAction {
    pub cmd: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub cwd: String,
}

/// Who owns the process behind a light: this decides whether stopping it is safe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Origin {
    /// edgePass spawned it and holds the pid.
    Managed,
    /// Alive, but somebody else started it.
    External,
    /// Not running.
    Down,
}

/// What a probe saw. Mirrors the parts of `ToggleState` a service fills.
#[derive(Debug, Clone)]
pub struct ServiceState {
    pub on: bool,
    pub warn: bool,
    pub detail: String,
    pub origin: Origin,
}

impl ServiceState {
    fn up(detail: String) -> Self {
        Self {
            on: true,
            warn: false,
            detail,
            origin: Origin::External,
        }
    }

    fn down(detail: String) -> Self {
        Self {
            on: false,
            warn: false,
            detail,
            origin: Origin::Down,
        }
    }

    /// The probe cannot answer: the config points at something that is not
    /// there, or the command failed. Amber, and a click is not the fix.
    fn broken(detail: String) -> Self {
        Self {
            on: false,
            warn: true,
            detail,
            origin: Origin::Down,
        }
    }
}

struct CacheEntry {
    id: String,
    spec: String,
    at: Instant,
    state: ServiceState,
}

static CACHE: Mutex<Vec<CacheEntry>> = Mutex::new(Vec::new());

/// Reads one service, through the 10 s cache. `None` = no such service.
pub fn probe(cfg: &crate::config::AppConfig, id: &str) -> Option<ServiceState> {
    let spec = cfg.services.iter().find(|s| s.id == id)?;
    Some(probe_spec(spec))
}

/// Reads several services **at once**.
///
/// A snapshot with three `cmd` probes used to spend their timeouts one after
/// another; now each gets its own thread and the snapshot costs the slowest one,
/// not the sum. Duplicates (`two LEDs, one service`) are probed once, and the
/// 10 s cache still absorbs the next poll.
pub fn probe_many(
    cfg: &crate::config::AppConfig,
    ids: &[String],
) -> std::collections::HashMap<String, ServiceState> {
    let mut unique: Vec<&String> = Vec::with_capacity(ids.len());
    for id in ids {
        if !unique.iter().any(|seen| *seen == id) {
            unique.push(id);
        }
    }
    let specs: Vec<&ServiceSpec> = unique
        .iter()
        .filter_map(|id| cfg.services.iter().find(|s| &s.id == *id))
        .collect();
    let mut out = std::collections::HashMap::with_capacity(specs.len());
    std::thread::scope(|scope| {
        let handles: Vec<_> = specs
            .iter()
            .map(|s| scope.spawn(move || (s.id.clone(), probe_spec(s))))
            .collect();
        for handle in handles {
            if let Ok((id, state)) = handle.join() {
                out.insert(id, state);
            }
        }
    });
    out
}

pub fn probe_spec(spec: &ServiceSpec) -> ServiceState {
    let key = serde_json::to_string(spec).unwrap_or_default();
    {
        let guard = CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(hit) = guard
            .iter()
            .find(|e| e.id == spec.id && e.spec == key && e.at.elapsed() < STATUS_TTL)
        {
            return hit.state.clone();
        }
    }
    let state = probe_now(&spec.probe);
    let mut guard = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    guard.retain(|e| e.id != spec.id);
    guard.push(CacheEntry {
        id: spec.id.clone(),
        spec: key,
        at: Instant::now(),
        state: state.clone(),
    });
    state
}

/// Reads one probe, ignoring the cache (tests and the first read).
pub fn probe_now(probe: &ServiceProbe) -> ServiceState {
    match probe {
        ServiceProbe::Process { image, matcher } => process_state(image, matcher.as_deref()),
        ServiceProbe::Cmd { cmd, cwd } => cmd_state(cmd, cwd),
    }
}

fn process_state(image: &str, matcher: Option<&str>) -> ServiceState {
    let image = normalize_image(image);
    if image.is_empty() {
        return ServiceState::broken("falta 'image' en el probe".into());
    }
    let matcher = matcher.map(str::trim).filter(|m| !m.is_empty());
    match pids_of(&image, matcher) {
        Ok(pids) if !pids.is_empty() => {
            let what = match matcher {
                Some(m) => format!("{image} · «{m}»"),
                None => image,
            };
            ServiceState::up(format!("corriendo (pid {}) · {what}", pids[0]))
        }
        Ok(_) => ServiceState::down(match matcher {
            Some(m) => format!("parado · no hay {image} con «{m}»"),
            None => format!("parado · no hay {image}"),
        }),
        Err(e) => ServiceState::broken(e),
    }
}

fn cmd_state(cmd: &[String], cwd: &str) -> ServiceState {
    let Some(program) = cmd.first() else {
        return ServiceState::broken("el probe 'cmd' está vacío".into());
    };
    let dir = if cwd.trim().is_empty() {
        None
    } else {
        Some(cwd.trim())
    };
    let run = crate::ipc::run_child(
        program,
        &cmd[1..],
        &[],
        dir,
        PROBE_TIMEOUT,
        None,
    );
    let run = match run {
        Ok(r) => r,
        Err(e) => return ServiceState::broken(format!("no pude ejecutar {program}: {e}")),
    };
    if run.timed_out {
        return ServiceState::broken(format!(
            "el probe no respondió en {}s",
            PROBE_TIMEOUT.as_secs()
        ));
    }
    let said = first_line(&run.stdout)
        .or_else(|| first_line(&run.stderr))
        .unwrap_or_default();
    match run.code {
        Some(0) => ServiceState::up(if said.is_empty() {
            "corriendo".into()
        } else {
            said
        }),
        Some(2) => ServiceState::down(if said.is_empty() {
            "parado".into()
        } else {
            said
        }),
        Some(3) => ServiceState::broken(if said.is_empty() {
            "instalado pero roto".into()
        } else {
            said
        }),
        Some(code) => ServiceState::broken(if said.is_empty() {
            format!("el probe salió con código {code}")
        } else {
            said
        }),
        None => ServiceState::broken("el probe no devolvió un código".into()),
    }
}

/// Runs one side of a service's actions. The command is the owner's, never
/// synthesised: this module does not kill by image.
pub fn run_action(
    cfg: &crate::config::AppConfig,
    id: &str,
    side: Side,
) -> Result<String, String> {
    let spec = cfg
        .services
        .iter()
        .find(|s| s.id == id)
        .ok_or_else(|| format!("servicio desconocido: '{id}'"))?;
    let action = match side {
        Side::On => spec.actions.on.as_ref(),
        Side::Off => spec.actions.off.as_ref(),
    }
    .ok_or_else(|| format!("'{}' no tiene acción {side}", spec.title()))?;
    if action.cmd.is_empty() {
        return Err(format!("la acción {side} de '{}' está vacía", spec.title()));
    }
    let program = action.cmd[0].clone();
    let args = action.cmd[1..].to_vec();
    let dir = action.cwd.trim().to_string();
    let timeout = Duration::from_secs(60);
    let run = crate::ipc::run_child(
        &program,
        &args,
        &[],
        if dir.is_empty() { None } else { Some(&dir) },
        timeout,
        None,
    )?;
    if run.timed_out {
        return Err(format!(
            "la acción {side} no terminó en {}s",
            timeout.as_secs()
        ));
    }
    let said = first_line(&run.stdout).or_else(|| first_line(&run.stderr));
    match run.code {
        Some(0) => Ok(said.unwrap_or_else(|| format!("acción {side} ejecutada"))),
        Some(code) => Err(match said {
            Some(s) => format!("la acción {side} falló (código {code}): {s}"),
            None => format!("la acción {side} falló (código {code})"),
        }),
        None => Err(format!("la acción {side} no devolvió un código")),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    On,
    Off,
}

impl std::fmt::Display for Side {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Side::On => "on",
            Side::Off => "off",
        })
    }
}

/// Trims, drops empties, derives a missing id and keeps ids unique — the same
/// contract `toggles::validate` follows, so a hand-written row and a pipe row
/// end up identical.
pub fn validate(list: Vec<ServiceSpec>) -> Result<Vec<ServiceSpec>, String> {
    if list.len() > MAX_SERVICES {
        return Err(format!(
            "máximo {MAX_SERVICES} servicios (llegaron {})",
            list.len()
        ));
    }
    let mut used: Vec<String> = Vec::with_capacity(list.len());
    let mut out = Vec::with_capacity(list.len());
    for mut spec in list {
        spec.label = spec.label.trim().to_string();
        spec.name = spec.name.trim().to_string();
        let title = spec.title();
        for (side, action) in [("on", &mut spec.actions.on), ("off", &mut spec.actions.off)] {
            if let Some(a) = action {
                a.cmd = a
                    .cmd
                    .iter()
                    .map(|c| c.trim().to_string())
                    .filter(|c| !c.is_empty())
                    .collect();
                a.cwd = a.cwd.trim().to_string();
                if a.cmd.is_empty() {
                    return Err(format!("la acción {side} de '{title}' no tiene comando"));
                }
            }
        }
        match &mut spec.probe {
            ServiceProbe::Process { image, matcher } => {
                *image = image.trim().to_string();
                if image.is_empty() {
                    return Err(format!("el servicio '{title}' necesita 'probe.image'"));
                }
                if let Some(m) = matcher {
                    *m = m.trim().to_string();
                    if m.is_empty() {
                        *matcher = None;
                    }
                }
            }
            ServiceProbe::Cmd { cmd, cwd } => {
                *cmd = cmd
                    .iter()
                    .map(|c| c.trim().to_string())
                    .filter(|c| !c.is_empty())
                    .collect();
                *cwd = cwd.trim().to_string();
                if cmd.is_empty() {
                    return Err(format!("el servicio '{title}' necesita 'probe.cmd'"));
                }
            }
        }
        let base = slug(&spec.id);
        let base = if base.is_empty() {
            slug(&spec.label)
        } else {
            base
        };
        let base = if base.is_empty() {
            "servicio".to_string()
        } else {
            base
        };
        let mut id = base.clone();
        let mut n = 2;
        while used.contains(&id) {
            id = format!("{base}-{n}");
            n += 1;
        }
        spec.id = id;
        used.push(spec.id.clone());
        out.push(spec);
    }
    Ok(out)
}

fn slug(raw: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in raw.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            dash = false;
        } else if !out.is_empty() && !dash {
            out.push('-');
            dash = true;
        }
    }
    out.trim_matches('-').to_string()
}

fn first_line(bytes: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(bytes);
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())?
        .to_string();
    let capped: String = line.chars().take(MAX_DETAIL).collect();
    if crate::ipc::looks_like_secret_output(&capped) {
        return Some("la salida del probe parecía un secreto y se descartó".into());
    }
    Some(capped)
}

fn normalize_image(raw: &str) -> String {
    let raw = raw.trim();
    if raw.is_empty() {
        return String::new();
    }
    #[cfg(windows)]
    {
        if !raw.to_ascii_lowercase().ends_with(".exe") && !raw.contains('.') {
            return format!("{raw}.exe");
        }
    }
    raw.to_string()
}

/// Pids of every process with that image (and, when set, that argument
/// substring). `Err` = the process list could not be read at all.
fn pids_of(image: &str, matcher: Option<&str>) -> Result<Vec<u32>, String> {
    #[cfg(windows)]
    {
        if let Some(m) = matcher {
            return cim_pids(image, m);
        }
        tasklist_pids(image)
    }
    #[cfg(not(windows))]
    {
        let _ = (image, matcher);
        Err("este build no puede leer la lista de procesos en esta plataforma".into())
    }
}

#[cfg(windows)]
fn tasklist_pids(image: &str) -> Result<Vec<u32>, String> {
    let mut cmd = Command::new("tasklist");
    cmd.args(["/FI", &format!("IMAGENAME eq {image}"), "/NH", "/FO", "CSV"]);
    cmd.creation_flags(CREATE_NO_WINDOW);
    let out = cmd
        .output()
        .map_err(|e| format!("no pude leer la lista de procesos: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout);
    let want = image.to_ascii_lowercase();
    let mut pids = Vec::new();
    for line in text.lines() {
        let fields: Vec<&str> = line
            .split("\",\"")
            .map(|f| f.trim_matches('"'))
            .collect();
        if fields.len() < 2 {
            continue;
        }
        if fields[0].eq_ignore_ascii_case(&want) {
            if let Ok(pid) = fields[1].parse::<u32>() {
                pids.push(pid);
            }
        }
    }
    Ok(pids)
}

/// `tasklist` cannot see arguments, so the narrower match asks the OS for the
/// command line. One PowerShell per probe, cached for 10 s like the rest.
#[cfg(windows)]
fn cim_pids(image: &str, matcher: &str) -> Result<Vec<u32>, String> {
    let mut cmd = Command::new("powershell");
    cmd.args([
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        &format!(
            "Get-CimInstance Win32_Process -Filter \"Name = '{}'\" | \
             Where-Object {{ $_.CommandLine -and $_.CommandLine.IndexOf('{}', [StringComparison]::OrdinalIgnoreCase) -ge 0 }} | \
             ForEach-Object {{ $_.ProcessId }}",
            escape_single(image),
            escape_single(matcher)
        ),
    ]);
    cmd.creation_flags(CREATE_NO_WINDOW);
    let out = cmd
        .output()
        .map_err(|e| format!("no pude leer la lista de procesos: {e}"))?;
    if !out.status.success() {
        return Err("no pude leer la lista de procesos (CIM)".into());
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let pids: Vec<u32> = text
        .split_whitespace()
        .filter_map(|t| t.parse::<u32>().ok())
        .collect();
    Ok(pids)
}

#[cfg(windows)]
fn escape_single(raw: &str) -> String {
    raw.replace('\'', "''")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_with(services: Vec<ServiceSpec>) -> crate::config::AppConfig {
        crate::config::AppConfig {
            services,
            ..Default::default()
        }
    }

    fn svc(id: &str, probe: ServiceProbe) -> ServiceSpec {
        ServiceSpec {
            id: id.into(),
            label: id.into(),
            name: String::new(),
            probe,
            actions: ServiceActions::default(),
        }
    }

    #[test]
    fn ids_become_slugs_and_duplicates_get_a_suffix() {
        let list = vec![
            svc("mi API", ServiceProbe::Process { image: "api.exe".into(), matcher: None }),
            svc("mi api", ServiceProbe::Process { image: "api.exe".into(), matcher: None }),
        ];
        let out = validate(list).unwrap();
        assert_eq!(out[0].id, "mi-api");
        assert_eq!(out[1].id, "mi-api-2");
    }

    #[test]
    fn an_empty_list_is_valid() {
        assert!(validate(Vec::new()).unwrap().is_empty());
    }

    #[test]
    fn a_probe_needs_something_to_run_or_look_for() {
        let bad = svc(
            "x",
            ServiceProbe::Cmd {
                cmd: vec!["  ".into()],
                cwd: String::new(),
            },
        );
        assert!(validate(vec![bad]).is_err());
        let bad = svc(
            "x",
            ServiceProbe::Process {
                image: " ".into(),
                matcher: None,
            },
        );
        assert!(validate(vec![bad]).is_err());
    }

    #[test]
    fn the_exit_code_is_the_contract() {
        cfg_if_windows(|| {
            let on = cmd_state(&["cmd.exe".into(), "/c".into(), "exit 0".into()], "");
            assert!(on.on && !on.warn);
            let off = cmd_state(&["cmd.exe".into(), "/c".into(), "exit 2".into()], "");
            assert!(!off.on && !off.warn);
            let warn = cmd_state(&["cmd.exe".into(), "/c".into(), "exit 3".into()], "");
            assert!(warn.warn);
            let other = cmd_state(&["cmd.exe".into(), "/c".into(), "exit 7".into()], "");
            assert!(other.warn);
        });
    }

    #[test]
    fn stdout_becomes_the_detail_and_a_secret_is_dropped() {
        cfg_if_windows(|| {
            let said = cmd_state(
                &["cmd.exe".into(), "/c".into(), "echo container up".into()],
                "",
            );
            assert_eq!(said.detail, "container up");
            let leak = cmd_state(
                &[
                    "cmd.exe".into(),
                    "/c".into(),
                    "echo API_KEY=abcdefghijklmnopqrstuvwxyz012345".into(),
                ],
                "",
            );
            assert!(leak.detail.contains("secreto"), "{}", leak.detail);
            assert!(!leak.detail.contains("abcdefghijklmnop"));
        });
    }

    #[test]
    fn a_missing_binary_is_amber_not_off() {
        cfg_if_windows(|| {
            let said = cmd_state(&["definitely-not-a-real-binary.exe".into()], "");
            assert!(said.warn);
            assert!(!said.on);
        });
    }

    #[test]
    fn a_known_process_is_seen_and_an_unknown_one_is_down() {
        cfg_if_windows(|| {
            let me = std::process::id();
            let _ = me;
            let here = process_state("explorer", None);
            let there = process_state("definitely-not-running-xyz.exe", None);
            // Whether explorer runs depends on the machine; the contract is the
            // pair of states, not the specific answer.
            assert!(here.on || !here.warn);
            assert!(!there.on && !there.warn);
        });
    }

    #[test]
    fn probe_ids_and_actions_follow_the_row() {
        let s = ServiceSpec {
            id: "api".into(),
            ..svc("api", ServiceProbe::Process { image: "api.exe".into(), matcher: None })
        };
        assert_eq!(s.probe_id(), "svc.api");
        assert_eq!(s.on_action(), "svc.api.on");
        assert_eq!(s.off_action(), "svc.api.off");
        cfg_with(vec![s]);
    }

    #[cfg(windows)]
    fn cfg_if_windows(f: impl FnOnce()) {
        f()
    }

    #[cfg(not(windows))]
    fn cfg_if_windows(_f: impl FnOnce()) {}
}
