// Logging macros live at the crate root, *before* the module declarations, so
// they are unambiguously in scope for every module below.
#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => { $crate::logging::line("INFO", &format!($($arg)*)) };
}

#[macro_export]
macro_rules! log_warn {
    ($($arg:tt)*) => { $crate::logging::line("WARN", &format!($($arg)*)) };
}

#[macro_export]
macro_rules! log_error {
    ($($arg:tt)*) => { $crate::logging::line("ERROR", &format!($($arg)*)) };
}

mod apps;
mod autostart;
mod cli;
mod config;
pub use cli::cli_main;
mod gha;
mod grant;
mod ipc;
mod locale;
mod logging;
mod mcp;
mod net;
mod notices;
mod openbao;
mod passphrase;
#[cfg(windows)]
mod pipe_acl;
mod placement;
mod pointer;
mod qring;
mod rand;
mod services;
mod skills;
mod store;
mod toggles;
mod unseal;
mod winservice;

use config::{
    bootstrap_token_from_init, has_token, load_config, load_config_raw, save_config, set_token,
    AppConfig,
};
use openbao::{
    delete_secret_key, read_secret_map, read_secret_value, scan_index, upsert_secret_field,
    upsert_secret_map,
};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use store::{SearchHit, Store};
use tauri::{Manager, State};
use unseal::{ensure_unsealed, seal, seal_status, unseal, SealStatusDto};
use winservice::ServiceStatusDto;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestFormDto {
    pub id: u64,
    pub namespace: String,
    pub path: String,
    pub keys: Vec<String>,
    pub title: Option<String>,
    /// `None` = the human types the values (`ingest-form`); `"vault"` = the
    /// widget reads this path itself and fills the same panel with its real
    /// fields (`meter`). Either way the pipe never carries a value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchField {
    pub key: String,
    pub value: String,
}

pub struct AppState {
    pub store: Mutex<Store>,
    /// Unix secs of last vault activity (copy/sync/write/unseal-for-use).
    pub last_activity: AtomicU64,
    /// Unix secs until which the guardian must not auto-seal (0 = no hold).
    pub hold_until: AtomicU64,
    pub pending_ingest: Mutex<Option<IngestFormDto>>,
    /// Detached `run` children (in-memory only; no autoresume after widget kill).
    pub runs: crate::ipc::DetachedRuns,
    /// GW approval queue (in-memory; Allow once / 2h). No secret values.
    pub grants: crate::grant::GrantBoard,
    /// Last job already announced per runner id, so the log viewer opens **once** per
    /// job instead of once per watcher tick (in-memory by design).
    pub gha_announced: Mutex<std::collections::HashMap<String, String>>,
}

pub(crate) fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub(crate) fn touch(state: &AppState) {
    state.last_activity.store(now_secs(), Ordering::Relaxed);
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusDto {
    pub bao_addr: String,
    pub mount: String,
    pub namespaces: Vec<String>,
    pub has_token: bool,
    pub indexed: i64,
    pub last_sync: Option<String>,
    pub auto_unseal: bool,
    pub auto_seal_idle_secs: u64,
    pub auto_collapse_idle_secs: u64,
    pub auto_sync: bool,
    pub auto_load_token: bool,
    pub init_path: String,
    pub seal: SealStatusDto,
    /// Why the vault is (or is not) usable, with the next steps. Both the window
    /// and an agent read this; `code` is the stable part.
    pub setup: SetupHint,
    pub idle_secs: u64,
    pub service: ServiceStatusDto,
    /// The header LEDs, as data: the stored spec plus what its probe sees right
    /// now (see the `toggles` module — the vocabulary ships in `catalog`).
    pub toggles: Vec<toggles::ToggleView>,
    pub catalog: toggles::Catalog,
    /// The MCP list with live process state (see the `mcp` module).
    pub mcp: mcp::McpStatusDto,
    /// Owner-defined services (`services[]`), as stored. The `svc.<id>` half of
    /// `catalog` is derived from this list, so the editor can render both from
    /// one snapshot.
    pub services: Vec<services::ServiceSpec>,
}

/// What a first-run install is missing, in a shape both a person and an agent
/// can act on.
///
/// `code` is stable (an agent branches on it), `message`/`steps` are English and
/// actionable, and the links are the official ones. The window localizes its own
/// copy from `code`; it does not print `steps`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupHint {
    pub needed: bool,
    /// `vault_unreachable` | `vault_uninitialized` | `init_file_missing` |
    /// `no_token` | `vault_sealed` | `ready`
    pub code: String,
    pub message: String,
    pub steps: Vec<String>,
    pub install_url: String,
    pub releases_url: String,
    pub readme_url: String,
}

const OPENBAO_INSTALL_URL: &str = "https://openbao.org/docs/install/";
const OPENBAO_RELEASES_URL: &str = "https://github.com/openbao/openbao/releases";
const SETUP_README_URL: &str = "https://github.com/Xhuk/EdgePass#readme";

/// Why the vault is not ready, and the next four steps. Pure and cheap: called
/// from `get_status` and from the pipe's `status`, so an agent and the window
/// see the same answer.
pub(crate) fn setup_hint(cfg: &AppConfig, seal: &SealStatusDto, has_token: bool) -> SetupHint {
    let addr = {
        let raw = cfg.bao_addr.trim();
        if raw.is_empty() {
            "the configured address"
        } else {
            raw
        }
    };
    let init_path = cfg.init_path.trim();
    let build = |code: &str, message: String, steps: Vec<String>| SetupHint {
        needed: true,
        code: code.into(),
        message,
        steps,
        install_url: OPENBAO_INSTALL_URL.into(),
        releases_url: OPENBAO_RELEASES_URL.into(),
        readme_url: SETUP_README_URL.into(),
    };
    let steps = |items: &[&str]| items.iter().map(|s| s.to_string()).collect::<Vec<_>>();

    if !seal.reachable {
        return build(
            "vault_unreachable",
            format!("OpenBao is not reachable at {addr}."),
            steps(&[
                "Install OpenBao 2.x from the install guide or the releases page, and run it as a local service.",
                "Listen on loopback; 127.0.0.1:8200 is the address edgePass expects by default.",
                "Initialize it once (bao operator init) and keep the init file outside every repository.",
                "In edgePass: Settings > Vault, confirm the address, then paste a token or point initPath at the init file.",
            ]),
        );
    }
    if !seal.initialized {
        return build(
            "vault_uninitialized",
            "OpenBao answers but has not been initialized.".into(),
            steps(&[
                "Run bao operator init once and store the init file outside every repository.",
                "Put that path in Settings > Vault (initPath), or paste a token directly.",
                "Unseal the vault if it is still sealed.",
            ]),
        );
    }
    if !init_path.is_empty() && !std::path::Path::new(init_path).exists() {
        return build(
            "init_file_missing",
            format!("The configured init file does not exist: {init_path}"),
            steps(&[
                "Move the init file to that path, or update initPath in Settings > Vault.",
                "The init file holds the unseal keys and the root token; keep it out of every repository.",
            ]),
        );
    }
    if !has_token && init_path.is_empty() {
        return build(
            "no_token",
            "The vault answers but edgePass has no token.".into(),
            steps(&[
                "In Settings > Vault, paste a token with read/list on the KV mount, or set initPath so edgePass can load it.",
                "Save. The index syncs automatically after the token is stored.",
            ]),
        );
    }
    if seal.sealed && (init_path.is_empty() || !cfg.auto_unseal) {
        return build(
            "vault_sealed",
            "The vault is sealed and edgePass cannot open it.".into(),
            steps(&[
                "Unseal it with bao operator unseal, or enable auto-unseal with an init file in Settings > Vault.",
                "For a sealed vault, the unseal keys come from the init file; the token alone is not enough.",
            ]),
        );
    }
    SetupHint {
        needed: false,
        code: "ready".into(),
        message: "OpenBao is reachable and ready.".into(),
        steps: Vec::new(),
        install_url: OPENBAO_INSTALL_URL.into(),
        releases_url: OPENBAO_RELEASES_URL.into(),
        readme_url: SETUP_README_URL.into(),
    }
}

