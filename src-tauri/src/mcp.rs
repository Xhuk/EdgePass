//! MCP servers that edgePass can start, watch and restart.
//!
//! The list is *data*, like the header LEDs: `config.json` → `mcpServers[]`. A
//! row names a command and how to recognise its process; edgePass starts it,
//! restarts it and — when the row says `autostart` — brings it up with the
//! widget, which is already launched at logon by the `edgePass` Scheduled Task.
//!
//! Supervision reuses GW's mechanism (`qring::status`): keep the child we
//! spawned *and* ask `tasklist` for the image, so a server someone started by
//! hand is seen instead of being started twice.
//!
//! A row may also name a **Cloudflare named tunnel** (`tunnel`). That is the
//! missing half of "the server is up but nobody can reach it": `reasonixmcp`
//! listens on `127.0.0.1:8788` and `cloudflared tunnel run edgepass-mcp` is what
//! puts it on `https://mcp.networkvoid.xyz`. The tunnel is supervised exactly
//! like the server — started with it, stopped with it, `autostart` included —
//! because a chain where half the links come up by hand is a chain that is down
//! every time the machine reboots.
//!
//! Two things are deliberately narrow:
//!
//! - **No secrets here.** `env` is for non-secret values (flags, paths). A
//!   server that needs a credential reads it itself — `reasonixmcp` reads its
//!   gate secret from its own state dir — so nothing sensitive is written to
//!   `config.json`, which is the same rule the rest of the widget follows.
//! - **Stopping never sprays.** A child edgePass spawned is killed by pid. A
//!   process started elsewhere is only matched through `tasklist` when the image
//!   is distinctive; for a generic interpreter (`node.exe`, `python.exe`…) the
//!   kill is refused with an explanation instead of taking out unrelated work.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::config::AppConfig;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Cap on the list. Unlike the header LEDs this is not a layout budget: the
/// panel scrolls. It exists so a runaway config cannot turn one `tasklist` per
/// poll into hundreds.
pub const MAX_SERVERS: usize = 24;

/// Longest label the panel shows before truncating.
pub const MAX_LABEL_CHARS: usize = 24;

/// `tasklist` costs tens of milliseconds and `get_status` runs on a poll, so the
/// process sweep is cached. Same TTL idea as `skills::STATUS_TTL`.
const STATUS_TTL: Duration = Duration::from_secs(10);

/// Interpreters whose name says nothing about *which* work is running: killing
/// by image would take out unrelated processes, so it is refused.
const GENERIC_IMAGES: &[&str] = &[
    "node.exe",
    "npx.cmd",
    "npm.cmd",
    "python.exe",
    "python3.exe",
    "py.exe",
    "cmd.exe",
    "powershell.exe",
    "pwsh.exe",
    "deno.exe",
    "bun.exe",
    "java.exe",
    "ruby.exe",
    "perl.exe",
];

/// One server, as stored in `config.json` and as the panel sends it back.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServer {
    /// Stable key: what the panel and the pipe use to address one server.
    pub id: String,
    /// What the panel and the tooltip show. Defaults to the id.
    #[serde(default)]
    pub label: String,
    /// Executable to run. An absolute path is best; a bare name is looked up on
    /// `PATH`, exactly like a shell would.
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// Working directory for the child. Empty = inherit edgePass's.
    #[serde(default)]
    pub cwd: String,
    /// Process image `tasklist` should look for. Empty = the file name of
    /// `command` with `.exe` when it has no extension.
    ///
    /// Worth setting by hand for a server launched through an interpreter,
    /// because the image is then the interpreter and not the server.
    #[serde(default)]
    pub image: String,
    /// Extra environment for the child. **Not for credentials** — see the module
    /// docs. Values here are written to `config.json` in clear text.
    #[serde(default)]
    pub env: Vec<McpEnv>,
    /// Credentials for the child, read from OpenBao **at spawn time** and put in
    /// its environment only. This is where a gate secret belongs: it never
    /// reaches `config.json`, the log, or the pipe.
    #[serde(default)]
    pub secret_env: Vec<McpSecretEnv>,
    /// Public origin this server is reachable at, for the connection block: the
    /// tunnel origin, e.g. `https://host.tailnet.ts.net:8443`. Empty = the
    /// server is only reachable locally and the block says so.
    #[serde(default)]
    pub public_url: String,
    /// Named Cloudflare tunnel that publishes `public_url` — the name you would
    /// type after `cloudflared tunnel run` (`edgepass-mcp`). Empty = the row is
    /// local only, and edgePass does not start any connector for it.
    ///
    /// Name, not URL: the routing (`config.yml`) and the credentials live in
    /// `~/.cloudflared`, which is Cloudflare's own contract and not something
    /// this widget should copy into `config.json`.
    #[serde(default)]
    pub tunnel: String,
    /// Path the MCP endpoint answers on. Defaults to `/mcp`.
    #[serde(default)]
    pub path: String,
    /// Clients this server is meant to be attached to (free text, for the
    /// panel's checklist: `Gemini`, `ChatGPT`, `Claude`…). Documentation only.
    #[serde(default)]
    pub clients: Vec<String>,
    /// Optional command that mints a client credential (for example
    /// `reasonixmcp -register-client -redirect …`). edgePass runs it on demand
    /// and shows what it printed, because only the *server* can issue a valid
    /// client id — the widget cannot invent one.
    #[serde(default)]
    pub credential_command: String,
    /// Start with the widget. Pair it with the `edgePass` logon task in Ajustes
    /// and the servers come up with the session.
    #[serde(default)]
    pub autostart: bool,
}

/// One environment entry for a child process.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpEnv {
    pub key: String,
    #[serde(default)]
    pub value: String,
}

/// One credential read from OpenBao and handed to a child process.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpSecretEnv {
    /// Environment variable name the child reads.
    pub key: String,
    /// Namespace in the vault (`""` = the config's first namespace).
    #[serde(default)]
    pub namespace: String,
    /// KV path holding the value.
    pub path: String,
    /// Field inside that secret.
    pub field: String,
}

/// One row of the panel: the **whole spec** plus what the process sweep sees.
///
/// The spec is flattened in on purpose, the same way `ToggleView` carries its
/// `ToggleSpec`: the panel edits this list and sends it back on save, so a row
/// that published only a summary would lose `env` and `secretEnv` on the way
/// back — a silent edit of a config the user cannot see.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerView {
    #[serde(flatten)]
    pub spec: McpServer,
    pub running: bool,
    /// `corriendo (pid 1234)` · `parado` · `falta el binario`
    pub detail: String,
    /// The row declares a named tunnel **and** its connector is running. `false`
    /// covers both "no tunnel declared" (see `spec.tunnel`) and "declared but
    /// down", which is the state the MCP LED refuses to call "on".
    pub tunnel_running: bool,
    /// The URL a client pastes into its "MCP server" field, or "" when the row
    /// has no public origin (then the server is local-only and the panel says
    /// so instead of offering a dead link).
    pub url: String,
    /// The image `tasklist` is watched for — the resolved one, which is not
    /// always what `spec.image` says.
    pub watched_image: String,
    /// Environment variable names the child receives. Only names: a value never
    /// leaves the vault.
    pub env_keys: Vec<String>,
    pub secret_env_keys: Vec<String>,
}

