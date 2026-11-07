use crate::toggles;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

const KEYRING_SERVICE: &str = "edgepass";
const KEYRING_USER: &str = "bao_token";

/// The UI polls status every 5s and status carries `hasToken`, so the OS
/// keyring was being read 12 times a minute. Short TTL, invalidated on write.
const TOKEN_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(20);

static TOKEN_CACHE: parking_lot::Mutex<Option<(std::time::Instant, Option<String>)>> =
    parking_lot::Mutex::new(None);

/// The vault init file, **known only when the user configures it**.
///
/// A public binary carries no paths of the machine it was built on: an install
/// starts with this empty and Settings → Vault asks for it (or the user pastes
/// a token directly). It is remembered only because the user typed it.
fn default_true() -> bool {
    true
}

/// Schema version written into `config.json`. Absent/0 = legacy config.
pub const CONFIG_VERSION: u32 = 4;

/// **Never** auto-seal by default. Sealing is a vault-wide side effect: it cuts
/// off every other consumer of the shared OpenBao service, so it must not be
/// triggered by *this widget* going quiet. 0 = never; set seconds to opt in.
fn default_idle_secs() -> u64 {
    0
}

/// Collapse the floating panel after this many seconds without the pointer
/// over the window. Matches the “~10s after mouse leave” expectation; 0 = off.
fn default_collapse_idle_secs() -> u64 {
    10
}