/// Opens one of the fixed setup links in the browser.
///
/// The caller names an id, not a URL: the destination is a constant here, so the
/// pipe cannot turn this into "open anything".
#[tauri::command]
fn open_setup_link(id: String) -> Result<(), String> {
    let url = match id.as_str() {
        "openbao_install" => OPENBAO_INSTALL_URL,
        "openbao_releases" => OPENBAO_RELEASES_URL,
        "readme" => SETUP_README_URL,
        other => return Err(format!("link desconocido: {other}")),
    };
    open_external(url)
}

fn open_external(url: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|e| format!("no pude abrir el navegador: {e}"))?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(url)
            .spawn()
            .map_err(|e| format!("no pude abrir el navegador: {e}"))?;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open")
            .arg(url)
            .spawn()
            .map_err(|e| format!("no pude abrir el navegador: {e}"))?;
    }
    Ok(())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BootstrapResult {
    pub seal: SealStatusDto,
    pub token_loaded: bool,
    pub unsealed_now: bool,
    pub synced: Option<usize>,
    pub message: String,
}

/// Validates the `services[]` list without storing it.
///
/// The Ajustes editor calls this while the JSON is being typed, so the error
/// (missing command, empty probe, over the cap) appears before Guardar and with
/// the same rules the save would apply. It returns the normalized list, which is
/// also what the save will send.
#[tauri::command]
fn validate_services(
    list: Vec<services::ServiceSpec>,
) -> Result<Vec<services::ServiceSpec>, String> {
    services::validate(list)
}

/// Open vault if sealed (when auto_unseal), ensure token exists.
pub(crate) async fn ensure_ready(state: &AppState) -> Result<(AppConfig, String), String> {
    let cfg = load_config()?;

    // Refuse a non-loopback vault before touching the keyring or the network.
    crate::net::assert_local(&cfg.bao_addr)?;

    if !has_token() && cfg.auto_load_token {
        // Not fatal: if the keyring is empty and the init file is gone, the
        // token lookup below produces the message the user can act on.
        match bootstrap_token_from_init(&cfg.init_path) {
            Ok(true) => crate::log_info!("token cargado del init al keyring"),
            Ok(false) => {}
            Err(e) => crate::log_warn!("no pude cargar el token del init: {e}"),
        }
    }

    let token = config::get_token()?
        .ok_or_else(|| "Sin token (activa auto-load o pégalo en ajustes)".to_string())?;

    let status = seal_status(&cfg).await;
    if !status.reachable {
        crate::log_warn!(
            "vault no responde ({}); intento arrancar el servicio OpenBao",
            status.message
        );
        let _ = winservice::ensure_running();
        tokio::time::sleep(std::time::Duration::from_millis(800)).await;
        let again = seal_status(&cfg).await;
        if !again.reachable {
            return Err(again.message);
        }
    }

    let status = seal_status(&cfg).await;
    if status.sealed {
        if !cfg.auto_unseal {
            return Err("OpenBao sellado (auto-unseal desactivado)".into());
        }
        crate::log_info!("vault sellado: abriendo con las keys del init");
        let after = ensure_unsealed(&cfg).await?;
        if after.sealed {
            return Err(format!("No se pudo abrir: {}", after.message));
        }
    }

    touch(state);
    Ok((cfg, token))
}

fn status_snapshot(state: &AppState, cfg: &AppConfig, seal: SealStatusDto) -> StatusDto {
    let (indexed, last_sync) = {
        let store = state.store.lock();
        (store.count().unwrap_or(0), store.last_sync().ok().flatten())
    };
    let last = state.last_activity.load(Ordering::Relaxed);
    let idle = if last == 0 {
        0
    } else {
        now_secs().saturating_sub(last)
    };
    // `views` probes the Q-ring only when a LED asks for it (the call spawns
    // `tasklist`, and this snapshot runs on a 5 s poll).
    let leds = toggles::views(cfg, &seal, !state.grants.list_public().is_empty());
    let has_token = has_token();
    let setup = setup_hint(cfg, &seal, has_token);

    StatusDto {
        bao_addr: cfg.bao_addr.clone(),
        mount: cfg.mount.clone(),
        namespaces: cfg.namespaces.clone(),
        has_token,
        setup,
        indexed,
        last_sync,
        auto_unseal: cfg.auto_unseal,
        auto_seal_idle_secs: cfg.auto_seal_idle_secs,
        auto_collapse_idle_secs: cfg.auto_collapse_idle_secs,
        auto_sync: cfg.auto_sync,
        auto_load_token: cfg.auto_load_token,
        init_path: cfg.init_path.clone(),
        seal,
        idle_secs: idle,
        // El poll de la UI (5s) usa la caché: no lanzamos `sc.exe` por tick.
        // El estado real (y fresco) se consulta al arrancar el servicio y en el bootstrap.
        service: winservice::status_cached(),
        toggles: leds,
        catalog: toggles::catalog(cfg),
        // Cached inside `mcp` (10 s TTL): this snapshot runs on a 5 s poll and a
        // process sweep is a `tasklist` per unresolved row.
        mcp: mcp::status(cfg),
        services: cfg.services.clone(),
    }
}

#[tauri::command]
async fn get_status(state: State<'_, Arc<AppState>>) -> Result<StatusDto, String> {
    let cfg = load_config()?;
    let seal = seal_status(&cfg).await;
    // The snapshot shells out (MCP sweep, `svc` probes) and would block a tokio
    // worker; with a couple of cold `cmd` services that is the difference
    // between one 5 s poll and a pile-up of them.
    let state = state.inner().clone();
    tokio::task::spawn_blocking(move || status_snapshot(&state, &cfg, seal))
        .await
        .map_err(|e| format!("status task: {e}"))
}

#[tauri::command]
async fn refresh_seal() -> Result<SealStatusDto, String> {
    let cfg = load_config()?;
    Ok(seal_status(&cfg).await)
}

/// Un comando de diagnóstico escrito desde la webview, al mismo log que el resto
/// (`%APPDATA%\edgePass\logs\edgepass.log`).
///
/// Existe porque el pipeline de geometría —mediciones, decisions de tamaño y los
/// frames del spring— no se puede diagnosticar con una captura: pasa en el
/// tiempo. La webview manda tandas cada 250 ms y acá se escriben tal cual, así
/// `Get-Content -Tail` cuenta la historia completa.
#[tauri::command]
fn ui_log(msg: String) {
    for line in msg.lines().filter(|l| !l.trim().is_empty()) {
        crate::log_info!("ui: {line}");
    }
}

/// The MCP list with live process state — the panel's refresh button.
#[tauri::command]
async fn mcp_status() -> Result<mcp::McpStatusDto, String> {
    let cfg = load_config()?;
    Ok(mcp::status_now(&cfg))
}

/// Start, stop or restart one MCP server. `dir` = `on` | `off` | `restart`.
///
/// Opening the vault happens only when a row declares `secretEnv`, so a list of
/// plain commands never unseals anything.
#[tauri::command]
async fn mcp_control(
    state: State<'_, Arc<AppState>>,
    id: String,
    dir: String,
) -> Result<mcp::McpStatusDto, String> {
    let mut cfg = load_config()?;
    let mut secrets = mcp::Secrets::new();
    if dir != "off" && mcp::any_needs_vault(&cfg) {
        let (fresh, token) = ensure_ready(state.inner()).await?;
        cfg = fresh;
        secrets = mcp::resolve_all(&cfg, &token).await;
    }
    let (id2, dir2) = (id, dir);
    let (cfg2, secrets2) = (cfg.clone(), secrets);
    tauri::async_runtime::spawn_blocking(move || match dir2.as_str() {
        "on" => mcp::start(&cfg2, &id2, &secrets2),
        "off" => mcp::stop(&cfg2, &id2),
        _ => mcp::restart(&cfg2, &id2, &secrets2),
    })
    .await
    .map_err(|e| format!("mcp task: {e}"))??;
    Ok(mcp::status_now(&cfg))
}