/// Path the MCP endpoint answers on, with a leading slash.
pub fn path_of(s: &McpServer) -> String {
    let raw = s.path.trim();
    if raw.is_empty() {
        return "/mcp".to_string();
    }
    if raw.starts_with('/') {
        raw.trim_end_matches('/').to_string()
    } else {
        format!("/{}", raw.trim_end_matches('/'))
    }
}

/// What a client pastes into its "MCP server URL" field.
pub fn connection_url(s: &McpServer) -> String {
    let base = s.public_url.trim().trim_end_matches('/');
    if base.is_empty() {
        return String::new();
    }
    format!("{}{}", base, path_of(s))
}

/// Mints a token from the OS CSPRNG, hex-encoded.
///
/// No RNG crate: `BCryptGenRandom` on Windows, `/dev/urandom` elsewhere. 32
/// bytes (64 hex chars) is the size used for every credential this module
/// produces.
pub fn generate_token(bytes: usize) -> Result<String, String> {
    let mut buf = vec![0u8; bytes];
    os_random(&mut buf)?;
    Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
}

/// Delegates to [`crate::rand`], the single source of randomness in the app, so
/// a token and a passphrase can never disagree about where the entropy came from.
fn os_random(buf: &mut [u8]) -> Result<(), String> {
    crate::rand::fill(buf)
}

/// Reads a list's `secretEnv` entries from the vault.
///
/// Called at spawn time, so the values exist only in the child's environment: they are
/// not written to `config.json`, not logged and not carried by the pipe.
pub async fn resolve_env_for(
    cfg: &AppConfig,
    token: &str,
    owner: &str,
    wants: &[McpSecretEnv],
) -> Result<Vec<(String, String)>, String> {
    let mut out = Vec::with_capacity(wants.len());
    for want in wants {
        let namespace = if want.namespace.trim().is_empty() {
            cfg.namespaces.first().cloned().unwrap_or_default()
        } else {
            want.namespace.trim().to_string()
        };
        let value =
            crate::openbao::read_secret_value(cfg, token, &namespace, &want.path, &want.field)
                .await
                .map_err(|e| format!("{owner}: no pude leer {namespace}/{} ({e})", want.path))?;
        out.push((want.key.clone(), value));
    }
    Ok(out)
}

/// Reads a server's `secretEnv` from the vault. See [`resolve_env_for`].
pub async fn resolve_secrets(
    cfg: &AppConfig,
    token: &str,
    s: &McpServer,
) -> Result<Vec<(String, String)>, String> {
    let owner = label_for(&s.label, &s.id);
    resolve_env_for(cfg, token, &owner, &s.secret_env).await
}

/// Cap on what a credential command may print back to the panel.
pub const MAX_CREDENTIAL_OUTPUT: usize = 4096;

/// Runs the row's `credential_command` and returns what it printed.
///
/// Only the *server* can mint a valid client id — the widget cannot invent one —
/// so this runs the command the row declares and hands back its output for
/// copying. It goes through a shell (`cmd /C`, `sh -c`) because a config field
/// like `reasonixmcp -register-client -redirect …` is meant to read the way you
/// would type it; it is config data, exactly as trusted as `command`.
pub fn run_credential_command(cfg: &AppConfig, id: &str) -> Result<String, String> {
    let s = find(cfg, id)?;
    let line = s.credential_command.trim();
    if line.is_empty() {
        return Err(format!(
            "'{}' no tiene comando de credenciales: cargalo en el panel MCP",
            s.label
        ));
    }
    #[cfg(windows)]
    let mut cmd = {
        let mut c = Command::new("cmd");
        c.args(["/C", line]);
        c
    };
    #[cfg(not(windows))]
    let mut cmd = {
        let mut c = Command::new("sh");
        c.args(["-c", line]);
        c
    };
    if !s.cwd.is_empty() {
        cmd.current_dir(&s.cwd);
    }
    quiet(&mut cmd);
    let out = cmd
        .output()
        .map_err(|e| format!("no pude correr el comando de credenciales: {e}"))?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let text = text.trim();
    if text.is_empty() {
        return Err("el comando de credenciales no imprimió nada".to_string());
    }
    Ok(if text.len() > MAX_CREDENTIAL_OUTPUT {
        let mut cut = MAX_CREDENTIAL_OUTPUT;
        while cut > 0 && !text.is_char_boundary(cut) {
            cut -= 1;
        }
        format!("{}\n…[cortado]", &text[..cut])
    } else {
        text.to_string()
    })
}

/// Whether starting this list needs the vault open: if no row declares a
/// `secretEnv`, the vault is left alone.
pub fn any_needs_vault(cfg: &AppConfig) -> bool {
    cfg.mcp_servers.iter().any(|s| !s.secret_env.is_empty())
}

/// Resolves every row's `secretEnv`. The caller opens the vault first, and only
/// when [`any_needs_vault`] says so.
pub async fn resolve_all(cfg: &AppConfig, token: &str) -> Secrets {
    let mut out = Secrets::new();
    for s in &cfg.mcp_servers {
        if s.secret_env.is_empty() {
            continue;
        }
        match resolve_secrets(cfg, token, s).await {
            Ok(pairs) => {
                out.insert(
                    s.id.clone(),
                    Resolved {
                        pairs,
                        error: None,
                    },
                );
            }
            Err(e) => note_error(&mut out, &s.id, e),
        }
    }
    out
}

/// The whole list, for `get_status` and the MCP LED's tooltip.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpStatusDto {
    pub total: usize,
    pub running: usize,
    pub autostart: usize,
    /// Rows that declare a named tunnel, and how many of those connectors are
    /// running. Separate from `running` on purpose: a row can be up while the
    /// world still cannot reach it.
    pub tunnels: usize,
    pub tunnels_up: usize,
    /// Something needs a fix, not a click: a missing server binary or a missing
    /// `cloudflared`. The LED paints this amber; a stopped process is just off.
    pub broken: bool,
    /// One line for the LED's tooltip.
    pub detail: String,
    pub servers: Vec<ServerView>,
}

impl McpStatusDto {
    fn empty(message: &str) -> Self {
        Self {
            total: 0,
            running: 0,
            autostart: 0,
            tunnels: 0,
            tunnels_up: 0,
            broken: false,
            detail: message.to_string(),
            servers: Vec::new(),
        }
    }

    /// `true` when every configured server is up **and** every declared tunnel
    /// is connected — what the LED calls "on". A server nobody can reach is not
    /// an "on" chain; that is the whole reason the tunnel is part of the probe.
    pub fn all_running(&self) -> bool {
        self.total > 0 && self.running == self.total && self.tunnels_up == self.tunnels
    }
}