/// The idle value v1 shipped as its default. Kept so the migration can tell
/// "user never touched this" from "user deliberately chose 300".
const LEGACY_IDLE_SECS: u64 = 300;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    pub bao_addr: String,
    pub mount: String,
    pub namespaces: Vec<String>,
    /// Unseal automatically when a secret op needs the vault.
    #[serde(default = "default_true")]
    pub auto_unseal: bool,
    /// After this many idle seconds while open, seal again. 0 = never auto-seal.
    #[serde(default = "default_idle_secs")]
    pub auto_seal_idle_secs: u64,
    /// Collapse the expanded panel after this many seconds without pointer
    /// hover. 0 = never. Independent of vault auto-seal.
    #[serde(default = "default_collapse_idle_secs")]
    pub auto_collapse_idle_secs: u64,
    #[serde(default = "default_true")]
    pub auto_sync: bool,
    #[serde(default = "default_true")]
    pub auto_load_token: bool,
    /// Path to the vault init file (unseal keys + root token), only if the user
    /// gave one: empty = not configured, and Settings → Vault asks for it. A
    /// token can also be pasted directly, in which case this stays empty.
    #[serde(default)]
    pub init_path: String,
    /// Absolute path to the Q-ring pack (`qring.py` + `bin/qringd.exe`).
    /// Empty = the integration is off: no default hides in the binary. Set it
    /// here or via `EDGEPASS_QRING_PACK`.
    #[serde(default)]
    pub qring_pack: String,
    /// Gate host for `qring.py enable|disable`. Empty = pack parent / env.
    #[serde(default)]
    pub qring_host: String,
    /// Governance: may Q-ring RUN at all? The pack reads this over the pipe before every
    /// expensive path, so `false` stops rings — and their spend — even while the gateway is up.
    /// Written by the header GOV toggle, by `{"op":"qring","enabled":bool}`, and by *stopping*
    /// the gateway (a stopped gateway that still claims the rings may run is the one state
    /// nobody can honour — see `qring::policy_after_stop`).
    #[serde(default = "default_true")]
    pub qring_governed: bool,
    /// The header LEDs, as data (see the `toggles` module). Capped by
    /// `toggles::MAX_TOGGLES`; edited from Settings or over the pipe
    /// (`{"op":"toggles","set":[…]}`, validated against the published catalog).
    ///
    /// A file written before this field existed showed exactly three hardcoded
    /// LEDs, so it gets `legacy_toggles` back — an upgrade must not rearrange
    /// anyone's header. A *fresh* install ships only the vault LED: the Q-ring
    /// pair is in the catalog, but an install with no pack should not open amber.
    #[serde(default = "toggles::legacy_toggles")]
    pub toggles: Vec<toggles::ToggleSpec>,
    /// MCP servers this widget starts, watches and restarts (see the `mcp`
    /// module). Data, like the LEDs: the MCP panel renders from it, the `MCP`
    /// LED supervises it, and the rows flagged `autostart` come up with the
    /// widget — which is already launched at logon by the `edgePass` task.
    #[serde(default)]
    pub mcp_servers: Vec<crate::mcp::McpServer>,
    /// Owner-defined services behind a LED (see the `services` module).
    ///
    /// Each row is one thing this machine runs that the binary does not know
    /// about: a container, a local API, a tunnel. The derived probe is
    /// `svc.<id>` and the derived actions are `svc.<id>.on` / `.off`, so a
    /// `toggles[]` row can name any of them without the exe changing.
    #[serde(default)]
    pub services: Vec<crate::services::ServiceSpec>,
    /// Local GitHub-Actions runners this widget keeps alive (see the `gha` module).
    ///
    /// Generic on purpose: the image is reusable, so a project is a row — repo, labels
    /// and (optionally) the OpenBao credentials the runner gets in its environment.
    /// Empty means nothing declared, and the `GW` LED says exactly that instead of
    /// showing a green light over nothing.
    #[serde(default)]
    pub gha_runners: Vec<crate::gha::GhaRunnerSpec>,
    /// Where the runner's `Dockerfile` + compose live (the assets ship with the repo).
    /// Empty = not configured: the runner actions refuse with that message instead
    /// of guessing a directory. Set it here or via `EDGEPASS_GHA_DIR`.
    #[serde(default)]
    pub gha_dir: String,
    /// Docker Desktop's binary, started when the engine does not answer. Empty =
    /// `C:\Program Files\Docker\Docker\Docker Desktop.exe` or `EDGEPASS_DOCKER_DESKTOP`.
    #[serde(default)]
    pub docker_desktop_exe: String,
    /// DaemonCurier's widget (the WSL2/system HUD the `GW` alt opens). Empty =
    /// not configured: "open" refuses with that message. Set it here or via
    /// `EDGEPASS_DAEMONCURIER`.
    #[serde(default)]
    pub daemoncurier_exe: String,
    /// `cloudflared` binary, used to run the named tunnels an MCP row declares
    /// (see [`crate::mcp::McpServer::tunnel`]). Empty = `cloudflared` on `PATH`
    /// or `EDGEPASS_CLOUDFLARED`.
    ///
    /// Worth setting: the MSI/winget install drops it in
    /// `%LOCALAPPDATA%\cloudflared` and does **not** add it to `PATH`, which is
    /// why a hand-typed `cloudflared tunnel run …` fails on a fresh machine.
    #[serde(default)]
    pub cloudflared_exe: String,
    /// Open the runner's log viewer by itself when a job starts.
    ///
    /// On by default: the whole point is *not* having to go look at GitHub to find out
    /// that something is running. Off = no window pops while you work (the LED still
    /// says which job is in flight, and `gha-logs` still answers).
    #[serde(default = "default_true")]
    pub gha_watch: bool,
    /// Written by this version. 0 means the file predates the seal-ownership
    /// change and gets migrated on first load.
    #[serde(default)]
    pub config_version: u32,
    /// The style of the suggested passphrases (see `passphrase`).
    ///
    /// `configured` flips the first time a person picks it, so the widget asks
    /// once and remembers. Until then the defaults apply but the UI still asks.
    #[serde(default)]
    pub passphrase: crate::passphrase::PassphraseConfig,
    /// Per-installation salt (hex, 16 bytes) mixed into the passphrase draw.
    ///
    /// Not a secret and not extra entropy: it rotates the vocabulary mapping so
    /// two machines cloned from the same snapshot do not draw the same words.
    /// Generated on the first passphrase and kept here; empty = not generated.
    #[serde(default)]
    pub install_salt: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            bao_addr: "http://127.0.0.1:8200".into(),
            mount: "kv".into(),
            namespaces: vec!["".into()],
            auto_unseal: true,
            auto_seal_idle_secs: default_idle_secs(),
            auto_collapse_idle_secs: default_collapse_idle_secs(),
            auto_sync: true,
            auto_load_token: true,
            init_path: String::new(),
            qring_pack: String::new(),
            qring_host: String::new(),
            qring_governed: true,
            toggles: toggles::default_toggles(),
            mcp_servers: Vec::new(),
            services: Vec::new(),
            gha_runners: Vec::new(),
            gha_dir: String::new(),
            docker_desktop_exe: String::new(),
            daemoncurier_exe: String::new(),
            cloudflared_exe: String::new(),
            gha_watch: true,
            passphrase: crate::passphrase::PassphraseConfig::default(),
            install_salt: String::new(),
            config_version: CONFIG_VERSION,
        }
    }
}