/// Mints a fresh token, writes it where the row's `secretEnv` entry for `key`
/// points, and returns it so the panel can show it for copying.
///
/// This is the «generar token» button. The value is created here, stored in
/// OpenBao, and injected into the server's environment at spawn time — it is
/// never written to `config.json`, never logged, and never carried by the pipe.
#[tauri::command]
async fn mcp_mint_token(
    state: State<'_, Arc<AppState>>,
    id: String,
    key: String,
) -> Result<String, String> {
    let cfg = load_config()?;
    let (namespace, path, field, label) = {
        let s = cfg
            .mcp_servers
            .iter()
            .find(|s| s.id == id.trim())
            .ok_or_else(|| format!("servidor MCP desconocido: '{id}'"))?;
        let want = s.secret_env.iter().find(|e| e.key == key.trim()).ok_or_else(|| {
            format!(
                "'{}' no declara secretEnv para '{}': agregalo en el panel MCP",
                s.label, key
            )
        })?;
        let namespace = if want.namespace.trim().is_empty() {
            cfg.namespaces.first().cloned().unwrap_or_default()
        } else {
            want.namespace.clone()
        };
        (namespace, want.path.clone(), want.field.clone(), s.label.clone())
    };
    let value = mcp::generate_token(32)?;
    let (cfg, token) = ensure_ready(state.inner()).await?;
    crate::openbao::upsert_secret_field(&cfg, &token, &namespace, &path, &field, &value, false)
        .await
        .map_err(|e| format!("{label}: {namespace}/{path}: {e}"))?;
    crate::log_info!("mcp: token nuevo para {label} en {namespace}/{path} campo {field}");
    Ok(value)
}

/// Builds a Diceware passphrase inside the widget.
///
/// This is the «sugerir» button of the Meter tab. The value is created here and
/// shown in the row the person asked to fill. It is written nowhere else: it
/// does not travel over the local pipe, it is not logged, and it does not reach
/// the clipboard until the person copies it.
#[tauri::command]
fn make_passphrase(options: passphrase::Options) -> Result<passphrase::Made, String> {
    let mut cfg = load_config()?;
    let salt = ensure_install_salt(&mut cfg);
    let made = passphrase::make_with(&options, &salt)?;
    // Only the shape is recorded. The value stays out of the log.
    crate::log_info!(
        "passphrase: {} palabras, {:.1} bits, {} caracteres",
        made.words,
        made.entropy_bits,
        made.length
    );
    Ok(made)
}

/// The per-installation salt, generated once and stored in `config.json`.
///
/// It is **not** a secret and adds no entropy: it rotates the vocabulary
/// mapping, so two machines cloned from the same snapshot still draw different
/// words. A failure to generate or decode it degrades to "no salt" instead of
/// blocking the passphrase.
fn ensure_install_salt(cfg: &mut AppConfig) -> Vec<u8> {
    if cfg.install_salt.trim().is_empty() {
        let mut bytes = [0u8; 16];
        if crate::rand::fill(&mut bytes).is_ok() {
            cfg.install_salt = bytes.iter().map(|b| format!("{b:02x}")).collect();
            if let Err(e) = save_config(cfg) {
                crate::log_warn!("no pude guardar el salt de instalación: {e}");
            }
        }
    }
    let salt = decode_hex(&cfg.install_salt);
    if salt.is_empty() && !cfg.install_salt.trim().is_empty() {
        crate::log_warn!("installSalt inválido: se ignora");
    }
    salt
}

/// Hex → bytes, strict. `Vec::new()` for anything that is not an even-length
/// run of hex digits, so a hand-edited config cannot inject a weird salt.
fn decode_hex(raw: &str) -> Vec<u8> {
    let raw = raw.trim();
    if raw.is_empty() || raw.len() % 2 != 0 {
        return Vec::new();
    }
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() / 2);
    for pair in bytes.chunks_exact(2) {
        let hi = (pair[0] as char).to_digit(16);
        let lo = (pair[1] as char).to_digit(16);
        match (hi, lo) {
            (Some(h), Some(l)) => out.push((h * 16 + l) as u8),
            _ => return Vec::new(),
        }
    }
    out
}

/// The stored passphrase style, so the window can render the settings it
/// actually has instead of guessing.
#[tauri::command]
fn passphrase_config() -> Result<passphrase::PassphraseConfig, String> {
    Ok(load_config()?.passphrase)
}

/// Saves the style and marks it configured.
///
/// `configured` is set here, not taken from the caller: the only way to reach
/// this command is a person choosing the style, which is exactly the event the
/// flag records. That keeps the first-run question from ever being asked twice.
#[tauri::command]
fn save_passphrase_config(
    settings: passphrase::PassphraseConfig,
) -> Result<passphrase::PassphraseConfig, String> {
    let mut cfg = load_config()?;
    cfg.passphrase = passphrase::PassphraseConfig {
        configured: true,
        ..settings
    };
    save_config(&cfg)?;
    crate::log_info!(
        "passphrase: estilo guardado ({} palabras, separador {:?}, compatible={})",
        cfg.passphrase.words,
        cfg.passphrase.separator,
        cfg.passphrase.compatible
    );
    Ok(cfg.passphrase)
}

/// Runs the row's `credential_command` and returns what it printed, for copying
/// into the client. Only the server can mint a valid client id, so the widget
/// runs the command the row declares instead of inventing one.
#[tauri::command]
async fn mcp_credential_command(id: String) -> Result<String, String> {
    let cfg = load_config()?;
    tauri::async_runtime::spawn_blocking(move || mcp::run_credential_command(&cfg, &id))
        .await
        .map_err(|e| format!("mcp task: {e}"))?
}

/// Open the vault and count it as activity. Shared by the `unseal_vault` command
/// and the `vault.unseal` toggle action, so a header click and the pipe's
/// `{"op":"unseal"}` cannot drift.
pub(crate) async fn unseal_now(cfg: &AppConfig, state: &AppState) -> Result<SealStatusDto, String> {
    let seal = unseal(cfg).await?;
    touch(state);
    Ok(seal)
}

/// Seal the vault. Shared by the `seal_vault` command and the `vault.seal` toggle
/// action. Deliberately does **not** touch activity: we closed on purpose.
pub(crate) async fn seal_vault_now(cfg: &AppConfig) -> Result<SealStatusDto, String> {
    if !has_token() && cfg.auto_load_token {
        let _ = bootstrap_token_from_init(&cfg.init_path)?;
    }
    let token = config::get_token()?.ok_or_else(|| "Sin token para sellar".to_string())?;
    seal(cfg, &token).await
}

#[tauri::command]
async fn unseal_vault(state: State<'_, Arc<AppState>>) -> Result<SealStatusDto, String> {
    let cfg = load_config()?;
    unseal_now(&cfg, state.inner()).await
}

#[tauri::command]
async fn seal_vault() -> Result<SealStatusDto, String> {
    let cfg = load_config()?;
    seal_vault_now(&cfg).await
}

/// Fire one header LED — what a click does. `dir` = `flip` (default) | `on` |
/// `off` | `alt`. The caller names the LED and the direction; which action that
/// means is read from the config (see `toggles::fire`).
#[tauri::command]
async fn fire_toggle(
    state: State<'_, Arc<AppState>>,
    id: String,
    dir: Option<String>,
) -> Result<toggles::ActionOutcome, String> {
    let cfg = load_config()?;
    toggles::fire(&cfg, id.trim(), dir.as_deref().unwrap_or("flip"), state.inner()).await
}