/// Trims the list and normalizes ids/labels. Refuses what cannot work.
pub fn validate(list: Vec<McpServer>) -> Result<Vec<McpServer>, String> {
    if list.len() > MAX_SERVERS {
        return Err(format!(
            "demasiados servidores MCP: {} (máximo {MAX_SERVERS})",
            list.len()
        ));
    }
    let mut used: Vec<String> = Vec::new();
    let mut out = Vec::with_capacity(list.len());
    for mut s in list {
        s.command = s.command.trim().to_string();
        s.cwd = s.cwd.trim().to_string();
        s.image = s.image.trim().to_string();
        s.public_url = s.public_url.trim().trim_end_matches('/').to_string();
        s.tunnel = s.tunnel.trim().to_string();
        s.path = path_of(&s);
        s.credential_command = s.credential_command.trim().to_string();
        s.clients = s
            .clients
            .into_iter()
            .map(|c| c.trim().to_string())
            .filter(|c| !c.is_empty())
            .collect();
        s.args = s
            .args
            .into_iter()
            .map(|a| a.trim().to_string())
            .filter(|a| !a.is_empty())
            .collect();
        s.env = s
            .env
            .into_iter()
            .filter_map(|e| {
                let key = e.key.trim().to_string();
                if key.is_empty() {
                    None
                } else {
                    Some(McpEnv { key, value: e.value })
                }
            })
            .collect();
        s.secret_env = s
            .secret_env
            .into_iter()
            .filter_map(|e| {
                let key = e.key.trim().to_string();
                let path = e.path.trim().to_string();
                let field = e.field.trim().to_string();
                if key.is_empty() || path.is_empty() || field.is_empty() {
                    None
                } else {
                    Some(McpSecretEnv {
                        key,
                        namespace: e.namespace.trim().to_string(),
                        path,
                        field,
                    })
                }
            })
            .collect();
        if s.command.is_empty() {
            return Err(format!(
                "servidor MCP '{}': falta el comando a ejecutar",
                s.id.trim()
            ));
        }
        s.id = unique_id(&s.id, &s.command, &used);
        used.push(s.id.clone());
        s.label = label_for(&s.label, &s.id);
        out.push(s);
    }
    Ok(out)
}

/// Derives a unique id: the given one, else the command's file stem.
fn unique_id(raw: &str, command: &str, used: &[String]) -> String {
    let base = {
        let candidate = slug(raw);
        if !candidate.is_empty() {
            candidate
        } else {
            let stem = std::path::Path::new(command)
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            let candidate = slug(&stem);
            if candidate.is_empty() {
                "server".to_string()
            } else {
                candidate
            }
        }
    };
    if !used.iter().any(|u| u == &base) {
        return base;
    }
    for n in 2..1000 {
        let candidate = format!("{base}-{n}");
        if !used.iter().any(|u| u == &candidate) {
            return candidate;
        }
    }
    base
}

fn slug(raw: &str) -> String {
    raw.trim()
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

fn label_for(label: &str, id: &str) -> String {
    let raw = if label.trim().is_empty() { id } else { label.trim() };
    let mut out: String = raw.chars().take(MAX_LABEL_CHARS).collect();
    if out.is_empty() {
        out = id.to_string();
    }
    out
}

/// The process image to look for.
pub fn image_of(s: &McpServer) -> String {
    if !s.image.is_empty() {
        return s.image.to_ascii_lowercase();
    }
    let name = std::path::Path::new(&s.command)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    if name.is_empty() {
        return String::new();
    }
    if name.contains('.') {
        name.to_ascii_lowercase()
    } else {
        format!("{}.exe", name.to_ascii_lowercase())
    }
}

/// Builds one panel row. One place, because `views_of` and `status_now` must not
/// drift in what they publish.
fn view_of(s: &McpServer, running: bool, detail: String, tunnel_running: bool) -> ServerView {
    let secret_env_keys: Vec<String> = s.secret_env.iter().map(|e| e.key.clone()).collect();
    let mut env_keys: Vec<String> = s.env.iter().map(|e| e.key.clone()).collect();
    env_keys.extend(secret_env_keys.iter().cloned());
    // The row carries the normalized spec (label and path filled in), which is
    // both what the panel shows and what it sends back on save.
    let mut spec = s.clone();
    spec.label = label_for(&s.label, &s.id);
    spec.path = path_of(s);
    ServerView {
        spec,
        running,
        detail,
        tunnel_running,
        url: connection_url(s),
        watched_image: image_of(s),
        env_keys,
        secret_env_keys,
    }
}

/// Cached status: the probe runs on a 5 s poll, and each unresolved row costs a
/// `tasklist`.
pub fn status(cfg: &AppConfig) -> McpStatusDto {
    {
        let guard = CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(cache) = guard.as_ref() {
            if cache.at.elapsed() < STATUS_TTL {
                return cache.value.clone();
            }
        }
    }
    let value = status_now(cfg);
    let mut guard = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    *guard = Some(Cache {
        at: Instant::now(),
        value: value.clone(),
    });
    value
}

/// Reads every row now, ignoring the cache.
pub fn status_now(cfg: &AppConfig) -> McpStatusDto {
    if cfg.mcp_servers.is_empty() {
        return McpStatusDto::empty("sin servidores MCP configurados");
    }
    let mut servers = Vec::with_capacity(cfg.mcp_servers.len());
    let mut running = 0;
    let mut autostart = 0;
    let mut tunnels = 0;
    let mut tunnels_up = 0;
    let mut broken = false;
    let mut missing: Vec<String> = Vec::new();
    let mut tunnels_down: Vec<String> = Vec::new();
    let mut tunnels_up_names: Vec<String> = Vec::new();
    let mut dead: Vec<String> = Vec::new();
    for s in &cfg.mcp_servers {
        if s.autostart {
            autostart += 1;
        }
        let (is_up, mut detail) = read_one(s);
        // A row that died under our hands says so, exit code included: that is
        // the difference between "click it" and "go read the log".
        if !is_up && detail.starts_with("murió") {
            dead.push(format!("{}: {detail}", s.label));
        }
        let tunnel = tunnel_state(cfg, s);
        if tunnel.declared {
            tunnels += 1;
            if tunnel.running {
                tunnels_up += 1;
                tunnels_up_names.push(s.tunnel.trim().to_string());
            } else {
                tunnels_down.push(tunnel.detail.clone());
            }
            if tunnel.broken {
                broken = true;
            }
            detail = format!("{detail} · {}", tunnel.detail);
        }
        if is_up {
            running += 1;
        }
        if detail.starts_with("falta el binario") {
            missing.push(s.label.clone());
            broken = true;
        }
        servers.push(view_of(s, is_up, detail, tunnel.running));
    }
    let total = servers.len();
    let mut detail = if !missing.is_empty() {
        format!(
            "{} de {total} corriendo · falta el binario: {}",
            running,
            missing.join(", ")
        )
    } else if running == total {
        format!("{total} de {total} corriendo")
    } else {
        format!(
            "{running} de {total} corriendo · {} · {} con autostart",
            if total - running == 1 {
                "1 parado".to_string()
            } else {
                format!("{} parados", total - running)
            },
            autostart
        )
    };
    if !dead.is_empty() {
        detail = format!("{detail} · {}", dead.join(" · "));
    }
    if !tunnels_down.is_empty() {
        detail = format!("{detail} · {}", tunnels_down.join(" · "));
    } else if !tunnels_up_names.is_empty() {
        // Say the half that nobody can see from `tasklist`: the connector is up,
        // so the public origin is really publishing this server.
        detail = format!("{detail} · túnel {} arriba", tunnels_up_names.join(", "));
    }
    McpStatusDto {
        total,
        running,
        autostart,
        tunnels,
        tunnels_up,
        broken,
        detail,
        servers,
    }
}

/// What a row's tunnel is doing. `declared == false` is the common case: a row
/// with no `tunnel` is local-only and nothing is expected of it.
#[derive(Debug, Clone, PartialEq)]
pub struct TunnelState {
    pub declared: bool,
    pub running: bool,
    /// `cloudflared` itself is missing: a fix, not a click.
    pub broken: bool,
    pub detail: String,
}

impl TunnelState {
    fn none() -> Self {
        Self {
            declared: false,
            running: false,
            broken: false,
            detail: String::new(),
        }
    }
}

/// Image every Cloudflare connector shows up as in `tasklist`.
const CLOUDFLARED_IMAGE: &str = "cloudflared.exe";

/// Credentials resolved for one server: the pairs to put in its environment, or
/// the reason they could not be read.
#[derive(Debug, Clone, Default)]
pub struct Resolved {
    pub pairs: Vec<(String, String)>,
    pub error: Option<String>,
}

/// Resolved `secretEnv` per server id, filled by the async caller that can open
/// the vault. Keeping the resolution out of this module is what lets the spawn
/// path stay synchronous.
pub type Secrets = HashMap<String, Resolved>;

/// Records that a server's credentials could not be read, so the start path can
/// refuse it instead of launching it half-configured.
pub fn note_error(secrets: &mut Secrets, id: &str, message: String) {
    secrets.entry(id.to_string()).or_default().error = Some(message);
}

fn env_for<'a>(secrets: &'a Secrets, id: &str) -> &'a [(String, String)] {
    secrets.get(id).map(|r| r.pairs.as_slice()).unwrap_or(&[])
}

