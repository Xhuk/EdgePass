//! Puente con `skills-sync`: la biblioteca de skills de la máquina y sus enlaces.
//!
//! El programa que hace el trabajo es Python y vive fuera de este repo, en su propia
//! Acá sólo se lo invoca y se traduce su respuesta a un LED.
//!
//! Por qué fuera y no adentro de edgePass: sincroniza la biblioteca de *la máquina*
//! (`~/.agents/skills` de cada agente), no algo de este producto. Se usa igual desde
//! una terminal, desde un hook o desde edgePass. Contrato: `docs/TOGGLE-CONTRACT.md`.
//!
//! Cuidados que sí importan acá:
//! - Cada `status` completo tarda ~1 s (50 carpetas, 21 mil enlaces), así que el LED lee
//!   de una **caché** y el refresco corre en un hilo aparte: sondear no puede bloquear la UI.
//! - El vigilante (`watch`) es un **proceso hijo** que se mata al apagar edgePass; el LED
//!   `skills.watch` lee su estado sin arrancar Python.
//! - Un script ausente o un Python sin PATH se reportan como `warn` con el motivo, nunca
//!   como un LED muerto sin explicación.

use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Cuánto vale un `status` antes de pedir otro. El LED se sonda cada 5 s.
const STATUS_TTL: Duration = Duration::from_secs(15);

/// El sincronizador vive donde el usuario lo diga: `EDGEPASS_SKILLS_SYNC` (para
/// probar otro checkout) o nada — y "nada" es el LED ámbar con el cómo, no una
/// ruta escondida en el binario.
pub fn script_path() -> PathBuf {
    match std::env::var("EDGEPASS_SKILLS_SYNC") {
        Ok(p) if !p.trim().is_empty() => PathBuf::from(p.trim()),
        _ => PathBuf::new(),
    }
}

fn script_ok(path: &std::path::Path) -> bool {
    !path.as_os_str().is_empty() && path.is_file()
}

/// Lo que el LED necesita: un puñado de números y, si algo falló, el motivo.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillsStatus {
    /// La biblioteca y los enlaces están en fase: `drift == 0`.
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
    /// El vigilante está corriendo (proceso hijo vivo).
    pub watching: bool,
    /// El script no está donde dice el default, o Python falló.
    pub error: Option<String>,
    /// Cuándo se leyó (para el tooltip: "hace 3 s").
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
// Invocación
// ---------------------------------------------------------------------------

/// Corre `python <script> <args...>` y devuelve el **último** objeto JSON de stdout.
///
/// El sincronizador imprime una línea JSON por operación, pero también puede escribir
/// logs antes; por eso se busca el último objeto parseable y no la primera línea.
fn run_json(args: &[&str], timeout_secs: u64) -> Result<serde_json::Value, String> {
    let script = script_path();
    if !script_ok(&script) {
        return Err(match script.to_str().unwrap_or("") {
            "" => "sincronizador no configurado (variable EDGEPASS_SKILLS_SYNC)".into(),
            other => format!("no encontré el sincronizador en {other} (revisá EDGEPASS_SKILLS_SYNC)"),
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
        .map_err(|e| format!("no pude lanzar python ({}): {e}", script.display()))?;

    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Ok(None) => {
                let _ = child.kill();
                return Err(format!("el sincronizador no terminó en {timeout_secs} s"));
            }
            Err(e) => return Err(format!("esperando al sincronizador: {e}")),
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
            "el sincronizador salió con error: {}",
            first_line(&err).unwrap_or_else(|| "sin detalle".into())
        )),
        None => Err("el sincronizador no devolvió JSON".into()),
    }
}

fn first_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(|l| l.chars().take(200).collect())
}

/// Traduce el JSON del sincronizador a `SkillsStatus`. Tolerante a campos ausentes.
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
// Caché del status (el LED no puede esperar ~1 s)
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

/// Lee el status: devuelve lo último cacheado **ya** y, si venció, pide el refresco en un
/// hilo aparte. Nunca bloquea (por eso el LED se puede sondear cada 5 s).
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
        // Primer sondeo de esta sesión: todavía no hay lectura. El LED muestra "leyendo" y
        // el refresco ya va en camino; no bloqueamos la UI por ~1 s.
        None => SkillsStatus {
            insync: false,
            watching: watch_running(),
            ..Default::default()
        },
    }
}

/// `status` de verdad, esperando. Lo usa el pipe (`{"op":"skills"}`) y las pruebas.
pub fn status_now() -> Result<SkillsStatus, String> {
    let value = run_json(&["status", "--json", "--verify"], 60)?;
    Ok(store_status(to_status(&value)))
}