#[tauri::command]
fn load_token_from_init() -> Result<bool, String> {
    let cfg = load_config()?;
    bootstrap_token_from_init(&cfg.init_path)
}

#[tauri::command]
async fn bootstrap(state: State<'_, Arc<AppState>>) -> Result<BootstrapResult, String> {
    let cfg = load_config()?;
    let mut token_loaded = false;
    let mut unsealed_now = false;
    let mut synced = None;
    let mut parts = Vec::new();

    // Check service status (do NOT auto-start — let the user decide)
    let svc = winservice::status();
    if svc.installed && !svc.running {
        parts.push(format!("svc: {} (detenido)", svc.state));
    } else if svc.installed && svc.running {
        parts.push("svc OK".into());
    } else {
        parts.push("svc: no instalado (modo API)".into());
    }

    let mut seal = seal_status(&cfg).await;
    if !seal.reachable {
        // No auto-start here — just report it
        return Ok(BootstrapResult {
            seal,
            token_loaded: false,
            unsealed_now: false,
            synced: None,
            message: "OpenBao API no responde — revisa servicio OpenBao".into(),
        });
    }

    if !has_token() && cfg.auto_load_token {
        match bootstrap_token_from_init(&cfg.init_path) {
            Ok(true) => {
                token_loaded = true;
                parts.push("token OK".into());
            }
            Ok(false) => parts.push("token ya había".into()),
            Err(e) => parts.push(format!("token: {e}")),
        }
    }

    if seal.sealed && cfg.auto_unseal {
        match ensure_unsealed(&cfg).await {
            Ok(s) => {
                unsealed_now = !s.sealed;
                seal = s;
                if unsealed_now {
                    parts.push("abierto".into());
                    touch(state.inner());
                }
            }
            Err(e) => parts.push(format!("abrir: {e}")),
        }
    }

    if !seal.sealed && has_token() && cfg.auto_sync {
        match sync_index_inner(state.inner()).await {
            Ok(n) => {
                synced = Some(n);
                parts.push(format!("{n} claves"));
            }
            Err(e) => parts.push(format!("sync: {e}")),
        }
    }

    let message = if parts.is_empty() {
        seal.message.clone()
    } else {
        parts.join(" · ")
    };

    Ok(BootstrapResult {
        seal,
        token_loaded,
        unsealed_now,
        synced,
        message,
    })
}

#[tauri::command]
fn openbao_service_status() -> ServiceStatusDto {
    winservice::status()
}

#[tauri::command]
fn openbao_service_start() -> Result<ServiceStatusDto, String> {
    winservice::start()
}

#[tauri::command]
fn autostart_status() -> autostart::AutostartStatusDto {
    autostart::status()
}

#[tauri::command]
fn autostart_install() -> Result<autostart::AutostartStatusDto, String> {
    autostart::install()
}

#[tauri::command]
fn autostart_remove() -> Result<autostart::AutostartStatusDto, String> {
    autostart::remove()
}

#[tauri::command]
fn qring_status() -> qring::QringStatusDto {
    let cfg = load_config().unwrap_or_default();
    qring::status(&cfg.qring_pack, &cfg.qring_host)
}

#[tauri::command]
fn qring_govern(enabled: bool) -> Result<qring::QringGovernDto, String> {
    qring::govern(enabled)
}

#[tauri::command]
async fn qring_control(action: String) -> Result<qring::QringControlDto, String> {
    let cfg = load_config().unwrap_or_default();
    let act = action.trim().to_ascii_lowercase();
    let needs_vault = matches!(
        act.as_str(),
        "on" | "enable" | "start" | "restart" | "rerun" | "re-run" | "toggle"
    );
    // Toggle-off does not need the vault; toggle-on does — probe first.
    let will_start = match act.as_str() {
        "toggle" => !qring::status(&cfg.qring_pack, &cfg.qring_host).running,
        "off" | "disable" | "stop" => false,
        _ => needs_vault,
    };
    if will_start {
        let _ = ensure_unsealed(&cfg).await;
    }
    // Blocking CLI work off the async runtime.
    let pack = cfg.qring_pack.clone();
    let host = cfg.qring_host.clone();
    tokio::task::spawn_blocking(move || qring::control(&pack, &host, &action))
        .await
        .map_err(|e| format!("qring task: {e}"))?
}

#[tauri::command]
fn close_app(app: tauri::AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.close();
    }
}

/// How long the window may stay hidden waiting for the UI's reveal before we
/// show it anyway. Comfortably longer than a release build's webview startup.
const REVEAL_BACKSTOP_MS: u64 = 4_000;

/// The other half of `visible: false`.
///
/// Creating the window hidden is what makes a jump-free start possible — the UI
/// reveals it only after restoring the remembered spot, so the OS default
/// position is never on screen. But a widget with `skipTaskbar` and no tray
/// icon has no way back if the webview never gets that far (JS error before the
/// restore, WebView2 refusing to load): a permanently invisible window.
/// This timer is the floor under that, not the normal path.
fn arm_reveal_backstop(win: tauri::WebviewWindow) {
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(REVEAL_BACKSTOP_MS));
        if !matches!(win.is_visible(), Ok(true)) {
            crate::log_warn!("la UI no avisó de que estaba lista; muestro la ventana igual");
            let _ = win.show();
        }
    });
}

/// Where the widget opened last time; `None` on a first run (or an unreadable
/// file — never a reason to hold up a launch).
#[tauri::command]
fn get_placement() -> Option<placement::Placement> {
    placement::load()
}

/// The UI owns the geometry (which edge, what size, how it clamps), so it owns
/// the snapshot too: it calls this once a move has settled. Rust just remembers.
#[tauri::command]
fn save_placement(placement: placement::Placement) -> Result<(), String> {
    placement::save(&placement)
}

/// Reveal the window — see `arm_reveal_backstop`.
/// The committee's notices, as the pill's counter and the Avisos panel need
/// them: the tail of `steering-notices.jsonl` plus how many lines are still
/// unseen. Never fails: a missing or half-written file is "no notices", not a
/// broken header.
#[tauri::command]
fn notices_list(limit: Option<usize>) -> notices::Notices {
    notices::list(limit)
}

/// Call everything currently in the file seen. Returns the fresh list, so the
/// panel and the counter move in one round trip.
#[tauri::command]
fn notices_mark_seen() -> Result<notices::Notices, String> {
    notices::mark_seen()
}

#[tauri::command]
fn show_window(app: tauri::AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
    }
}

/// Is the left mouse button still held? `None` where the platform cannot tell
/// (macOS for now) so the UI falls back to deciding immediately — see `pointer`.
#[tauri::command]
fn pointer_left_down() -> Option<bool> {
    pointer::left_button_down()
}

/// The machine's regional locale (`en-US`, `es-ES`…). Picks the language of the
/// labels — see `locale`.
#[tauri::command]
fn system_locale() -> String {
    locale::user_locale()
}

pub(crate) async fn sync_index_inner(state: &AppState) -> Result<usize, String> {
    let (cfg, token) = ensure_ready(state).await?;
    let items = scan_index(&cfg, &token).await.map_err(|e| e.to_string())?;
    let n = {
        let store = state.store.lock();
        store.replace_index(&items)?
    };
    Ok(n)
}