fn secret_error<'a>(secrets: &'a Secrets, id: &str) -> Option<&'a str> {
    secrets.get(id).and_then(|r| r.error.as_deref())
}

/// Starts every row flagged `autostart`. Called from the app's setup: anything
/// already up is left alone, and rows whose secrets could not be read are
/// reported instead of started half-configured.
pub fn autostart_all(cfg: &AppConfig, secrets: &Secrets) -> Vec<String> {
    let mut notes = Vec::new();
    for s in cfg.mcp_servers.iter().filter(|s| s.autostart) {
        if let Some(err) = secret_error(secrets, &s.id) {
            notes.push(format!("{}: {err}", s.label));
            continue;
        }
        match start_one(cfg, s, env_for(secrets, &s.id)) {
            Ok(msg) => {
                // Anything that actually started (the server, its tunnel) is
                // worth a log line; "ya estaba corriendo" is not.
                if msg.contains("arrancado") {
                    notes.push(format!("{} ({msg})", s.label));
                }
            }
            Err(e) => notes.push(format!("{}: {e}", s.label)),
        }
    }
    if !notes.is_empty() {
        invalidate();
    }
    notes
}

/// Starts one server by id. Idempotent: an already-running row is reported, not
/// started twice.
pub fn start(cfg: &AppConfig, id: &str, secrets: &Secrets) -> Result<String, String> {
    let s = find(cfg, id)?;
    if let Some(err) = secret_error(secrets, &s.id) {
        return Err(err.to_string());
    }
    let out = start_one(cfg, s, env_for(secrets, &s.id));
    invalidate();
    out.map(|m| format!("{}: {m}", s.label))
}

/// Stops one server by id. See the module docs for why this is careful.
pub fn stop(cfg: &AppConfig, id: &str) -> Result<String, String> {
    let s = find(cfg, id)?;
    let out = stop_one(s);
    invalidate();
    out.map(|m| format!("{}: {m}", s.label))
}

/// Restarts one server by id — what `GW`'s `alt` does for the gateway.
pub fn restart(cfg: &AppConfig, id: &str, secrets: &Secrets) -> Result<String, String> {
    let s = find(cfg, id)?;
    if let Some(err) = secret_error(secrets, &s.id) {
        return Err(err.to_string());
    }
    let out = restart_one(cfg, s, env_for(secrets, &s.id));
    invalidate();
    out.map(|m| format!("{}: {m}", s.label))
}

/// Starts every configured server.
pub fn start_all(cfg: &AppConfig, secrets: &Secrets) -> Result<String, String> {
    let out = each(cfg, |s| {
        if let Some(err) = secret_error(secrets, &s.id) {
            return Err(err.to_string());
        }
        start_one(cfg, s, env_for(secrets, &s.id))
    });
    invalidate();
    Ok(out)
}

/// Stops every configured server.
pub fn stop_all(cfg: &AppConfig) -> Result<String, String> {
    let out = each(cfg, stop_one);
    invalidate();
    Ok(out)
}

/// Restarts every configured server.
pub fn restart_all(cfg: &AppConfig, secrets: &Secrets) -> Result<String, String> {
    let out = each(cfg, |s| {
        if let Some(err) = secret_error(secrets, &s.id) {
            return Err(err.to_string());
        }
        restart_one(cfg, s, env_for(secrets, &s.id))
    });
    invalidate();
    Ok(out)
}

/// Runs one operation over the list, collecting per-row results so a single bad
/// row cannot hide the others.
fn each(cfg: &AppConfig, op: impl Fn(&McpServer) -> Result<String, String>) -> String {
    if cfg.mcp_servers.is_empty() {
        return "sin servidores MCP configurados".to_string();
    }
    let mut parts = Vec::with_capacity(cfg.mcp_servers.len());
    for s in &cfg.mcp_servers {
        parts.push(match op(s) {
            Ok(msg) => format!("{}: {msg}", s.label),
            Err(e) => format!("{}: {e}", s.label),
        });
    }
    parts.join(" · ")
}

