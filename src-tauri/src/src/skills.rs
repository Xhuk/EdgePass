//! Bridge to `skills-sync`: the machine's skill library and its links.
//!
//! The program that does the work is Python and lives outside this repo, on its own.
//! Here it is only invoked and its answer is translated into a LED.
//!
//! Why outside and not inside edgePass: it synchronizes the *machine's* library
//! (`~/.agents/skills` of each agent), not something from this product. It is used the
//! same way from a terminal, from a hook or from edgePass. Contract: `docs/TOGGLE-CONTRACT.md`.
//!
//! Care points that do matter here:
//! - Every full `status` takes ~1 s (50 folders, 21 thousand links), so the LED reads
//!   from a **cache** and the refresh runs on a separate thread: polling cannot block the UI.
//! - The watcher (`watch`) is a **child process** killed when edgePass shuts down; the LED
//!   `skills.watch` reads its state without starting Python.
//! - A missing script or a Python not on PATH are reported as `warn` with the reason, never
//!   as a dead LED with no explanation.

use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// How long a `status` is good for before asking for another. The LED polls every 5 s.
const STATUS_TTL: Duration = Duration::from_secs(15);

/// The synchronizer lives wherever the user says: `EDGEPASS_SKILLS_SYNC` (to try
/// another checkout) or nothing — and "nothing" is the amber LED with the how, not a
/// path hidden in the binary.
pub fn script_path() -> PathBuf {
    match std::env::var("EDGEPASS_SKILLS_SYNC") {
        Ok(p) if !p.trim().is_empty() => PathBuf::from(p.trim()),
        _ => PathBuf::new(),
    }
}

fn script_ok(path: &std::path::Path) -> bool {
    !path.as_os_str().is_empty() && path.is_file()
}

/// What the LED needs: a handful of numbers and, if something failed, the reason.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillsStatus {
    /// The library and the links are in phase: `drift == 0`.
    pub insync: bool,
    pub drift: i64,
    pub faltan: i64,
    pub mal_apuntados: i64,
    pub muertos: i64,
    pub omitidos: i64,
    pub skills: i64,
    pub tipos: i64,
    pub anidadas: i64,
    pub agentes: i64,
    /// The watcher is running (live child process).
    pub watching: bool,
    /// The script is not where the default says, or Python failed.
    pub error: Option<String>,
    /// When it was read (for the tooltip: "3 s ago").
    pub age_secs: u64,
}

impl SkillsStatus {
    fn broken(msg: impl Into<String>) -> Self {
        SkillsStatus {
            error: Some(msg.into()),
            ..Default::default()
        }
    }
}

// ---------------------------------------------------------------------------
// Invocation
// ---------------------------------------------------------------------------

/// Runs `python <script> <args...>` and returns the **last** JSON object on stdout.
///
/// The synchronizer prints one JSON line per operation, but it may also write logs
/// before that; so the last parseable object is looked for, not the first line.
fn run_json(args: &[&str], timeout_secs: u64) -> Result<serde_json::Value, String> {
    let script = script_path();
    if !script_ok(&script) {
        return Err(match script.to_str().unwrap_or("") {
            "" => "synchronizer not configured (EDGEPASS_SKILLS_SYNC variable)".into(),
            other => format!("synchronizer not found at {other} (check EDGEPASS_SKILLS_SYNC)"),
        });
    }

    let mut cmd = Command::new("python");
    cmd.arg(&script);
    for a in args {
        cmd.arg(a);
    }
    cmd.current_dir(script.parent().unwrap_or(std::path::Path::new(".")));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("could not launch python ({}): {e}", script.display()))?;

    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Ok(None) => {
                let _ = child.kill();
                return Err(format!("the synchronizer did not finish within {timeout_secs} s"));
            }
            Err(e) => return Err(format!("waiting for the synchronizer: {e}")),
        }
    };

    let mut out = String::new();
    let mut err = String::new();
    if let Some(mut o) = child.stdout.take() {
        let _ = o.read_to_string(&mut out);
    }
    if let Some(mut e) = child.stderr.take() {
        let _ = e.read_to_string(&mut err);
    }

    let parsed = out
        .lines()
        .rev()
        .find_map(|line| serde_json::from_str::<serde_json::Value>(line.trim()).ok());

    match parsed {
        Some(value) => Ok(value),
        None if !status.success() => Err(format!(
            "the synchronizer exited with an error: {}",
            first_line(&err).unwrap_or_else(|| "no detail".into())
        )),
        None => Err("the synchronizer returned no JSON".into()),
    }
}