#[tauri::command]
// The flat parameter list is the wire contract: the settings form sends these
// by name, one key per field (`invoke("save_settings", { baoAddr, ... })`).
// Grouping them into a DTO would change what every caller has to send for a
// lint, so the lint is silenced here instead — deliberately, and only here.
#[allow(clippy::too_many_arguments)]
fn save_settings(
    bao_addr: String,
    mount: String,
    namespaces: Vec<String>,
    token: Option<String>,
    auto_unseal: Option<bool>,
    auto_seal_idle_secs: Option<u64>,
    auto_collapse_idle_secs: Option<u64>,
    auto_sync: Option<bool>,
    auto_load_token: Option<bool>,
    init_path: Option<String>,
    toggles: Option<Vec<toggles::ToggleSpec>>,
    mcp_servers: Option<Vec<mcp::McpServer>>,
    services: Option<Vec<services::ServiceSpec>>,
) -> Result<(), String> {
    // `load_config_raw`, not `load_config`: the fallback defaults for omitted
    // fields must come from the file, never from a per-launch `EDGEPASS_*`
    // override that would then get written to disk as a real setting.
    let prev = load_config_raw().unwrap_or_default();
    // Validate against the *incoming* inventory: a save can add a service and
    // the LED that watches it in the same round trip, so `toggles` has to see
    // the services being stored, not only the ones already in the file.
    let next_services = match services {
        Some(list) => crate::services::validate(list)?,
        None => prev.services.clone(),
    };
    let mut with_services = prev.clone();
    with_services.services = next_services.clone();
    let next_toggles = match toggles {
        Some(list) => Some(crate::toggles::validate(&with_services, list)?),
        None => None,
    };
    let mut cfg = AppConfig {
        bao_addr: bao_addr.trim().trim_end_matches('/').to_string(),
        mount: mount.trim().trim_matches('/').to_string(),
        namespaces: namespaces
            .into_iter()
            .map(|n| n.trim().to_string())
            .collect(),
        auto_unseal: auto_unseal.unwrap_or(prev.auto_unseal),
        auto_seal_idle_secs: auto_seal_idle_secs.unwrap_or(prev.auto_seal_idle_secs),
        auto_collapse_idle_secs: auto_collapse_idle_secs
            .unwrap_or(prev.auto_collapse_idle_secs),
        auto_sync: auto_sync.unwrap_or(prev.auto_sync),
        auto_load_token: auto_load_token.unwrap_or(prev.auto_load_token),
        init_path: init_path
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or(prev.init_path),
        qring_pack: prev.qring_pack,
        qring_host: prev.qring_host,
        // The save-settings form does not carry governance: it is its own toggle (GOV), so the
        // stored policy is preserved rather than silently reset on every settings save.
        qring_governed: prev.qring_governed,
        // The header LEDs are a whole list owned by the editor: validate what
        // arrives (ids against the catalog, the LED cap) and keep the stored one
        // when the caller does not send it at all.
        toggles: next_toggles.unwrap_or(prev.toggles),
        // Same deal for the MCP list: it is owned by the MCP panel.
        mcp_servers: match mcp_servers {
            Some(list) => crate::mcp::validate(list)?,
            None => prev.mcp_servers,
        },
        // …and for the owner's services: the Servicios editor sends the whole list.
        services: next_services,
        // Y lo mismo para el runner local de GHA y sus rutas: los lleva el LED GW y
        // el script `gha-local.ps1`, no el formulario de Ajustes — así que un
        // *Guardar* no los borra.
        gha_runners: prev.gha_runners,
        gha_dir: prev.gha_dir,
        docker_desktop_exe: prev.docker_desktop_exe,
        daemoncurier_exe: prev.daemoncurier_exe,
        // `cloudflared` is a machine path, not a form field: the MCP panel names
        // the tunnel, and this points at the binary that runs it.
        cloudflared_exe: prev.cloudflared_exe,
        gha_watch: prev.gha_watch,
        // The passphrase style has its own command (`save_passphrase_config`):
        // a Guardar in Ajustes must not reset it to the defaults.
        passphrase: prev.passphrase,
        // The install salt belongs to the machine, not to the form.
        install_salt: prev.install_salt,
        config_version: config::CONFIG_VERSION,
    };
    if cfg.mount.is_empty() {
        cfg.mount = "kv".into();
    }
    if cfg.namespaces.is_empty() {
        cfg.namespaces.push(String::new());
    }
    let mut seen = std::collections::HashSet::new();
    cfg.namespaces.retain(|n| seen.insert(n.clone()));

    save_config(&cfg)?;
    if let Some(t) = token {
        let trimmed = t.trim();
        if !trimmed.is_empty() {
            set_token(trimmed)?;
        }
    }
    crate::log_info!(
        "ajustes guardados: addr={} mount={} ns={:?} autoUnseal={} autoSealIdle={}s autoCollapseIdle={}s autoSync={} autoToken={} toggles={}",
        cfg.bao_addr,
        cfg.mount,
        cfg.namespaces,
        cfg.auto_unseal,
        cfg.auto_seal_idle_secs,
        cfg.auto_collapse_idle_secs,
        cfg.auto_sync,
        cfg.auto_load_token,
        cfg.toggles
            .iter()
            .map(|t| t.id.as_str())
            .collect::<Vec<_>>()
            .join(",")
    );
    Ok(())
}

#[tauri::command]
fn clear_token() -> Result<(), String> {
    set_token("")
}

#[tauri::command]
async fn sync_index(state: State<'_, Arc<AppState>>) -> Result<usize, String> {
    sync_index_inner(state.inner()).await
}

#[tauri::command]
fn search(
    state: State<'_, Arc<AppState>>,
    query: String,
    limit: Option<usize>,
) -> Result<Vec<SearchHit>, String> {
    // Search is local index only — no need to open vault.
    let store = state.store.lock();
    store.search(&query, limit.unwrap_or(80))
}