fn find<'a>(cfg: &'a AppConfig, id: &str) -> Result<&'a McpServer, String> {
    let id = id.trim();
    cfg.mcp_servers.iter().find(|s| s.id == id).ok_or_else(|| {
        format!(
            "servidor MCP desconocido: '{id}' · hay: {}",
            if cfg.mcp_servers.is_empty() {
                "(ninguno)".to_string()
            } else {
                cfg.mcp_servers
                    .iter()
                    .map(|s| s.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        )
    })
}

fn start_one(cfg: &AppConfig, s: &McpServer, extra: &[(String, String)]) -> Result<String, String> {
    if !binary_present(s) {
        return Err(format!("falta el binario: {}", s.command));
    }
    let mut parts: Vec<String> = Vec::new();
    match alive_pid(s) {
        // A stopped process is just off; nothing gets started twice.
        Some(0) => parts.push(format!("ya estaba corriendo ({})", image_of(s))),
        Some(pid) => parts.push(format!("ya estaba corriendo (pid {pid})")),
        None => parts.push(spawn_server(s, extra)?),
    }
    // The connector comes up right after the thing it publishes. A tunnel that
    // refuses to start is reported, not fatal: the server it fronts is fine.
    match start_tunnel(cfg, s) {
        Ok(msg) if !msg.is_empty() => parts.push(msg),
        Ok(_) => {}
        Err(e) => parts.push(e),
    }
    Ok(parts.join(" · "))
}

/// Spawns the server itself and reads back what happened.
///
/// Its output goes to `%APPDATA%\edgePass\logs\mcp-<id>.log` instead of `null`:
/// a widget has no console, so a server that dies on startup (port taken, bad
/// config, gate secret rejected) used to leave *nothing* — the log said
/// "arrancado (pid 30432)" and ten minutes later the port was empty with no way
/// to know why. The file is appended to, so a restart does not erase the reason
/// the last one died.
fn spawn_server(s: &McpServer, extra: &[(String, String)]) -> Result<String, String> {
    let mut cmd = Command::new(&s.command);
    cmd.args(&s.args);
    cmd.stdin(Stdio::null());
    match server_log(s) {
        Some((out, err)) => {
            cmd.stdout(out);
            cmd.stderr(err);
        }
        // No log dir we can write to: better a blind child than no child.
        None => {
            cmd.stdout(Stdio::null());
            cmd.stderr(Stdio::null());
        }
    }
    for e in &s.env {
        cmd.env(&e.key, &e.value);
    }
    // Credentials last: a `secretEnv` entry wins over a plain `env` entry with
    // the same name, because the vault is the source of truth for a secret.
    for (key, value) in extra {
        cmd.env(key, value);
    }
    if !s.cwd.is_empty() {
        cmd.current_dir(&s.cwd);
    }
    quiet(&mut cmd);
    match cmd.spawn() {
        Ok(child) => {
            let pid = child.id();
            let mut guard = CHILDREN.lock().unwrap_or_else(|e| e.into_inner());
            guard
                .get_or_insert_with(HashMap::new)
                .insert(s.id.clone(), child);
            // A fresh start makes the last death old news: the probe should say
            // "corriendo", not "murió ayer con código 1".
            forget_exit(&s.id);
            Ok(format!("arrancado (pid {pid})"))
        }
        Err(e) => Err(format!("no pude arrancar '{}': {e}", s.command)),
    }
}

/// Stops a row: the connector first (so nobody hits a refused connection while
/// the port is being closed), then the server.
fn stop_one(s: &McpServer) -> Result<String, String> {
    let mut parts: Vec<String> = Vec::new();
    match stop_tunnel(s) {
        Ok(msg) if !msg.is_empty() => parts.push(msg),
        Ok(_) => {}
        Err(e) => return Err(e),
    }
    parts.push(stop_server(s)?);
    Ok(parts.join(" · "))
}

/// Stops the server itself. See the module docs for why this is careful.
fn stop_server(s: &McpServer) -> Result<String, String> {
    // 1. The process we own: exact pid, no collateral.
    {
        let mut guard = CHILDREN.lock().unwrap_or_else(|e| e.into_inner());
        let ours = guard.as_mut().and_then(|map| map.remove(&s.id));
        if let Some(mut child) = ours {
            let pid = child.id();
            let _ = child.kill();
            let _ = child.wait();
            return Ok(format!("detenido (pid {pid})"));
        }
    }
    // 2. A process started elsewhere: only if the image is distinctive.
    let image = image_of(s);
    if image.is_empty() {
        return Err("no sé qué proceso detener: poné 'command' o 'image'".to_string());
    }
    if !alive_image(&image) {
        return Ok("ya estaba parado".to_string());
    }
    if is_generic_image(&image) {
        return Err(format!(
            "no lo detengo: '{image}' es un intérprete genérico y matar por imagen se llevaría trabajo ajeno. \
             Poné 'image' con el proceso real del servidor, o paralo desde su propia consola"
        ));
    }
    match kill_by_image(&image) {
        true => Ok(format!("detenido (por imagen {image})")),
        false => Err(format!("no pude detener {image}")),
    }
}

fn restart_one(cfg: &AppConfig, s: &McpServer, extra: &[(String, String)]) -> Result<String, String> {
    let stopped = match stop_one(s) {
        Ok(msg) => msg,
        // A generic image cannot be stopped by name; say so and still try to
        // bring it up, because "already running" is a fine restart result.
        Err(e) => {
            let started = start_one(cfg, s, extra)?;
            return Ok(format!("{e} · {started}"));
        }
    };
    // Brief settle so `tasklist` and the pid release before we spawn again.
    std::thread::sleep(Duration::from_millis(250));
    let started = start_one(cfg, s, extra)?;
    Ok(format!("{stopped} · {started}"))
}

fn invalidate() {
    let mut guard = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    *guard = None;
}

/// `Some(pid)` when the server is up: the child we hold, else the image.
fn alive_pid(s: &McpServer) -> Option<u32> {
    alive_child(&s.id, &image_of(s))
}

/// The same question for any key in the child map — the server and its tunnel
/// share one mechanism. `Some(0)` means "we held a child and it has exited".
fn alive_child(key: &str, image: &str) -> Option<u32> {
    let mut exited: Option<i32> = None;
    {
        let mut guard = CHILDREN.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(map) = guard.as_mut() {
            let ours = match map.get_mut(key) {
                Some(child) => match child.try_wait() {
                    Ok(None) => Some(child.id()),
                    Ok(Some(status)) => {
                        exited = Some(status.code().unwrap_or(-1));
                        Some(0)
                    }
                    Err(_) => Some(0),
                },
                None => None,
            };
            match ours {
                Some(pid) if pid != 0 => return Some(pid),
                // Exited on its own: drop the handle so the image check decides.
                Some(_) => {
                    map.remove(key);
                }
                None => {}
            }
        }
    }
    // Outside the child lock on purpose: two mutexes, always in this order.
    if let Some(code) = exited {
        record_exit(key, code);
    }
    if image.is_empty() || !alive_image(image) {
        return None;
    }
    // We know it is up but not which pid: `tasklist` only answered "there is one".
    Some(0)
}

fn read_one(s: &McpServer) -> (bool, String) {
    if !binary_present(s) {
        return (false, format!("falta el binario: {}", s.command));
    }
    match alive_pid(s) {
        Some(0) => (true, format!("corriendo ({})", image_of(s))),
        Some(pid) => (true, format!("corriendo (pid {pid})")),
        None => {
            // "parado" is a click; "murió (código 1)" is a diagnosis.
            let note = exit_note(&s.id, &server_log_name(s));
            if note.is_empty() {
                (false, "parado".to_string())
            } else {
                (false, note)
            }
        }
    }
}

/// Whether the command can be run at all: an absolute path must exist, a bare
/// name just has to be non-empty (it may live on `PATH`).
fn binary_present(s: &McpServer) -> bool {
    path_present(&s.command)
}

fn path_present(raw: &str) -> bool {
    let path = std::path::Path::new(raw);
    if path.is_absolute() {
        return path.is_file();
    }
    !raw.trim().is_empty()
}

// ── the named tunnel ─────────────────────────────────────────────────────────
//
// A server on `127.0.0.1:8788` is only half of a chain a hosted client can use:
// the other half is `cloudflared`, which is what `https://mcp.networkvoid.xyz`
// actually resolves to. So a row that names a `tunnel` owns a *second* child, and
// the probe answers for both — a green LED over an unreachable origin is exactly
// the lie this widget exists to avoid.

/// Where `cloudflared` is. Empty in the config means `PATH`, which is the trap:
/// the MSI/winget install drops the exe in `%LOCALAPPDATA%\cloudflared` and does
/// not touch `PATH`, so a hand-typed `cloudflared tunnel run …` fails while the
/// binary is right there.
fn cloudflared_exe(cfg: &AppConfig) -> PathBuf {
    let raw = cfg.cloudflared_exe.trim();
    if !raw.is_empty() {
        return PathBuf::from(raw);
    }
    if let Ok(env) = std::env::var("EDGEPASS_CLOUDFLARED") {
        let e = env.trim();
        if !e.is_empty() {
            return PathBuf::from(e);
        }
    }
    PathBuf::from("cloudflared")
}

/// The key that row's connector uses in the child map, separate from the
/// server's so each can be held — and stopped — on its own.
fn tunnel_key(s: &McpServer) -> String {
    format!("{}#tunnel", s.id)
}

/// Where the connector logs. edgePass sends a child's output to a file (see
/// [`append_log`]), but `cloudflared` has its own `--logfile`: using it keeps the
/// connector's JSON lines in its native format instead of ours.
fn tunnel_log_name(s: &McpServer) -> String {
    format!("cloudflared-{}.log", slug(&s.id))
}

fn tunnel_log_path(s: &McpServer) -> Option<PathBuf> {
    let dir = crate::logging::log_dir();
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join(tunnel_log_name(s)))
}