// ---------------------------------------------------------------------------
// Acciones
// ---------------------------------------------------------------------------

/// Lo que hizo un `sync`: sirve para el toast ("+50 enlaces").
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncReport {
    pub creados: i64,
    pub arreglados: i64,
    pub retirados: i64,
    pub omitidos: i64,
}

/// Re-enlaza todos los agentes desde el hub. Devuelve el reporte y el status ya actualizado.
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
// Vigilante (proceso hijo de edgePass)
// ---------------------------------------------------------------------------

static WATCHER: Mutex<Option<Child>> = Mutex::new(None);

fn with_watcher<T>(f: impl FnOnce(&mut Option<Child>) -> T) -> T {
    let mut guard = WATCHER.lock().unwrap_or_else(|e| e.into_inner());
    f(&mut guard)
}

/// El vigilante vive mientras viva edgePass: es un hijo, no un servicio.
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

/// Arranca `watch` (auto-sincronización). Idempotente.
pub fn watch_on() -> Result<(), String> {
    if watch_running() {
        return Ok(());
    }
    let script = script_path();
    if !script_ok(&script) {
        return Err(format!("no encontré el sincronizador en {}", script.display()));
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
        .map_err(|e| format!("no pude lanzar el vigilante: {e}"))?;
    with_watcher(|slot| *slot = Some(child));
    Ok(())
}

/// Detiene el vigilante.
pub fn watch_off() -> Result<(), String> {
    with_watcher(|slot| {
        if let Some(mut child) = slot.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    });
    Ok(())
}

/// Deja el vigilante como pida la acción, y devuelve una línea para el toast.
pub fn watch_set(on: bool) -> Result<String, String> {
    if on {
        watch_on()?;
        Ok("auto-sincronización encendida (cada 5 s)".into())
    } else {
        watch_off()?;
        Ok("auto-sincronización apagada".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Los dos tests tocan `EDGEPASS_SKILLS_SYNC` (variable de **proceso**) y cargo los corre en
    /// paralelo: se serializan con este candado o uno lee lo que el otro escribió.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn script_path_respeta_la_variable_de_entorno() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("EDGEPASS_SKILLS_SYNC", r"C:\otro\skills_sync.py");
        assert_eq!(script_path(), PathBuf::from(r"C:\otro\skills_sync.py"));
        std::env::remove_var("EDGEPASS_SKILLS_SYNC");
        // Sin variable no hay default: el binario no esconde una ruta de otra máquina.
        assert!(script_path().as_os_str().is_empty());
    }

    #[test]
    fn to_status_lee_el_payload_del_sincronizador() {
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
    fn to_status_marca_fuera_de_fase_cuando_hay_drift() {
        let payload = serde_json::json!({"ok": false, "drift": 3, "faltan": 3});
        let status = to_status(&payload);
        assert!(!status.insync);
        assert_eq!(status.drift, 3);
    }

    #[test]
    fn to_status_no_se_cae_con_payload_vacio() {
        let status = to_status(&serde_json::json!({}));
        assert_eq!(status.drift, 0);
        assert!(status.error.is_none());
    }

    /// Contrato real: si el sincronizador está, tiene que contestar JSON con los campos que
    /// `to_status` lee. Es la prueba que habría cachado un nombre de campo mal escrito.
    /// En una máquina sin la herramienta (o sin `python` en PATH) se salta sola.
    #[test]
    fn status_now_habla_con_el_sincronizador_cuando_esta() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Sin EDGEPASS_SKILLS_SYNC el script no está configurado: es el caso "sin
        // la herramienta" de este test, y se salta.
        let script = script_path();
        if script.as_os_str().is_empty() || !script.is_file() {
            eprintln!("sincronizador no configurado: prueba saltada");
            return;
        }
        match status_now() {
            Ok(status) => {
                assert!(status.error.is_none(), "error inesperado: {:?}", status.error);
                assert!(status.skills > 0, "no leyó skills: {status:?}");
                assert!(status.agentes > 0, "no leyó agentes: {status:?}");
                assert!(status.tipos > 0, "no leyó tipos: {status:?}");
                assert_eq!(status.drift, 0, "la biblioteca debería estar en fase: {status:?}");
            }
            Err(msg) if msg.contains("no pude lanzar python") => {
                eprintln!("python no está en PATH: prueba saltada ({msg})");
            }
            Err(msg) => panic!("el sincronizador falló: {msg}"),
        }
    }
}