pub fn data_dir() -> Result<PathBuf, String> {
    let base = dirs::data_dir().ok_or("could not resolve data dir")?;
    let dir = base.join("edgePass");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// Machine-local data: the index, the logs and the placement.
///
/// `%APPDATA%` roams with the profile; a SQLite index and a 4 MB log should not
/// travel to another machine. Config and the keychain entry stay in [`data_dir`]
/// (they are the things a person expects to follow them).
pub fn local_data_dir() -> Result<PathBuf, String> {
    let base = dirs::data_local_dir()
        .or_else(dirs::data_dir)
        .ok_or("could not resolve local data dir")?;
    let dir = base.join("edgePass");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// The config file in use.
///
/// `config.yaml` is the documented format from this version on; `config.json` is
/// what every install before it wrote. Whichever exists is the one read **and
/// written**, so nobody's settings move under them (a migration is an explicit
/// act: `edgepass migrate`). A fresh install gets YAML.
pub fn config_path() -> Result<PathBuf, String> {
    let dir = data_dir()?;
    let yaml = dir.join("config.yaml");
    if yaml.exists() {
        return Ok(yaml);
    }
    let json = dir.join("config.json");
    if json.exists() {
        return Ok(json);
    }
    Ok(yaml)
}

/// `true` when that path is YAML (`config.yaml` / `.yml`). Extension-driven on
/// purpose: the file says what it is, not a setting.
fn is_yaml(path: &Path) -> bool {
    matches!(
        path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()),
        Some(ext) if ext == "yaml" || ext == "yml"
    )
}

/// Reads either syntax into the same structs.
///
/// YAML goes through `serde_json::Value`, so there is exactly one schema and one
/// error path: a YAML config can never mean something the JSON one cannot, and
/// every `#[serde(default)]` the structs already have applies to both.
pub fn parse_config(raw: &str, path: &Path) -> Result<AppConfig, String> {
    if is_yaml(path) {
        let value: serde_json::Value = serde_yaml_ng::from_str(raw)
            .map_err(|e| format!("{}: invalid YAML: {e}", path.display()))?;
        serde_json::from_value(value).map_err(|e| format!("{}: {e}", path.display()))
    } else {
        serde_json::from_str(raw).map_err(|e| format!("{}: {e}", path.display()))
    }
}

/// Renders the config in the syntax its own path has.
fn render_config(cfg: &AppConfig, path: &Path) -> Result<String, String> {
    let value = serde_json::to_value(cfg).map_err(|e| e.to_string())?;
    if is_yaml(path) {
        serde_yaml_ng::to_string(&value).map_err(|e| e.to_string())
    } else {
        serde_json::to_string_pretty(&value).map_err(|e| e.to_string())
    }
}

/// Reads a config from an explicit path — what `edgepass validate` and the tests
/// use, so a bad file can be checked without touching the user's own.
pub fn load_config_at(path: &Path) -> Result<AppConfig, String> {
    let raw = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut cfg = parse_config(&raw, path)?;
    migrate(&mut cfg);
    Ok(cfg)
}

pub fn db_path() -> Result<PathBuf, String> {
    let path = local_data_dir()?.join("edgepass.db");
    // Upgrade from <= 0.4.x, where the index lived in Roaming. Move it once so
    // the first launch after the update does not reindex from scratch.
    if !path.exists() {
        if let Ok(old) = data_dir().map(|dir| dir.join("edgepass.db")) {
            if old.exists() {
                if fs::rename(&old, &path).is_err() {
                    let _ = fs::copy(&old, &path);
                    let _ = fs::remove_file(&old);
                }
                crate::log_info!("index moved to %LOCALAPPDATA%\\edgePass");
            }
        }
    }
    Ok(path)
}

/// Config as the runtime should see it: file + one-time migration + the
/// `EDGEPASS_*` overrides of the current process.
pub fn load_config() -> Result<AppConfig, String> {
    let mut cfg = load_config_raw()?;
    advertise_env_overrides(&apply_env_overrides(&mut cfg));
    Ok(cfg)
}

/// The file on disk, verbatim (plus migration). Settings must round-trip
/// through *this* one: writing back an env-forced value would silently turn a
/// per-launch override into a permanent setting.
pub fn load_config_raw() -> Result<AppConfig, String> {
    let path = config_path()?;
    if !path.exists() {
        let cfg = AppConfig::default();
        save_config(&cfg)?;
        return Ok(cfg);
    }
    load_config_at(&path)
}

/// Per-process overrides, so an agent (or a second launcher) can point a run at
/// a different namespace set without rewriting the user's `config.json`. None
/// of these are ever persisted — see [`load_config_raw`].
///
/// | Var | Effect |
/// |-----|--------|
/// | `EDGEPASS_AUTO_UNSEAL` | `1`/`true`/`yes`/`on` · `0`/`false`/`no`/`off` |
/// | `EDGEPASS_AUTO_SEAL_IDLE` | seconds of idle before sealing (`0` = never) |
/// | `EDGEPASS_AUTO_COLLAPSE_IDLE` | seconds without pointer before collapsing (`0` = never) |
/// | `EDGEPASS_NAMESPACES` | comma-separated; `.` = root + all child namespaces |
fn apply_env_overrides(cfg: &mut AppConfig) -> Vec<String> {
    apply_env_overrides_with(cfg, |k| std::env::var(k).ok())
}

/// The parse half, with the environment injected so it can be tested without
/// mutating process-wide state.
fn apply_env_overrides_with<F>(cfg: &mut AppConfig, get: F) -> Vec<String>
where
    F: Fn(&str) -> Option<String>,
{
    let value = |key: &str| {
        get(key)
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    let mut applied = Vec::new();

    if let Some(raw) = value("EDGEPASS_AUTO_UNSEAL") {
        match parse_bool(&raw) {
            Some(on) => {
                cfg.auto_unseal = on;
                applied.push(format!("EDGEPASS_AUTO_UNSEAL={raw}"));
            }
            None => crate::log_warn!("EDGEPASS_AUTO_UNSEAL='{raw}' is not a boolean; ignoring it"),
        }
    }

    if let Some(raw) = value("EDGEPASS_AUTO_SEAL_IDLE") {
        match raw.parse::<u64>() {
            Ok(secs) => {
                cfg.auto_seal_idle_secs = secs;
                applied.push(format!("EDGEPASS_AUTO_SEAL_IDLE={secs}"));
            }
            Err(e) => {
                crate::log_warn!("EDGEPASS_AUTO_SEAL_IDLE='{raw}' is not a number ({e}); ignoring it")
            }
        }
    }

    if let Some(raw) = value("EDGEPASS_AUTO_COLLAPSE_IDLE") {
        match raw.parse::<u64>() {
            Ok(secs) => {
                cfg.auto_collapse_idle_secs = secs;
                applied.push(format!("EDGEPASS_AUTO_COLLAPSE_IDLE={secs}"));
            }
            Err(e) => crate::log_warn!(
                "EDGEPASS_AUTO_COLLAPSE_IDLE='{raw}' is not a number ({e}); ignoring it"
            ),
        }
    }

    if let Some(raw) = value("EDGEPASS_NAMESPACES") {
        let namespaces = parse_namespaces(&raw);
        if namespaces.is_empty() {
            crate::log_warn!("EDGEPASS_NAMESPACES='{raw}' leaves no namespace; ignoring it");
        } else {
            cfg.namespaces = namespaces;
            applied.push(format!("EDGEPASS_NAMESPACES={raw}"));
        }
    }

    if let Some(raw) = value("EDGEPASS_QRING_PACK") {
        cfg.qring_pack = raw;
        applied.push("EDGEPASS_QRING_PACK".into());
    }
    if let Some(raw) = value("EDGEPASS_QRING_HOST") {
        cfg.qring_host = raw;
        applied.push("EDGEPASS_QRING_HOST".into());
    }

    applied
}

/// Same syntax as the Settings field: `.`, `integrations` (`.` = root + every child ns).
fn parse_namespaces(raw: &str) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    raw.split(',')
        .map(|n| n.trim())
        .map(|n| if n == "." { "" } else { n })
        .filter(|n| seen.insert(n.to_string()))
        .map(|n| n.to_string())
        .collect()
}

fn parse_bool(raw: &str) -> Option<bool> {
    match raw.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

/// The UI polls `get_status` every 5s and every poll re-reads the config, so
/// without this the "override in effect" line would repeat forever.
fn advertise_env_overrides(applied: &[String]) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static ANNOUNCED: AtomicBool = AtomicBool::new(false);
    if applied.is_empty() || ANNOUNCED.swap(true, Ordering::Relaxed) {
        return;
    }
    crate::log_warn!(
        "environment overrides in effect (they are not saved to config.json): {}",
        applied.join(", ")
    );
}

/// One-time upgrade of configs written before the seal-ownership change.
/// v1 auto-sealed the *shared* OpenBao after 300s of widget idle — a
/// vault-wide side effect triggered by a search box going quiet. Only the
/// untouched legacy default is rewritten, so a deliberate value survives.
fn migrate(cfg: &mut AppConfig) {
    if !migrate_values(cfg) {
        return;
    }
    if let Err(e) = save_config(cfg) {
        crate::log_warn!("could not save the migrated config: {e}");
    }
}

/// The decision half of [`migrate`], with no file access — so it can be tested
/// without rewriting the machine's real `config.json`. Returns whether the
/// config changed.
fn migrate_values(cfg: &mut AppConfig) -> bool {
    if cfg.config_version >= CONFIG_VERSION {
        return false;
    }
    if cfg.auto_seal_idle_secs == LEGACY_IDLE_SECS {
        cfg.auto_seal_idle_secs = 0;
        crate::log_warn!(
            "config v1->v2: autoSealIdleSecs 300->0 (auto-sealing cuts the vault off from all its clients; enable it in Settings if you want it)"
        );
    }
    if cfg.config_version < 3 {
        // Nothing to rewrite: the serde default already restored the three LEDs a
        // pre-v3 header hardcoded. Say where they live now, once.
        crate::log_info!(
            "config v2->v3: header LEDs become configurable toggles[] (the usual ones are kept; edit them in Settings → Toggles)"
        );
    }
    if cfg.config_version < 4 {
        // `GW` *was* the Q-ring gateway's process. It is now the local GHA runner
        // (Docker Desktop + the container that deploys for us), so the old spec moves to
        // its own `QR` LED — the off→policy coupling travels with it — and `GW` comes
        // back with its new job, appended so nothing already on the header moves.
        //
        // Only the stock spec is touched: a header someone edited by hand (another probe,
        // another label) is left exactly as it is, because that is a choice, not a default.
        if let Some(pos) = cfg.toggles.iter().position(|t| *t == toggles::stock_gw()) {
            cfg.toggles[pos] = toggles::qr();
            if !cfg.toggles.iter().any(|t| t.id == "gw") && cfg.toggles.len() < toggles::MAX_TOGGLES
            {
                cfg.toggles.push(toggles::gw());
            }
            crate::log_info!(
                "config v3->v4: the GW LED now watches the local GHA runner (Docker Desktop + container); the Q-ring process moves to the QR LED"
            );
        }
    }
    cfg.config_version = CONFIG_VERSION;
    true
}

pub fn save_config(cfg: &AppConfig) -> Result<(), String> {
    let path = config_path()?;
    let raw = render_config(cfg, &path)?;
    fs::write(path, raw).map_err(|e| e.to_string())
}

/// Writes a config to an explicit path, in that path's own syntax.
pub fn save_config_at(cfg: &AppConfig, path: &Path) -> Result<(), String> {
    let raw = render_config(cfg, path)?;
    fs::write(path, raw).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn get_token() -> Result<Option<String>, String> {
    {
        let cache = TOKEN_CACHE.lock();
        if let Some((at, value)) = cache.as_ref() {
            if at.elapsed() < TOKEN_CACHE_TTL {
                return Ok(value.clone());
            }
        }
    }
    let value = read_keyring()?;
    *TOKEN_CACHE.lock() = Some((std::time::Instant::now(), value.clone()));
    Ok(value)
}

fn read_keyring() -> Result<Option<String>, String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).map_err(|e| e.to_string())?;
    match entry.get_password() {
        Ok(p) if !p.is_empty() => Ok(Some(p)),
        Ok(_) => Ok(None),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

pub fn set_token(token: &str) -> Result<(), String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).map_err(|e| e.to_string())?;
    if token.is_empty() {
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => {}
            Err(e) => return Err(e.to_string()),
        }
        crate::log_info!("token deleted from the keyring");
    } else {
        entry.set_password(token).map_err(|e| e.to_string())?;
        crate::log_info!("token saved to the keyring");
    }
    *TOKEN_CACHE.lock() = None;
    Ok(())
}