/// Is the row's tunnel up? Answers `declared: false` when the row names none.
pub fn tunnel_state(cfg: &AppConfig, s: &McpServer) -> TunnelState {
    let name = s.tunnel.trim();
    if name.is_empty() {
        return TunnelState::none();
    }
    let exe = cloudflared_exe(cfg);
    if !path_present(&exe.to_string_lossy()) {
        return TunnelState {
            declared: true,
            running: false,
            broken: true,
            detail: format!("falta cloudflared para el túnel {name}: {}", exe.display()),
        };
    }
    // Our own child first; otherwise whoever answers to that image. A single
    // connector per machine is the normal case — `stop_tunnel` still kills by
    // pid (see below), so naming a second tunnel on the same box is what would
    // make this read optimistic, not a stop that takes the wrong process out.
    let running = alive_child(&tunnel_key(s), CLOUDFLARED_IMAGE).is_some();
    let note = exit_note(&tunnel_key(s), &tunnel_log_name(s));
    TunnelState {
        declared: true,
        running,
        broken: false,
        detail: if running {
            format!("túnel {name} arriba")
        } else if !note.is_empty() {
            format!("túnel {name} {note}")
        } else {
            format!("túnel {name} parado")
        },
    }
}

/// Starts the row's connector. Idempotent, like the server's start.
fn start_tunnel(cfg: &AppConfig, s: &McpServer) -> Result<String, String> {
    let name = s.tunnel.trim();
    if name.is_empty() {
        return Ok(String::new());
    }
    if tunnel_state(cfg, s).running {
        return Ok(format!("túnel {name} ya estaba arriba"));
    }
    let exe = cloudflared_exe(cfg);
    if !path_present(&exe.to_string_lossy()) {
        return Err(format!(
            "falta cloudflared: {} · poné cloudflaredExe en config.json",
            exe.display()
        ));
    }
    let mut cmd = Command::new(&exe);
    // Flags before the subcommand: `--logfile` is cloudflared's own, and it is
    // what makes a connector that dies on preflight leave a trace.
    if let Some(log) = tunnel_log_path(s) {
        cmd.arg("--logfile").arg(log);
    }
    cmd.args(["tunnel", "run", name]);
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::null());
    cmd.stderr(Stdio::null());
    quiet(&mut cmd);
    match cmd.spawn() {
        Ok(child) => {
            let pid = child.id();
            let mut guard = CHILDREN.lock().unwrap_or_else(|e| e.into_inner());
            guard
                .get_or_insert_with(HashMap::new)
                .insert(tunnel_key(s), child);
            forget_exit(&tunnel_key(s));
            Ok(format!("túnel {name} arrancado (pid {pid})"))
        }
        Err(e) => Err(format!(
            "no pude arrancar '{}' para el túnel {name}: {e}",
            exe.display()
        )),
    }
}

/// Stops the row's connector: our own child by pid, and then **any** other
/// connector running that tunnel, matched by pid on the command line. Never by
/// image: another `cloudflared` on this machine may be serving a different tunnel,
/// and taking it down would be the kind of collateral this module refuses
/// everywhere else.
fn stop_tunnel(s: &McpServer) -> Result<String, String> {
    let name = s.tunnel.trim();
    if name.is_empty() {
        return Ok(String::new());
    }
    let mut parts: Vec<String> = Vec::new();
    {
        let mut guard = CHILDREN.lock().unwrap_or_else(|e| e.into_inner());
        let ours = guard.as_mut().and_then(|map| map.remove(&tunnel_key(s)));
        if let Some(mut child) = ours {
            let pid = child.id();
            let _ = child.kill();
            let _ = child.wait();
            parts.push(format!("pid {pid}"));
        }
    }
    // A connector started by hand (or one whose pid we lost) is stopped too:
    // "off" that leaves a stray connector publishing the origin is not off.
    let extra = stop_connectors_by_name(name);
    if extra > 0 {
        parts.push(format!("{extra} conector(es) más"));
    }
    if parts.is_empty() {
        return Ok(format!("túnel {name} ya estaba parado"));
    }
    Ok(format!("túnel {name} detenido ({})", parts.join(" · ")))
}

/// Kills every `cloudflared` whose command line runs that tunnel. Returns how
/// many it stopped. One PowerShell/CIM call, only on the stop path — the 5 s
/// poll never pays for it.
fn stop_connectors_by_name(name: &str) -> usize {
    let safe: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    if safe.is_empty() {
        return 0;
    }
    #[cfg(windows)]
    {
        let script = format!(
            "$n = 0\n\
             Get-CimInstance Win32_Process -Filter \"Name='cloudflared.exe'\" |\n\
               Where-Object {{ $_.CommandLine -match 'tunnel\\s+run\\s+{safe}(\\s|$)' }} |\n\
               ForEach-Object {{ Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue; $n++ }}\n\
             $n"
        );
        let mut cmd = Command::new("powershell");
        cmd.args(["-NoProfile", "-Command", &script]);
        quiet(&mut cmd);
        match cmd.output() {
            Ok(out) => String::from_utf8_lossy(&out.stdout)
                .trim()
                .parse::<usize>()
                .unwrap_or(0),
            Err(_) => 0,
        }
    }
    #[cfg(not(windows))]
    {
        let _ = safe;
        0
    }
}

fn is_generic_image(image: &str) -> bool {
    GENERIC_IMAGES.contains(&image.to_ascii_lowercase().as_str())
}

/// `true` when `tasklist` sees at least one process with that image.
fn alive_image(image: &str) -> bool {
    #[cfg(windows)]
    {
        let mut cmd = Command::new("tasklist");
        cmd.args(["/FI", &format!("IMAGENAME eq {image}"), "/NH"]);
        quiet(&mut cmd);
        match cmd.output() {
            Ok(out) => {
                let text = String::from_utf8_lossy(&out.stdout).to_ascii_lowercase();
                text.contains(&image.to_ascii_lowercase())
            }
            Err(_) => false,
        }
    }
    #[cfg(not(windows))]
    {
        let _ = image;
        false
    }
}

/// Kills every process with that image. Only reached for a distinctive name.
fn kill_by_image(image: &str) -> bool {
    #[cfg(windows)]
    {
        let mut cmd = Command::new("powershell");
        cmd.args([
            "-NoProfile",
            "-Command",
            &format!(
                "Get-Process -Name '{}' -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue",
                image.replace(".exe", "").replace('\'', "''")
            ),
        ]);
        quiet(&mut cmd);
        cmd.status().map(|s| s.success()).unwrap_or(false)
    }
    #[cfg(not(windows))]
    {
        let _ = image;
        false
    }
}

fn quiet(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
}

// ── logs, and the last words of a child ──────────────────────────────────────
//
// edgePass is a widget: a child it spawns inherits no console, so its output
// goes nowhere unless we send it somewhere — and a server that dies on startup
// (port taken, bad config) then leaves *nothing* behind. That is exactly how a
// dead `reasonixmcp` looked "arrancado (pid 30432)" in the log with an empty port
// ten minutes later. Two cheap pieces answer "why": a per-row log file, and the
// exit code of the last child we held.