fn first_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(|l| l.chars().take(200).collect())
}

/// Translates the synchronizer's JSON into `SkillsStatus`. Tolerant of missing fields.
fn to_status(value: &serde_json::Value) -> SkillsStatus {
    let num = |key: &str| value.get(key).and_then(|v| v.as_i64()).unwrap_or(0);
    let omite = num("omitidos");
    SkillsStatus {
        insync: value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) && num("drift") == 0,
        drift: num("drift"),
        faltan: num("faltan"),
        mal_apuntados: num("mal_apuntados"),
        muertos: num("muertos"),
        omitidos: omite,
        skills: num("skills"),
        tipos: num("tipos"),
        anidadas: num("anidadas"),
        agentes: num("agentes"),
        watching: value
            .get("watching")
            .and_then(|v| v.as_bool())
            .unwrap_or_else(|| watch_running()),
        error: None,
        age_secs: 0,
    }
}

// ---------------------------------------------------------------------------
// Status cache (the LED cannot wait ~1 s)
// ---------------------------------------------------------------------------

struct Cache {
    at: Option<Instant>,
    value: Option<SkillsStatus>,
    refreshing: bool,
}

static CACHE: Mutex<Option<Cache>> = Mutex::new(None);

fn with_cache<T>(f: impl FnOnce(&mut Cache) -> T) -> T {
    let mut guard = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let cache = guard.get_or_insert(Cache {
        at: None,
        value: None,
        refreshing: false,
    });
    f(cache)
}

fn store_status(status: SkillsStatus) -> SkillsStatus {
    with_cache(|cache| {
        cache.value = Some(status.clone());
        cache.at = Some(Instant::now());
        cache.refreshing = false;
        status
    })
}

/// Reads the status: returns the last cached value **now** and, if expired, asks for a
/// refresh on a separate thread. Never blocks (which is why the LED can poll every 5 s).
pub fn status() -> SkillsStatus {
    let (cached, stale, busy) = with_cache(|cache| {
        let stale = cache
            .at
            .map(|at| at.elapsed() > STATUS_TTL)
            .unwrap_or(true);
        (cache.value.clone(), stale, cache.refreshing)
    });

    if stale && !busy {
        with_cache(|cache| cache.refreshing = true);
        std::thread::spawn(|| match run_json(&["status", "--json"], 20) {
            Ok(value) => {
                store_status(to_status(&value));
            }
            Err(msg) => {
                store_status(SkillsStatus::broken(msg));
            }
        });
    }

    match cached {
        Some(mut status) => {
            status.watching = watch_running();
            status
        }
        // First poll of this session: there is no reading yet. The LED shows "reading" and
        // the refresh is already on its way; the UI is not blocked for ~1 s.
        None => SkillsStatus {
            insync: false,
            watching: watch_running(),
            ..Default::default()
        },
    }
}

/// The real `status`, waiting. Used by the pipe (`{"op":"skills"}`) and the tests.
pub fn status_now() -> Result<SkillsStatus, String> {
    let value = run_json(&["status", "--json", "--verify"], 60)?;
    Ok(store_status(to_status(&value)))
}

// ---------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------

/// What a `sync` did: feeds the toast ("+50 links").
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncReport {
    pub creados: i64,
    pub arreglados: i64,
    pub retirados: i64,
    pub omitidos: i64,
}

/// Re-links every agent from the hub. Returns the report and the status already refreshed.
pub fn sync_now() -> Result<(SyncReport, SkillsStatus), String> {
    let value = run_json(&["sync", "--json"], 300)?;
    let num = |key: &str| value.get(key).and_then(|v| v.as_i64()).unwrap_or(0);
    let report = SyncReport {
        creados: num("creados"),
        arreglados: num("arreglados"),
        retirados: num("retirados"),
        omitidos: num("omitidos"),
    };
    let mut status = to_status(&value);
    status.watching = watch_running();
    Ok((report, store_status(status)))
}

// ---------------------------------------------------------------------------
// Watcher (edgePass child process)
// ---------------------------------------------------------------------------

static WATCHER: Mutex<Option<Child>> = Mutex::new(None);

fn with_watcher<T>(f: impl FnOnce(&mut Option<Child>) -> T) -> T {
    let mut guard = WATCHER.lock().unwrap_or_else(|e| e.into_inner());
    f(&mut guard)
}