#[tauri::command]
async fn reveal_and_copy(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    namespace: String,
    path: String,
    key: String,
) -> Result<String, String> {
    // Addressed by its stable reference, never by a database rowid: the index
    // is rewritten on every sync, so an id held by the UI could point at a
    // different secret by the time the user clicks.
    let (cfg, token) = ensure_ready(state.inner()).await?;
    let value = read_secret_value(&cfg, &token, &namespace, &path, &key)
        .await
        .map_err(|e| e.to_string())?;

    use tauri_plugin_clipboard_manager::ClipboardExt;
    app.clipboard()
        .write_text(value)
        .map_err(|e| e.to_string())?;

    {
        let store = state.store.lock();
        store.record_use_ref(&namespace, &path, &key)?;
        // Nota autogenerada: el acceso queda escrito *al lado* del secreto, en la
        // base local (el vault no guarda comentarios). `auto` la distingue de las
        // que escribe una persona o el agente.
        let _ = store.add_note(
            &namespace,
            &path,
            &key,
            "copiada al portapapeles desde el widget",
            "auto",
            "ui",
        );
    }

    let ns = if namespace.is_empty() {
        "root"
    } else {
        namespace.as_str()
    };
    crate::log_info!("copiado {ns}/{path} · {key}");
    Ok(format!("{ns}/{path} · {key}"))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretFieldDto {
    pub key: String,
    pub value: String,
}

/// Las notas de un path (o de un campo concreto). Nada de valores: sólo
/// referencias y texto escrito por una persona, por el agente o por edgePass.
#[tauri::command]
fn notes_list(
    state: State<'_, Arc<AppState>>,
    namespace: String,
    path: String,
    key: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<store::Note>, String> {
    let store = state.store.lock();
    store.notes_for(
        &namespace,
        &path,
        key.as_deref().filter(|k| !k.trim().is_empty()),
        limit.unwrap_or(50),
    )
}

/// Agrega una nota a mano (desde el widget o el agente).
#[tauri::command]
fn notes_add(
    state: State<'_, Arc<AppState>>,
    namespace: String,
    path: String,
    key: Option<String>,
    body: String,
    actor: Option<String>,
) -> Result<Vec<store::Note>, String> {
    if path.trim().is_empty() {
        return Err("path es requerido".into());
    }
    {
        let store = state.store.lock();
        store.add_note(
            &namespace,
            &path,
            key.as_deref().unwrap_or(""),
            &body,
            "user",
            actor.as_deref().unwrap_or("ui"),
        )?;
    }
    let store = state.store.lock();
    store.notes_for(&namespace, &path, None, 50)
}

/// Load every field of a path for the inline editor (new KV version on save).
#[tauri::command]
async fn read_secret_path(
    state: State<'_, Arc<AppState>>,
    namespace: String,
    path: String,
) -> Result<Vec<SecretFieldDto>, String> {
    let (cfg, token) = ensure_ready(state.inner()).await?;
    let ns = if namespace.trim() == "." {
        String::new()
    } else {
        namespace.trim().to_string()
    };
    let path = path.trim().trim_matches('/').to_string();
    if path.is_empty() {
        return Err("path es requerido".into());
    }
    let map = read_secret_map(&cfg, &token, &ns, &path)
        .await
        .map_err(|e| e.to_string())?;
    if !map.is_empty() {
        // El editor también es un acceso: una línea por vez que se lee el path
        // completo (`read_secret_path` es lo que usa Meter y el alta inline).
        let store = state.store.lock();
        let _ = store.add_note(
            &ns,
            &path,
            "",
            &format!("leído para editar ({} campos)", map.len()),
            "auto",
            "ui",
        );
    }
    Ok(map
        .into_iter()
        .map(|(key, value)| SecretFieldDto { key, value })
        .collect())
}

#[tauri::command]
async fn delete_secret(
    state: State<'_, Arc<AppState>>,
    namespace: String,
    path: String,
    key: String,
) -> Result<String, String> {
    let (cfg, token) = ensure_ready(state.inner()).await?;
    let ns = if namespace.trim() == "." {
        String::new()
    } else {
        namespace.trim().to_string()
    };
    let path = path.trim().trim_matches('/').to_string();
    let key = key.trim().to_string();
    if path.is_empty() || key.is_empty() {
        return Err("path y key son requeridos".into());
    }
    delete_secret_key(&cfg, &token, &ns, &path, &key)
        .await
        .map_err(|e| e.to_string())?;
    let _ = sync_index_inner(state.inner()).await;
    let label_ns = if ns.is_empty() { "root" } else { ns.as_str() };
    crate::log_info!("borrado {label_ns}/{path} · {key}");
    Ok(format!("Borrado {label_ns}/{path} · {key}"))
}

#[tauri::command]
async fn upsert_secret(
    state: State<'_, Arc<AppState>>,
    namespace: String,
    path: String,
    key: String,
    value: String,
    replace_all: Option<bool>,
) -> Result<String, String> {
    let (cfg, token) = ensure_ready(state.inner()).await?;
    let ns = if namespace.trim() == "." {
        String::new()
    } else {
        namespace.trim().to_string()
    };
    let path = path.trim().trim_matches('/').to_string();
    let key = key.trim().to_string();
    if path.is_empty() || key.is_empty() {
        return Err("path y key son requeridos".into());
    }
    if value.is_empty() {
        return Err("value no puede estar vacío".into());
    }

    upsert_secret_field(
        &cfg,
        &token,
        &ns,
        &path,
        &key,
        &value,
        replace_all.unwrap_or(false),
    )
    .await
    .map_err(|e| e.to_string())?;

    let _ = sync_index_inner(state.inner()).await;
    let label_ns = if ns.is_empty() { "root" } else { ns.as_str() };
    Ok(format!("OK {label_ns}/{path} · {key}"))
}

#[tauri::command]
fn take_pending_ingest(state: State<'_, Arc<AppState>>) -> Option<IngestFormDto> {
    state.pending_ingest.lock().take()
}

#[tauri::command]
fn peek_pending_ingest(state: State<'_, Arc<AppState>>) -> Option<IngestFormDto> {
    state.pending_ingest.lock().clone()
}

#[tauri::command]
fn peek_pending_grant(state: State<'_, Arc<AppState>>) -> Option<crate::grant::GrantRequestDto> {
    state.grants.peek_pending()
}

#[tauri::command]
async fn resolve_grant(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    id: String,
    decision: String,
) -> Result<serde_json::Value, String> {
    crate::ipc::apply_decision(state.inner().clone(), &app, &id, &decision).await
}

#[tauri::command]
async fn upsert_secret_batch(
    state: State<'_, Arc<AppState>>,
    namespace: String,
    path: String,
    fields: Vec<BatchField>,
    replace_all: Option<bool>,
) -> Result<String, String> {
    let (cfg, token) = ensure_ready(state.inner()).await?;
    let ns = if namespace.trim() == "." {
        String::new()
    } else {
        namespace.trim().to_string()
    };
    let path = path.trim().trim_matches('/').to_string();
    if path.is_empty() {
        return Err("path es requerido".into());
    }
    let pairs: Vec<(String, String)> = fields
        .into_iter()
        .map(|f| (f.key.trim().to_string(), f.value))
        .filter(|(k, v)| !k.is_empty() && !v.is_empty())
        .collect();
    if pairs.is_empty() {
        return Err("completa al menos una variable con valor".into());
    }

    let names: Vec<String> = pairs.iter().map(|(k, _)| k.clone()).collect();
    upsert_secret_map(
        &cfg,
        &token,
        &ns,
        &path,
        &pairs,
        replace_all.unwrap_or(false),
    )
    .await
    .map_err(|e| e.to_string())?;

    {
        let mut slot = state.pending_ingest.lock();
        *slot = None;
    }
    let _ = sync_index_inner(state.inner()).await;
    let label_ns = if ns.is_empty() { "root" } else { ns.as_str() };
    Ok(format!("OK {label_ns}/{path} · {} keys", names.len()))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginImportRow {
    pub slug: String,
    pub name: String,
    pub url: String,
    pub username: String,
    pub password: String,
    pub notes: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginImportResult {
    pub imported: usize,
    pub skipped: usize,
    pub paths: Vec<String>,
    pub message: String,
}

/// Import browser/Google password rows into `personal/logins/{slug}`
/// (root namespace). Each path gets username/password/url/name/notes fields.
#[tauri::command]
async fn import_logins(
    state: State<'_, Arc<AppState>>,
    rows: Vec<LoginImportRow>,
    replace_all: Option<bool>,
) -> Result<LoginImportResult, String> {
    if rows.is_empty() {
        return Err("no hay filas para importar".into());
    }
    let (cfg, token) = ensure_ready(state.inner()).await?;
    let replace = replace_all.unwrap_or(true);
    let mut imported = 0usize;
    let mut skipped = 0usize;
    let mut paths = Vec::new();

    for row in rows {
        let slug = row
            .slug
            .trim()
            .trim_matches('/')
            .to_ascii_lowercase()
            .replace([' ', '\\'], "-");
        if slug.is_empty() {
            skipped += 1;
            continue;
        }
        if row.password.trim().is_empty() && row.username.trim().is_empty() {
            skipped += 1;
            continue;
        }
        let path = format!("personal/logins/{slug}");
        let mut fields: Vec<(String, String)> = Vec::new();
        if !row.name.trim().is_empty() {
            fields.push(("name".into(), row.name.trim().to_string()));
        }
        if !row.url.trim().is_empty() {
            fields.push(("url".into(), row.url.trim().to_string()));
        }
        if !row.username.trim().is_empty() {
            fields.push(("username".into(), row.username.trim().to_string()));
        }
        if !row.password.trim().is_empty() {
            fields.push(("password".into(), row.password));
        }
        if let Some(notes) = row.notes.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
            fields.push(("notes".into(), notes.to_string()));
        }
        if fields.is_empty() {
            skipped += 1;
            continue;
        }

        upsert_secret_map(&cfg, &token, "", &path, &fields, replace)
            .await
            .map_err(|e| format!("{path}: {e}"))?;
        imported += 1;
        paths.push(path);
    }

    let _ = sync_index_inner(state.inner()).await;
    crate::log_info!("import logins: {imported} ok, {skipped} omitidos");
    Ok(LoginImportResult {
        message: format!("Importados {imported} · omitidos {skipped}"),
        imported,
        skipped,
        paths,
    })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsDto {
    pub version: String,
    pub exe: String,
    pub build_time: Option<String>,
    pub os: String,
    pub config_path: String,
    pub db_path: String,
    pub log_path: String,
    pub webview2: String,
    pub indexed: i64,
    pub last_sync: Option<String>,
    pub has_token: bool,
    pub init_path: String,
    pub bao_addr: String,
    pub mount: String,
    pub namespaces: Vec<String>,
    pub auto_unseal: bool,
    pub auto_seal_idle_secs: u64,
    pub auto_collapse_idle_secs: u64,
    pub auto_sync: bool,
    pub auto_load_token: bool,
    pub seal: SealStatusDto,
    pub service: ServiceStatusDto,
    pub log_tail: String,
    /// Detached runs: runId + state + exit only (never secret values).
    pub runs: Vec<serde_json::Value>,
}

fn path_or_dash(path: Result<std::path::PathBuf, String>) -> String {
    path.map(|p| p.display().to_string())
        .unwrap_or_else(|_| "-".into())
}

/// Everything needed to debug a bad install in one call: which binary is
/// actually running, where its files live, what the vault and the Windows
/// service look like, and the tail of the log. The Diagnóstico panel renders
/// it and copies it as a report.
#[tauri::command]
async fn diagnostics(state: State<'_, Arc<AppState>>) -> Result<DiagnosticsDto, String> {
    let cfg = load_config().unwrap_or_default();
    let seal = seal_status(&cfg).await;
    let (indexed, last_sync) = {
        let store = state.store.lock();
        (store.count().unwrap_or(0), store.last_sync().ok().flatten())
    };

    let exe = std::env::current_exe().ok();
    // The exe's own timestamp is a better build id than a version string: it
    // answers "which build am I actually running?".
    let build_time = exe
        .as_ref()
        .and_then(|p| std::fs::metadata(p).ok())
        .and_then(|m| m.modified().ok())
        .map(|t| {
            let dt: chrono::DateTime<chrono::Local> = t.into();
            dt.format("%Y-%m-%d %H:%M:%S").to_string()
        });

    Ok(DiagnosticsDto {
        version: env!("CARGO_PKG_VERSION").to_string(),
        exe: exe
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "-".into()),
        build_time,
        os: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
        config_path: path_or_dash(config::config_path()),
        db_path: path_or_dash(config::db_path()),
        log_path: logging::log_path_string(),
        webview2: webview2_version(),
        indexed,
        last_sync,
        has_token: has_token(),
        init_path: cfg.init_path.clone(),
        bao_addr: cfg.bao_addr.clone(),
        mount: cfg.mount.clone(),
        namespaces: cfg.namespaces.clone(),
        auto_unseal: cfg.auto_unseal,
        auto_seal_idle_secs: cfg.auto_seal_idle_secs,
        auto_collapse_idle_secs: cfg.auto_collapse_idle_secs,
        auto_sync: cfg.auto_sync,
        auto_load_token: cfg.auto_load_token,
        seal,
        service: winservice::status(),
        log_tail: logging::tail(40),
        runs: state.runs.snapshot(),
    })
}

/// WebView2 runtime version. "Which engine did this build render in" is the
/// first question when a window comes up blank or shows an error page.
#[cfg(windows)]
fn webview2_version() -> String {
    const CLIENT: &str = r"Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}";
    for hive in ["HKLM\\SOFTWARE\\WOW6432Node", "HKCU\\SOFTWARE"] {
        let key = format!("{hive}\\{CLIENT}");
        let out = match winservice::run_hidden("reg.exe", &["query", &key, "/v", "pv"]) {
            Ok(out) if out.status.success() => out,
            _ => continue,
        };
        let text = String::from_utf8_lossy(&out.stdout);
        for line in text.lines() {
            if let Some(idx) = line.find("REG_SZ") {
                let value = line[idx + "REG_SZ".len()..].trim();
                if !value.is_empty() {
                    return value.to_string();
                }
            }
        }
    }
    "no encontrado (¿WebView2 Runtime instalado?)".into()
}

#[cfg(not(windows))]
fn webview2_version() -> String {
    "n/a".into()
}

/// The Diagnóstico panel copies through here, so the frontend needs no
/// clipboard binding of its own.
#[tauri::command]
fn copy_to_clipboard(app: tauri::AppHandle, text: String) -> Result<(), String> {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    app.clipboard().write_text(text).map_err(|e| e.to_string())
}

fn spawn_vault_guardian(state: Arc<AppState>) {
    tauri::async_runtime::spawn(async move {
        // Initial settle
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            let cfg = match load_config() {
                Ok(c) => c,
                Err(_) => continue,
            };

            let status = seal_status(&cfg).await;
            if !status.reachable || !status.initialized {
                continue;
            }

            // Auto-open if somehow sealed and we want it ready for upcoming use
            // is NOT done here — only on demand via ensure_ready.
            // Auto-close when idle:
            // A caller asked us to keep it open for a while.
            if now_secs() < state.hold_until.load(Ordering::Relaxed) {
                continue;
            }

            if !status.sealed && cfg.auto_seal_idle_secs > 0 {
                let last = state.last_activity.load(Ordering::Relaxed);
                if last > 0 {
                    let idle = now_secs().saturating_sub(last);
                    if idle >= cfg.auto_seal_idle_secs {
                        if let Ok(Some(token)) = config::get_token() {
                            match seal(&cfg, &token).await {
                                Ok(s) if s.sealed => {
                                    crate::log_warn!(
                                        "auto-seal tras {idle}s de inactividad del widget: el vault queda cerrado para TODOS sus clientes"
                                    );
                                }
                                Ok(_) => {}
                                Err(e) => crate::log_error!("auto-seal falló: {e}"),
                            }
                        }
                    }
                }
            }
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    logging::init();

    // A second launch must not become a second widget: two widgets means two
    // guardians, two SQLite handles and a race over the vault's seal state.
    // Single instance without pulling in another plugin.
    //
    // The mutex is the authority and it has no race; `notify_existing` only adds
    // the courtesy of surfacing the widget that is already running.
    match ipc::claim_single_instance() {
        Ok(true) => {}
        Ok(false) => {
            crate::log_warn!(
                "edgePass ya está corriendo en esta sesión: este proceso se cierra para no competir por \\\\.\\pipe\\edgepass"
            );
            ipc::notify_existing();
            return;
        }
        Err(e) => crate::log_warn!(
            "no pude tomar el mutex de instancia única ({e}); sigo, y dos instancias podrían competir por el pipe"
        ),
    }
    if ipc::notify_existing() {
        crate::log_info!("ya hay una instancia corriendo: le paso el foco y salgo");
        return;
    }

    let cfg = load_config().unwrap_or_default();
    crate::log_info!(
        "config: addr={} mount={} ns={:?} autoUnseal={} autoSync={} autoToken={} autoSealIdleSecs={} init={}",
        cfg.bao_addr,
        cfg.mount,
        cfg.namespaces,
        cfg.auto_unseal,
        cfg.auto_sync,
        cfg.auto_load_token,
        cfg.auto_seal_idle_secs,
        cfg.init_path
    );

    // Startup failures used to `expect()` -> panic -> abort with no console and
    // no trace. Now they land in the log.
    let db = match config::db_path() {
        Ok(p) => p,
        Err(e) => {
            crate::log_error!("no pude resolver el directorio de datos: {e}");
            return;
        }
    };
    let store = match Store::open(&db) {
        Ok(s) => s,
        Err(e) => {
            crate::log_error!("no pude abrir la base de datos {}: {e}", db.display());
            return;
        }
    };
    crate::log_info!("db en {}", db.display());

    let state = Arc::new(AppState {
        store: Mutex::new(store),
        last_activity: AtomicU64::new(now_secs()),
        hold_until: AtomicU64::new(0),
        pending_ingest: Mutex::new(None),
        runs: crate::ipc::DetachedRuns::new(),
        grants: crate::grant::GrantBoard::new(),
        gha_announced: Mutex::new(std::collections::HashMap::new()),
    });

    let guardian_state = state.clone();
    let ipc_state = state.clone();

    let result = tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(state)
        .setup(move |app| {
            if let Some(win) = app.get_webview_window("main") {
                match win.url() {
                    Ok(u) => crate::log_info!("webview url={u}"),
                    Err(e) => crate::log_warn!("webview url no disponible: {e}"),
                }
                arm_reveal_backstop(win);
            }
            crate::log_info!("ventana lista (oculta hasta que la UI restaure su sitio)");
            spawn_vault_guardian(guardian_state);
            ipc::spawn(ipc_state, app.handle().clone());
            // MCP servers flagged `autostart` come up with the widget, which the
            // `edgePass` logon task already starts. Off the setup path, so a slow
            // spawn — or a vault round trip when a row declares `secretEnv` —
            // never delays the window.
            {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    let cfg = match load_config() {
                        Ok(c) => c,
                        Err(e) => {
                            crate::log_warn!("MCP autostart: no pude leer la config: {e}");
                            return;
                        }
                    };
                    if cfg.mcp_servers.is_empty() {
                        return;
                    }
                    let mut secrets = mcp::Secrets::new();
                    if mcp::any_needs_vault(&cfg) {
                        let state = handle.state::<Arc<AppState>>().inner().clone();
                        match ensure_ready(&state).await {
                            Ok((fresh, token)) => secrets = mcp::resolve_all(&fresh, &token).await,
                            Err(e) => crate::log_warn!("MCP autostart sin vault: {e}"),
                        }
                    }
                    let notes = tauri::async_runtime::spawn_blocking(move || {
                        mcp::autostart_all(&cfg, &secrets)
                    })
                    .await
                    .unwrap_or_default();
                    for note in notes {
                        crate::log_info!("MCP autostart: {note}");
                    }
                });
            }
            // El visor de logs del runner local se abre solo: cada 3 s se mira si algún
            // runner arrancó un job y, si lo hizo, se le abre la ventana (`-Watch`).
            // Existe porque el runner no vuelca la salida de los steps a su stdout: sin
            // esto, para saber que algo corre (o por qué falló) hay que ir a GitHub.
            // `ghaWatch:false` lo apaga; el LED sigue diciendo qué job está en curso.
            {
                let state = app.state::<Arc<AppState>>().inner().clone();
                tauri::async_runtime::spawn(async move {
                    loop {
                        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                        let Ok(cfg) = load_config() else { continue };
                        if !cfg.gha_watch || cfg.gha_runners.is_empty() {
                            continue;
                        }
                        let probe = cfg.clone();
                        let live = match tauri::async_runtime::spawn_blocking(move || {
                            crate::gha::live_jobs(&probe)
                        })
                        .await
                        {
                            Ok(v) => v,
                            Err(_) => continue,
                        };
                        let fresh = {
                            let seen = state.gha_announced.lock();
                            crate::gha::new_jobs(&seen, &live)
                        };
                        for (id, job) in fresh {
                            state
                                .gha_announced
                                .lock()
                                .insert(id.clone(), job.started.clone());
                            crate::log_info!(
                                "runner {id}: arrancó el job «{}» — abro el visor de logs",
                                job.name
                            );
                            if let Err(e) = crate::gha::open_viewer(&cfg, &id) {
                                crate::log_warn!("no pude abrir el visor de logs: {e}");
                            }
                        }
                    }
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_status,
            ui_log,
            notes_list,
            notes_add,
            refresh_seal,
            mcp_status,
            mcp_control,
            mcp_mint_token,
            mcp_credential_command,
            make_passphrase,
            passphrase_config,
            save_passphrase_config,
            unseal_vault,
            seal_vault,
            load_token_from_init,
            bootstrap,
            save_settings,
            validate_services,
            open_setup_link,
            clear_token,
            sync_index,
            search,
            reveal_and_copy,
            read_secret_path,
            delete_secret,
            upsert_secret,
            upsert_secret_batch,
            import_logins,
            take_pending_ingest,
            peek_pending_ingest,
            peek_pending_grant,
            resolve_grant,
            openbao_service_status,
            openbao_service_start,
            autostart_status,
            autostart_install,
            autostart_remove,
            qring_status,
            qring_control,
            qring_govern,
            fire_toggle,
            diagnostics,
            copy_to_clipboard,
            close_app,
            get_placement,
            save_placement,
            notices_list,
            notices_mark_seen,
            show_window,
            pointer_left_down,
            system_locale,
        ])
        .run(tauri::generate_context!());

    if let Err(e) = result {
        crate::log_error!("error fatal al correr edgePass: {e}");
    }
}

#[cfg(test)]
mod setup_tests {
    use super::*;

    fn seal(reachable: bool, initialized: bool, sealed: bool) -> SealStatusDto {
        SealStatusDto {
            reachable,
            health_code: if sealed { 503 } else { 200 },
            initialized,
            sealed,
            progress: 0,
            threshold: 3,
            version: None,
            message: "sonda".into(),
        }
    }

    #[test]
    fn a_fresh_install_points_at_the_install_guide() {
        let hint = setup_hint(&AppConfig::default(), &seal(false, false, true), false);
        assert!(hint.needed);
        assert_eq!(hint.code, "vault_unreachable");
        assert!(hint.steps.len() >= 3);
        assert!(hint.install_url.contains("openbao.org"));
        assert!(hint.releases_url.contains("openbao/openbao"));
    }

    #[test]
    fn a_ready_vault_needs_nothing() {
        let hint = setup_hint(&AppConfig::default(), &seal(true, true, false), true);
        assert!(!hint.needed);
        assert_eq!(hint.code, "ready");
        assert!(hint.steps.is_empty());
    }

    #[test]
    fn an_uninitialized_vault_is_named() {
        let hint = setup_hint(&AppConfig::default(), &seal(true, false, true), false);
        assert_eq!(hint.code, "vault_uninitialized");
    }

    #[test]
    fn a_missing_token_is_named() {
        let hint = setup_hint(&AppConfig::default(), &seal(true, true, false), false);
        assert_eq!(hint.code, "no_token");
    }

    #[test]
    fn a_missing_init_file_is_named_even_with_a_token() {
        let mut cfg = AppConfig::default();
        cfg.init_path = r"C:\no\existe\init.json".into();
        let hint = setup_hint(&cfg, &seal(true, true, false), true);
        assert_eq!(hint.code, "init_file_missing");
    }

    #[test]
    fn a_sealed_vault_without_an_init_file_is_named() {
        let hint = setup_hint(&AppConfig::default(), &seal(true, true, true), true);
        assert_eq!(hint.code, "vault_sealed");
    }
}

#[cfg(test)]
mod salt_tests {
    use super::decode_hex;

    #[test]
    fn hex_round_trips_and_rejects_noise() {
        assert_eq!(decode_hex("00ff10"), vec![0x00, 0xff, 0x10]);
        assert_eq!(decode_hex(" 0A0b "), vec![0x0a, 0x0b]);
        assert!(decode_hex("0").is_empty(), "largo impar");
        assert!(decode_hex("zz").is_empty(), "no es hex");
        assert!(decode_hex("").is_empty());
    }
}