/// Cap before a row's log is rotated to `<name>.1`.
const MAX_CHILD_LOG_BYTES: u64 = 1024 * 1024;

/// Appends to `logs/<file>`, rotating once past the cap. Returns the handles the
/// child needs; both point at the same file, because the order *between* stdout
/// and stderr is what makes a stack trace readable.
fn append_log(file: &str) -> Option<(Stdio, Stdio)> {
    let dir = crate::logging::log_dir();
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(file);
    let too_big = std::fs::metadata(&path)
        .map(|m| m.len() > MAX_CHILD_LOG_BYTES)
        .unwrap_or(false);
    if too_big {
        let _ = std::fs::rename(&path, dir.join(format!("{file}.1")));
    }
    let out = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .ok()?;
    let err = out.try_clone().ok()?;
    Some((Stdio::from(out), Stdio::from(err)))
}

/// The row's server log, by file name (also what the tooltip points at).
fn server_log_name(s: &McpServer) -> String {
    format!("mcp-{}.log", slug(&s.id))
}

fn server_log(s: &McpServer) -> Option<(Stdio, Stdio)> {
    append_log(&server_log_name(s))
}

/// Exit code of the last child we held per key (`<id>` for a server,
/// `<id>#tunnel` for its connector). Cleared by a successful start.
static LAST_EXIT: Mutex<Option<HashMap<String, i32>>> = Mutex::new(None);

fn record_exit(key: &str, code: i32) {
    let mut guard = LAST_EXIT.lock().unwrap_or_else(|e| e.into_inner());
    guard
        .get_or_insert_with(HashMap::new)
        .insert(key.to_string(), code);
}

fn forget_exit(key: &str) {
    let mut guard = LAST_EXIT.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(map) = guard.as_mut() {
        map.remove(key);
    }
}

/// `murió (código 3) · log: mcp-x.log`, or "" when we never held that child.
fn exit_note(key: &str, log: &str) -> String {
    let guard = LAST_EXIT.lock().unwrap_or_else(|e| e.into_inner());
    match guard.as_ref().and_then(|m| m.get(key)) {
        Some(code) => format!("murió (código {code}) · log: {log}"),
        None => String::new(),
    }
}

struct Cache {
    at: Instant,
    value: McpStatusDto,
}

static CACHE: Mutex<Option<Cache>> = Mutex::new(None);