/// The watcher lives as long as edgePass does: it is a child, not a service.
pub fn watch_running() -> bool {
    with_watcher(|slot| match slot {
        Some(child) => match child.try_wait() {
            Ok(None) => true,
            Ok(Some(_)) | Err(_) => {
                *slot = None;
                false
            }
        },
        None => false,
    })
}

/// Starts `watch` (auto-sync). Idempotent.
pub fn watch_on() -> Result<(), String> {
    if watch_running() {
        return Ok(());
    }
    let script = script_path();
    if !script_ok(&script) {
        return Err(format!("could not find the synchronizer at {}", script.display()));
    }
    let mut cmd = Command::new("python");
    cmd.arg(&script)
        .arg("watch")
        .arg("--interval")
        .arg("5")
        .current_dir(script.parent().unwrap_or(std::path::Path::new(".")))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let child = cmd
        .spawn()
        .map_err(|e| format!("could not launch the watcher: {e}"))?;
    with_watcher(|slot| *slot = Some(child));
    Ok(())
}

/// Stops the watcher.
pub fn watch_off() -> Result<(), String> {
    with_watcher(|slot| {
        if let Some(mut child) = slot.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    });
    Ok(())
}

/// Leaves the watcher as the action asks, and returns a line for the toast.
pub fn watch_set(on: bool) -> Result<String, String> {
    if on {
        watch_on()?;
        Ok("auto-sync on (every 5 s)".into())
    } else {
        watch_off()?;
        Ok("auto-sync off".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both tests touch `EDGEPASS_SKILLS_SYNC` (a **process** variable) and cargo runs them
    /// in parallel: they are serialized with this lock or one reads what the other wrote.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn script_path_honours_the_env_var() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("EDGEPASS_SKILLS_SYNC", r"C:\other\skills_sync.py");
        assert_eq!(script_path(), PathBuf::from(r"C:\other\skills_sync.py"));
        std::env::remove_var("EDGEPASS_SKILLS_SYNC");
        // With no variable there is no default: the binary does not hide another machine's path.
        assert!(script_path().as_os_str().is_empty());
    }

    #[test]
    fn to_status_reads_the_synchronizer_payload() {
        let payload = serde_json::json!({
            "ok": true, "drift": 0, "faltan": 0, "mal_apuntados": 0, "muertos": 0,
            "omitidos": 1, "skills": 423, "tipos": 15, "anidadas": 12, "agentes": 50,
            "watching": false
        });
        let status = to_status(&payload);
        assert!(status.insync);
        assert_eq!(status.skills, 423);
        assert_eq!(status.agentes, 50);
        assert_eq!(status.omitidos, 1);
        assert!(status.error.is_none());
    }

    #[test]
    fn to_status_marks_out_of_phase_when_there_is_drift() {
        let payload = serde_json::json!({"ok": false, "drift": 3, "faltan": 3});
        let status = to_status(&payload);
        assert!(!status.insync);
        assert_eq!(status.drift, 3);
    }

    #[test]
    fn to_status_does_not_break_on_empty_payload() {
        let status = to_status(&serde_json::json!({}));
        assert_eq!(status.drift, 0);
        assert!(status.error.is_none());
    }

    /// Real contract: if the synchronizer is present, it must answer JSON with the fields
    /// `to_status` reads. This is the test that would have caught a misspelled field name.
    /// On a machine without the tool (or without `python` on PATH) it skips itself.
    #[test]
    fn status_now_talks_to_the_synchronizer_when_present() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Without EDGEPASS_SKILLS_SYNC the script is not configured: that is this test's
        // "no tool" case, and it skips.
        let script = script_path();
        if script.as_os_str().is_empty() || !script.is_file() {
            eprintln!("synchronizer not configured: test skipped");
            return;
        }
        match status_now() {
            Ok(status) => {
                assert!(status.error.is_none(), "unexpected error: {:?}", status.error);
                assert!(status.skills > 0, "did not read skills: {status:?}");
                assert!(status.agentes > 0, "did not read agents: {status:?}");
                assert!(status.tipos > 0, "did not read types: {status:?}");
                assert_eq!(status.drift, 0, "the library should be in phase: {status:?}");
            }
            Err(msg) if msg.contains("could not launch python") => {
                eprintln!("python is not on PATH: test skipped ({msg})");
            }
            Err(msg) => panic!("the synchronizer failed: {msg}"),
        }
    }
}