pub fn has_token() -> bool {
    matches!(get_token(), Ok(Some(_)))
}

/// Load root token from local init file into keyring. Never returns the token.
pub fn bootstrap_token_from_init(init_path: &str) -> Result<bool, String> {
    if has_token() {
        return Ok(false);
    }
    if init_path.trim().is_empty() {
        return Err(
            "the vault init file is not configured: set Init JSON or paste a token by hand in Settings → Vault"
                .into(),
        );
    }
    let path = PathBuf::from(init_path);
    if !path.exists() {
        return Err(format!("init file not found: {init_path}"));
    }
    let raw = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let raw = raw.trim_start_matches('\u{feff}');
    let v: serde_json::Value = serde_json::from_str(raw).map_err(|e| e.to_string())?;
    let token = v
        .get("root_token")
        .and_then(|t| t.as_str())
        .ok_or_else(|| "root_token missing in init file".to_string())?;
    set_token(token)?;
    crate::log_warn!(
        "token loaded from {init_path}: this is the ROOT token (full scope). A child token with a narrowed policy is better"
    );
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Injected environment: no process-wide state, so tests can run in
    /// parallel with each other and with a real widget on the same machine.
    fn env_from<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |key: &str| {
            pairs
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| (*v).to_string())
        }
    }

    #[test]
    fn legacy_auto_seal_default_is_migrated_once() {
        let mut cfg = AppConfig {
            auto_seal_idle_secs: LEGACY_IDLE_SECS,
            config_version: 0,
            ..Default::default()
        };
        assert!(migrate_values(&mut cfg));
        assert_eq!(
            cfg.auto_seal_idle_secs, 0,
            "the v1 default must not survive"
        );
        assert_eq!(cfg.config_version, CONFIG_VERSION);
        assert!(!migrate_values(&mut cfg), "second pass must be a no-op");
    }

    #[test]
    fn deliberate_auto_seal_survives_migration() {        let mut cfg = AppConfig {
            auto_seal_idle_secs: 900,
            config_version: 0,
            ..Default::default()
        };
        assert!(migrate_values(&mut cfg));
        assert_eq!(cfg.auto_seal_idle_secs, 900);
    }

    /// v3→v4: the Q-ring `GW` moves to `QR` and `GW` gets its new job (the local
    /// runner), without losing control of the gateway or moving what is already placed.
    #[test]
    fn gw_moves_to_qr_and_gets_its_new_job() {
        let mut cfg = AppConfig {
            config_version: 3,
            toggles: toggles::legacy_toggles(),
            ..Default::default()
        };
        assert!(migrate_values(&mut cfg));

        let ids: Vec<&str> = cfg.toggles.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, vec!["kv", "qr", "gov", "gw"]);
        let qr = cfg.toggles.iter().find(|t| t.id == "qr").unwrap();
        assert_eq!(qr.probe, "qring.process");
        assert_eq!(qr.alt.as_deref(), Some("qring.restart"));
        let gw = cfg.toggles.iter().find(|t| t.id == "gw").unwrap();
        assert_eq!(gw.probe, "gha.runner");
        assert_eq!(gw.on, "gha.up");

        assert!(!migrate_values(&mut cfg), "the second pass is a no-op");
        let ids: Vec<&str> = cfg.toggles.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, vec!["kv", "qr", "gov", "gw"], "the GW is not duplicated");
    }

    /// A hand-edited header is a decision, not a default: the migration leaves it alone.
    #[test]
    fn a_hand_edited_gw_is_left_alone() {
        let mut toggles = toggles::legacy_toggles();
        toggles[1].label = "GATE".into();
        let mut cfg = AppConfig {
            config_version: 3,
            toggles,
            ..Default::default()
        };
        assert!(migrate_values(&mut cfg));
        let ids: Vec<&str> = cfg.toggles.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, vec!["kv", "gw", "gov"], "kept as it is");
        assert_eq!(cfg.toggles[1].label, "GATE");
    }

    /// The whole point of `legacy_toggles`: a `config.json` written before the
    /// header was data must reopen with exactly the LEDs it used to hardcode.
    #[test]
    fn a_file_without_toggles_keeps_the_three_hardcoded_leds() {        let raw = r#"{
            "baoAddr": "http://127.0.0.1:8200",
            "mount": "kv",
            "namespaces": [""],
            "configVersion": 2
        }"#;
        let cfg: AppConfig = serde_json::from_str(raw).unwrap();
        let ids: Vec<&str> = cfg.toggles.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, vec!["kv", "gw", "gov"]);
        assert_eq!(cfg.toggles[1].alt.as_deref(), Some("qring.restart"));
    }

    /// …while an install that never had a file gets the one light every install
    /// can justify: the vault, because OpenBao is a prerequisite.
    #[test]
    fn a_fresh_config_ships_the_vault_led() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.toggles.len(), 1);
        assert_eq!(cfg.toggles[0].id, "kv");
    }

    #[test]
    fn toggles_round_trip_through_json() {
        let cfg = AppConfig {
            toggles: toggles::legacy_toggles(),
            ..Default::default()
        };
        let raw = serde_json::to_string(&cfg).unwrap();
        let back: AppConfig = serde_json::from_str(&raw).unwrap();
        assert_eq!(back.toggles, cfg.toggles);
        assert_eq!(back.config_version, CONFIG_VERSION);
    }

    #[test]
    fn env_overrides_win_over_the_file() {
        let mut cfg = AppConfig::default();
        let applied = apply_env_overrides_with(
            &mut cfg,
            env_from(&[
                ("EDGEPASS_AUTO_UNSEAL", " off "),
                ("EDGEPASS_AUTO_SEAL_IDLE", "120"),
                ("EDGEPASS_NAMESPACES", "., integrations , invoices"),
            ]),
        );
        assert!(!cfg.auto_unseal);
        assert_eq!(cfg.auto_seal_idle_secs, 120);
        assert_eq!(cfg.namespaces, vec!["", "integrations", "invoices"]);
        assert_eq!(applied.len(), 3);
    }

    #[test]
    fn junk_overrides_are_ignored_not_persisted() {
        let defaults = AppConfig::default();
        let mut cfg = AppConfig::default();
        let applied = apply_env_overrides_with(
            &mut cfg,
            env_from(&[
                ("EDGEPASS_AUTO_UNSEAL", "maybe"),
                ("EDGEPASS_AUTO_SEAL_IDLE", "five"),
                ("EDGEPASS_NAMESPACES", "   "),
            ]),
        );
        assert!(applied.is_empty());
        assert_eq!(cfg.auto_unseal, defaults.auto_unseal);
        assert_eq!(cfg.auto_seal_idle_secs, defaults.auto_seal_idle_secs);
        assert_eq!(cfg.namespaces, defaults.namespaces);
    }

    #[test]
    fn namespaces_parse_like_the_settings_field() {
        assert_eq!(parse_namespaces("."), vec![""]);
        assert_eq!(
            parse_namespaces(" integrations , invoices "),
            vec!["integrations", "invoices"]
        );
        assert_eq!(parse_namespaces("a,a,."), vec!["a", ""]);
    }

    #[test]
    fn bool_parsing_covers_the_documented_spellings() {
        for yes in ["1", "true", "TRUE", "yes", "on"] {
            assert_eq!(parse_bool(yes), Some(true), "{yes}");
        }
        for no in ["0", "false", "no", "off"] {
            assert_eq!(parse_bool(no), Some(false), "{no}");
        }
        assert_eq!(parse_bool("maybe"), None);
    }

    /// YAML is the documented format and JSON is what every install before it
    /// has: both have to produce the same struct, byte for byte in meaning.
    #[test]
    fn yaml_and_json_are_the_same_config() {
        let yaml = r#"
baoAddr: http://127.0.0.1:8200
mount: kv
namespaces: [""]
autoUnseal: false
autoSealIdleSecs: 900
qringPack: /opt/qring
cloudflaredExe: /usr/local/bin/cloudflared
toggles:
  - id: kv
    label: KV
    probe: vault.seal
    on: vault.unseal
    off: vault.seal
mcpServers:
  - id: demo
    label: demo
    command: /opt/demo/server
    tunnel: demo-tunnel
    autostart: true
configVersion: 4
"#;
        let json = r#"{
            "baoAddr": "http://127.0.0.1:8200",
            "mount": "kv",
            "namespaces": [""],
            "autoUnseal": false,
            "autoSealIdleSecs": 900,
            "qringPack": "/opt/qring",
            "cloudflaredExe": "/usr/local/bin/cloudflared",
            "toggles": [
                {"id": "kv", "label": "KV", "probe": "vault.seal", "on": "vault.unseal", "off": "vault.seal"}
            ],
            "mcpServers": [
                {"id": "demo", "label": "demo", "command": "/opt/demo/server", "tunnel": "demo-tunnel", "autostart": true}
            ],
            "configVersion": 4
        }"#;
        let from_yaml =
            parse_config(yaml, Path::new("config.yaml")).expect("yaml should parse");
        let from_json =
            parse_config(json, Path::new("config.json")).expect("json should parse");
        // Compared as a value: `AppConfig` does not derive `PartialEq`, and it
        // does not need to just for a test.
        assert_eq!(
            serde_json::to_value(&from_yaml).unwrap(),
            serde_json::to_value(&from_json).unwrap()
        );
        assert_eq!(from_yaml.mcp_servers[0].tunnel, "demo-tunnel");
        assert_eq!(from_yaml.cloudflared_exe, "/usr/local/bin/cloudflared");
        assert!(!from_yaml.auto_unseal, "a false in yaml is not a default");
    }

    /// The file says what it is: `render_config` writes the syntax of its own
    /// extension, so `edgepass init` writes YAML and an existing `config.json`
    /// never comes back as YAML behind the user's back.
    #[test]
    fn the_saved_syntax_matches_the_extension() {
        let cfg = AppConfig {
            mount: "kv".into(),
            ..Default::default()
        };
        let as_yaml = render_config(&cfg, Path::new("config.yaml")).unwrap();
        assert!(as_yaml.contains("mount: kv"), "got:\n{as_yaml}");
        let as_json = render_config(&cfg, Path::new("config.json")).unwrap();
        assert!(as_json.trim_start().starts_with('{'), "got:\n{as_json}");
        // …and both read back the same.
        let a = parse_config(&as_yaml, Path::new("config.yaml")).unwrap();
        let b = parse_config(&as_json, Path::new("config.json")).unwrap();
        assert_eq!(
            serde_json::to_value(&a).unwrap(),
            serde_json::to_value(&b).unwrap()
        );
    }

    /// A typo in the YAML must name the file, not come back as "invalid type".
    #[test]
    fn a_broken_yaml_names_the_file() {
        let err = parse_config("mount: [kv", Path::new("/tmp/config.yaml")).unwrap_err();
        assert!(err.contains("config.yaml"), "got: {err}");
    }
}