/// Children edgePass spawned, kept for exact-pid supervision.
static CHILDREN: Mutex<Option<HashMap<String, Child>>> = Mutex::new(None);

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(id: &str, command: &str) -> McpServer {
        McpServer {
            id: id.into(),
            label: String::new(),
            command: command.into(),
            args: Vec::new(),
            cwd: String::new(),
            image: String::new(),
            env: Vec::new(),
            secret_env: Vec::new(),
            public_url: String::new(),
            tunnel: String::new(),
            path: String::new(),
            clients: Vec::new(),
            credential_command: String::new(),
            autostart: false,
        }
    }

    fn none() -> Secrets {
        Secrets::new()
    }

    #[test]
    fn validate_fills_ids_and_labels_and_refuses_a_missing_command() {
        let out = validate(vec![spec("", r"C:\tools\reasonixmcp.exe")]).unwrap();
        assert_eq!(out[0].id, "reasonixmcp");
        assert_eq!(out[0].label, "reasonixmcp");
        assert!(validate(vec![spec("x", "   ")]).is_err());
    }

    #[test]
    fn ids_are_made_unique_so_a_click_addresses_one_row() {
        let out = validate(vec![spec("dup", "a"), spec("dup", "b"), spec("dup", "c")]).unwrap();
        let ids: Vec<&str> = out.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["dup", "dup-2", "dup-3"]);
    }

    #[test]
    fn the_list_is_capped() {
        let many: Vec<McpServer> = (0..MAX_SERVERS + 1)
            .map(|i| spec(&format!("s{i}"), "x.exe"))
            .collect();
        assert!(validate(many).is_err());
    }

    #[test]
    fn image_defaults_to_the_command_file_name() {
        assert_eq!(image_of(&spec("a", r"C:\tools\reasonixmcp.exe")), "reasonixmcp.exe");
        assert_eq!(image_of(&spec("a", "my-server")), "my-server.exe");
        assert_eq!(image_of(&spec("a", "MY-SERVER.EXE")), "my-server.exe");
        let mut explicit = spec("a", "node");
        explicit.image = "McpThing.exe".into();
        assert_eq!(image_of(&explicit), "mcpthing.exe");
    }

    #[test]
    fn a_generic_interpreter_is_never_killed_by_image() {
        // Killing every node.exe would take out unrelated servers, so the guard
        // has to hold for exactly these names.
        for image in ["node.exe", "python.exe", "cmd.exe", "pwsh.exe"] {
            assert!(is_generic_image(image), "{image} should be treated as generic");
        }
        assert!(!is_generic_image("reasonixmcp.exe"));
    }

    #[test]
    fn an_empty_list_reads_as_configured_nothing() {
        let cfg = AppConfig::default();
        let status = status_now(&cfg);
        assert_eq!(status.total, 0);
        assert!(!status.all_running());
        assert!(status.detail.contains("sin servidores"));
    }

    #[test]
    fn a_missing_binary_is_reported_not_spawned() {
        let cfg = AppConfig {
            mcp_servers: vec![spec("ghost", r"C:\no\existe\ghost.exe")],
            ..AppConfig::default()
        };
        let status = status_now(&cfg);
        assert_eq!(status.total, 1);
        assert_eq!(status.running, 0);
        assert!(status.servers[0].detail.starts_with("falta el binario"));
        assert!(start(&cfg, "ghost", &none()).is_err());
    }

    #[test]
    fn an_unknown_id_is_refused_and_the_error_lists_the_known_ones() {
        let cfg = AppConfig {
            mcp_servers: validate(vec![spec("uno", "uno.exe")]).unwrap(),
            ..AppConfig::default()
        };
        let err = stop(&cfg, "nope").unwrap_err();
        assert!(err.contains("nope") && err.contains("uno"), "got: {err}");
    }

    #[test]
    fn the_path_defaults_to_mcp_and_the_url_is_origin_plus_path() {
        let mut s = spec("reasonixmcp", r"C:\tools\reasonixmcp.exe");
        assert_eq!(path_of(&s), "/mcp");
        assert_eq!(connection_url(&s), "", "no public origin, no link to offer");

        s.public_url = "https://host.tailnet.ts.net:8443/".into();
        let out = validate(vec![s]).unwrap();
        assert_eq!(out[0].public_url, "https://host.tailnet.ts.net:8443");
        assert_eq!(out[0].path, "/mcp");
        assert_eq!(connection_url(&out[0]), "https://host.tailnet.ts.net:8443/mcp");
    }

    #[test]
    fn a_path_without_a_leading_slash_is_normalized() {
        let mut s = spec("x", "x.exe");
        s.path = "mcp/".into();
        assert_eq!(path_of(&s), "/mcp");
    }

    #[test]
    fn a_secret_ref_without_a_field_is_dropped_not_kept_half_configured() {
        let mut s = spec("x", "x.exe");
        s.secret_env = vec![
            McpSecretEnv {
                key: "OK".into(),
                namespace: String::new(),
                path: "projects/x".into(),
                field: "value".into(),
            },
            McpSecretEnv {
                key: "BAD".into(),
                namespace: String::new(),
                path: "projects/x".into(),
                field: "  ".into(),
            },
        ];
        let out = validate(vec![s]).unwrap();
        assert_eq!(out[0].secret_env.len(), 1);
        assert_eq!(out[0].secret_env[0].key, "OK");
    }

    #[test]
    fn the_panel_publishes_variable_names_but_never_a_secret_value() {
        let mut s = spec("x", "x.exe");
        s.env = vec![McpEnv {
            key: "MCP_MODE".into(),
            value: "prod".into(),
        }];
        s.secret_env = vec![McpSecretEnv {
            key: "GATE_SECRET".into(),
            namespace: String::new(),
            path: "projects/x".into(),
            field: "gate".into(),
        }];
        let cfg = AppConfig {
            mcp_servers: validate(vec![s]).unwrap(),
            ..AppConfig::default()
        };
        let row = &status_now(&cfg).servers[0];
        assert!(row.env_keys.contains(&"MCP_MODE".to_string()));
        assert!(row.env_keys.contains(&"GATE_SECRET".to_string()));
        assert_eq!(row.secret_env_keys, vec!["GATE_SECRET".to_string()]);

        let rendered = serde_json::to_string(&cfg.mcp_servers).unwrap();
        // A plain `env` value is not a secret: it lives in `config.json` in clear
        // text. That is the documented trade-off, and the reason `secretEnv`
        // exists as a separate thing.
        assert!(rendered.contains("prod"), "plain env serializes: {rendered}");

        // A `secretEnv` entry has nowhere to put a value: only where to read it.
        let secret = serde_json::to_string(&cfg.mcp_servers[0].secret_env).unwrap();
        assert!(secret.contains("\"path\":\"projects/x\""));
        assert!(secret.contains("\"field\":\"gate\""));
        assert!(
            !secret.contains("\"value\""),
            "a secretEnv must never carry a value: {secret}"
        );
    }

    #[test]
    fn a_token_is_hex_of_the_requested_size_and_not_repeated() {
        let a = generate_token(32).unwrap();
        let b = generate_token(32).unwrap();
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b, "two mints must not collide");
    }

    #[test]
    fn autostart_reports_a_row_whose_secret_could_not_be_read() {
        let mut s = spec("x", "x.exe");
        s.autostart = true;
        let cfg = AppConfig {
            mcp_servers: vec![s],
            ..AppConfig::default()
        };
        let mut secrets = Secrets::new();
        note_error(&mut secrets, "x", "no pude leer el vault".into());
        let notes = autostart_all(&cfg, &secrets);
        assert_eq!(notes.len(), 1);
        assert!(notes[0].contains("no pude leer el vault"), "got {notes:?}");
    }

    #[test]
    fn the_vault_is_left_alone_when_no_row_asks_for_it() {
        let plain = AppConfig {
            mcp_servers: validate(vec![spec("x", "x.exe")]).unwrap(),
            ..AppConfig::default()
        };
        assert!(!any_needs_vault(&plain));

        let mut s = spec("y", "y.exe");
        s.secret_env = vec![McpSecretEnv {
            key: "K".into(),
            namespace: String::new(),
            path: "p".into(),
            field: "f".into(),
        }];
        let with_secret = AppConfig {
            mcp_servers: vec![s],
            ..AppConfig::default()
        };
        assert!(any_needs_vault(&with_secret));
    }

    #[test]
    fn a_row_without_a_tunnel_expects_nothing_from_one() {
        let cfg = AppConfig::default();
        let t = tunnel_state(&cfg, &spec("x", "x.exe"));
        assert!(!t.declared);
        assert!(!t.running);
        assert!(!t.broken);
        assert!(t.detail.is_empty(), "nothing to say: {}", t.detail);
    }

    #[test]
    fn a_declared_tunnel_without_cloudflared_is_broken_not_merely_off() {
        let cfg = AppConfig {
            cloudflared_exe: r"C:\no\existe\cloudflared.exe".into(),
            ..AppConfig::default()
        };
        let mut s = spec("x", "x.exe");
        s.tunnel = "edgepass-mcp".into();
        let t = tunnel_state(&cfg, &s);
        assert!(t.declared);
        assert!(!t.running);
        assert!(t.broken, "a missing cloudflared is a fix, not a click");
        assert!(t.detail.starts_with("falta cloudflared"), "got {}", t.detail);
    }

    #[test]
    fn a_down_tunnel_keeps_the_led_off_even_with_every_server_up() {
        // The chain is only as reachable as its weakest link: a green LED over
        // an origin nobody can reach is the lie this probe exists to avoid.
        let exe = std::env::current_exe().unwrap();
        let mut s = spec("reasonixmcp", &exe.to_string_lossy());
        s.tunnel = "edgepass-mcp".into();
        let cfg = AppConfig {
            mcp_servers: validate(vec![s]).unwrap(),
            cloudflared_exe: r"C:\no\existe\cloudflared.exe".into(),
            ..AppConfig::default()
        };
        let status = status_now(&cfg);
        assert_eq!(status.tunnels, 1);
        assert_eq!(status.tunnels_up, 0);
        assert!(!status.all_running());
        assert!(status.broken);
        assert!(!status.servers[0].tunnel_running);
        assert!(status.detail.contains("túnel"), "got {}", status.detail);
    }

    #[test]
    fn the_tunnel_name_is_trimmed_on_save() {
        let mut s = spec("x", "x.exe");
        s.tunnel = "  edgepass-mcp  ".into();
        let out = validate(vec![s]).unwrap();
        assert_eq!(out[0].tunnel, "edgepass-mcp");
    }

    #[test]
    fn the_connector_has_its_own_key_so_a_stop_never_mixes_them() {
        let s = spec("reasonixmcp", "r.exe");
        assert_eq!(tunnel_key(&s), "reasonixmcp#tunnel");
        assert_ne!(tunnel_key(&s), s.id);
    }

    #[test]
    fn a_child_that_died_says_so_and_points_at_its_log() {
        // "parado" is a click; "murió (código 3) · log: …" is the diagnosis the
        // founder had to guess last time the server disappeared.
        let key = "test-died#exit";
        record_exit(key, 3);
        let note = exit_note(key, "mcp-test-died.log");
        assert!(note.contains("código 3"), "got {note}");
        assert!(note.contains("mcp-test-died.log"), "got {note}");
        forget_exit(key);
        assert!(exit_note(key, "mcp-test-died.log").is_empty());
    }

    #[test]
    fn a_dead_server_puts_its_exit_code_in_the_status_line() {
        // What the LED shows has to name the death, not just count it: an
        // "1 parado" is a click, a "murió (código 7)" is a diagnosis.
        let cfg = AppConfig {
            mcp_servers: validate(vec![spec("colgado", "nada-que-ver.exe")]).unwrap(),
            ..AppConfig::default()
        };
        record_exit("colgado", 7);
        let status = status_now(&cfg);
        forget_exit("colgado");
        assert!(!status.all_running());
        assert!(status.detail.contains("murió"), "got {}", status.detail);
        assert!(status.detail.contains("código 7"), "got {}", status.detail);
    }

    #[test]
    fn each_child_gets_its_own_log_name() {
        assert_eq!(server_log_name(&spec("reasonixmcp", "r.exe")), "mcp-reasonixmcp.log");
        assert_eq!(
            tunnel_log_name(&spec("reasonixmcp", "r.exe")),
            "cloudflared-reasonixmcp.log"
        );
    }
}
