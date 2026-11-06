//! Loopback-only IPC for external tools (scripts, agents, CI).
//!
//! Windows named pipe `\\.\pipe\edgepass`: one JSON object per line in, one out.
//!
//! Ops:
//! - `ping` / `focus` / `status` / `unseal` / `seal` / `hold`
//! - `qring` — read or write whether Q-ring may RUN: `{"op":"qring"}` reads,
//!   `{"op":"qring","enabled":false}` writes. Policy, not process control: the Q-ring pack
//!   reads this before every expensive path and refuses to fire a ring (and to spend) while
//!   it is false. `status` also carries `qring: {"enabled": bool}`. One coupling in the
//!   other direction: stopping the gateway (`{"op":"toggle","id":"qr","dir":"off"}`) writes
//!   `enabled:false` too. Writing `false` here does *not* stop the process — that is the
//!   whole point of the flag.
//! - `gha` — the local GitHub-Actions runner: `{"op":"gha"}` reads the state (Docker
//!   Desktop, every declared runner, and the job in flight) and
//!   `{"op":"gha-logs","id":"…","tail":200,"source":"both"}` returns the log text, so an
//!   agent can explain a failure it never watched.
//! - `search` — query the local index, key *names* only (see below)
//! - `meter` — open the Meter tab with every field of one secret (resolved
//!   explicitly or from a query), so the *human* can read and copy the values in
//!   the widget. The reply carries names only; values never cross the pipe.
//! - `sync_index` — rebuild that index now (the `↻` button, without the UI)
//! - `run` — inject secrets into a child process's environment (sync or `detach`)
//! - `run-status` / `run-wait` / `run-cancel` — observe or stop a detached run
//! - `request` / `request-wait` / `request-status` / `request-resolve` —
//!   GW approval for a `run` (names only; values never in the reply)
//! - `ingest-form` — open the UI table for path + key names (LLM push, no values)
//! - `toggles` — the header LEDs as data: `{"op":"toggles"}` reads them (with the
//!   vocabulary in `catalog`), `{"op":"toggles","set":[…]}` replaces the list
//!   (validated: ids from the catalog, 5 max), `{"op":"toggle","id":"kv","dir":"on|off|alt|flip"}`
//!   fires one. `status` also carries `toggles`.
//!
//! ## Threat model, honestly
//!
//! Anything running as *this Windows user* can open the pipe: a rogue npm
//! postinstall, an editor extension, an agent gone sideways. The old protocol
//! made that fatal, because `export-env` would write decrypted secrets to any
//! path the caller named. That op is gone; `run` only ever hands values to a
//! child process's environment, and the reply carries key *names* only.
//! Sealing/unsealing through the pipe is a DoS at worst, not a disclosure.
//!
//! `meter` keeps that boundary instead of widening it: values never travel over
//! the pipe. The op resolves one path and hands it to the widget, which reads
//! the fields with its own token and shows them to the human in the Meter tab —
//! so whoever called `meter` still has no value to paste anywhere.
//!
//! The pipe is created with a `SECURITY_ATTRIBUTES` whose DACL holds a single
//! ACE — full access for this user's SID and nothing else (see
//! [`crate::pipe_acl`]). If that descriptor cannot be built, the pipe is
//! created the old way and the log says so, rather than dropping the IPC
//! surface entirely.
//!
//! What is still taken on trust: an agent running *as this same user* is
//! inside the trust boundary and always was. The DACL narrows the pipe to that
//! boundary; it does not police what is already in it.

use crate::config::{get_token, load_config};
use crate::openbao::read_secret_map;
use crate::store::SearchHit;
use crate::unseal::{seal, seal_status, unseal};
use crate::{ensure_ready, now_secs, sync_index_inner, AppState, IngestFormDto};
use parking_lot::Mutex as ParkMutex;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

const PIPE_NAME: &str = r"\\.\pipe\edgepass";
const DEFAULT_HOLD_SECS: u64 = 3600;
const MAX_HOLD_SECS: u64 = 86_400;
/// Cap on captured child output returned over the pipe (sync `run`).
const MAX_OUTPUT_BYTES: usize = 64 * 1024;
/// Cap for optional `includeOutput` on detached status/wait.
const MAX_DETACH_OUTPUT_BYTES: usize = 2 * 1024;
/// `run` waits this long before killing the child (overridable per request).
const DEFAULT_RUN_TIMEOUT_SECS: u64 = 300;
const MAX_RUN_TIMEOUT_SECS: u64 = 3_600;
const MAX_RUN_WAIT_SECS: u64 = 5;
const MAX_DETACHED_SLOTS: usize = 4;
/// Cap on how many index rows a `search` may return.
const MAX_SEARCH_LIMIT: u64 = 200;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

static RUN_SEQ: AtomicU64 = AtomicU64::new(1);

fn touch(state: &AppState) {
    state.last_activity.store(now_secs(), Ordering::Relaxed);
}

/// In-memory registry of detached `run` children. Lost on widget exit (no autoresume).
#[derive(Default)]
pub struct DetachedRuns {
    slots: ParkMutex<HashMap<String, DetachedSlot>>,
}

#[derive(Clone)]
struct DetachedSlot {
    pid: u32,
    cmd0: String,
    env_names: Vec<String>,
    timeout_secs: u64,
    cancel: Arc<AtomicBool>,
    outcome: Arc<ParkMutex<DetachedOutcome>>,
}

#[derive(Clone)]
enum DetachedOutcome {
    Running,
    Done {
        code: Option<i32>,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
    },
    Failed {
        error: String,
    },
    Timeout {
        stdout: Vec<u8>,
        stderr: Vec<u8>,
    },
    Canceled {
        stdout: Vec<u8>,
        stderr: Vec<u8>,
    },
}

impl DetachedRuns {
    pub fn new() -> Self {
        Self::default()
    }

    fn running_count(&self) -> usize {
        self.slots
            .lock()
            .values()
            .filter(|s| matches!(*s.outcome.lock(), DetachedOutcome::Running))
            .count()
    }

    fn insert(&self, id: String, slot: DetachedSlot) {
        self.slots.lock().insert(id, slot);
    }

    fn get(&self, id: &str) -> Option<DetachedSlot> {
        self.slots.lock().get(id).cloned()
    }

    fn prune_finished_over_cap(&self) {
        // Keep history small: drop oldest finished when map grows past 32.
        let mut map = self.slots.lock();
        if map.len() <= 32 {
            return;
        }
        let finished: Vec<String> = map
            .iter()
            .filter(|(_, s)| !matches!(*s.outcome.lock(), DetachedOutcome::Running))
            .map(|(k, _)| k.clone())
            .collect();
        for id in finished.into_iter().take(map.len().saturating_sub(24)) {
            map.remove(&id);
        }
    }

    /// Snapshot for `status` / diagnostics — runId + state + exit, never values.
    pub fn snapshot(&self) -> Vec<Value> {
        let mut rows: Vec<(String, Value)> = self
            .slots
            .lock()
            .iter()
            .map(|(id, s)| {
                let (state, code) = match &*s.outcome.lock() {
                    DetachedOutcome::Running => ("running", Value::Null),
                    DetachedOutcome::Done { code, .. } => (
                        "done",
                        code.map(Value::from).unwrap_or(Value::Null),
                    ),
                    DetachedOutcome::Failed { .. } => ("failed", Value::Null),
                    DetachedOutcome::Timeout { .. } => ("timeout", Value::Null),
                    DetachedOutcome::Canceled { .. } => ("canceled", Value::Null),
                };
                (
                    id.clone(),
                    json!({
                        "runId": id,
                        "state": state,
                        "code": code,
                        "cmd": s.cmd0,
                        "pid": s.pid,
                        "env": s.env_names,
                        "timeoutSecs": s.timeout_secs,
                    }),
                )
            })
            .collect();
        rows.sort_by(|a, b| a.0.cmp(&b.0));
        rows.into_iter().map(|(_, v)| v).collect()
    }
}

fn next_run_id() -> String {
    let n = RUN_SEQ.fetch_add(1, Ordering::Relaxed);
    format!("r{}-{}", now_secs(), n)
}

/// True when another edgePass already owns the pipe.
///
/// Called *before* Tauri boots. A second launch should surface the widget that
/// is already running: two widgets means two guardians, two SQLite handles and
/// two processes racing over the vault's seal state.
pub fn notify_existing() -> bool {
    use std::io::Write;
    // On Windows a named pipe opens like a file. If the name exists, a server
    // is alive and we ask it to take focus; if not, this fails immediately.
    let mut pipe = match std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(PIPE_NAME)
    {
        Ok(p) => p,
        Err(_) => return false,
    };
    let _ = pipe.write_all(b"{\"op\":\"focus\"}\n");
    let _ = pipe.flush();
    true
}

/// Create one unconnected pipe instance.
///
/// `first` asks the OS to refuse if some other process already owns the name —
/// that is what stops a squatter from impersonating edgePass to scripts.
/// `security` is the DACL described in [`crate::pipe_acl`]; when it is `None`
/// the pipe is created with the logon session's default DACL, which is the
/// pre-hardening behaviour.
#[cfg(windows)]
fn create_pipe_instance(
    name: &str,
    first: bool,
    security: Option<&mut crate::pipe_acl::UserOnly>,
) -> std::io::Result<tokio::net::windows::named_pipe::NamedPipeServer> {
    use tokio::net::windows::named_pipe::ServerOptions;
    let mut opts = ServerOptions::new();
    opts.first_pipe_instance(first);
    match security {
        // SAFETY: `as_ptr` hands back the `SECURITY_ATTRIBUTES` owned by
        // `security`, which is borrowed for this whole call.
        Some(s) => unsafe { opts.create_with_security_attributes_raw(name, s.as_ptr()) },
        None => opts.create(name),
    }
}

#[cfg(windows)]
pub fn spawn(state: Arc<AppState>, app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        use crate::pipe_acl::UserOnly;
        loop {
            // The descriptor is built and consumed inside this block, on
            // purpose: it owns raw pointers, so holding it across the `await`
            // below would make this future non-`Send` and `spawn` would
            // refuse it. A fresh one per instance, too — a client is checked
            // against the DACL of the instance it lands on, not the first one.
            // `UserOnly::new` logs the reason itself when it cannot build one.
            let created = {
                let mut security = UserOnly::new();
                match create_pipe_instance(PIPE_NAME, true, security.as_mut()) {
                    Ok(s) => Ok(s),
                    Err(e) => {
                        crate::log_warn!(
                            "ipc: no pude ser el dueño del pipe ({e}); sigo sin exclusividad"
                        );
                        create_pipe_instance(PIPE_NAME, false, security.as_mut())
                    }
                }
            };
            let mut server = match created {
                Ok(s) => s,
                Err(e) => {
                    crate::log_error!("ipc: no pude crear el pipe: {e}");
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    continue;
                }
            };
            if server.connect().await.is_err() {
                continue;
            }
            let st = state.clone();
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = session(&mut server, st, app).await {
                    crate::log_warn!("ipc: sesión terminada: {e}");
                }
            });
        }
    });
}

#[cfg(not(windows))]
pub fn spawn(_state: Arc<AppState>, _app: AppHandle) {}

#[cfg(windows)]
async fn session(
    server: &mut tokio::net::windows::named_pipe::NamedPipeServer,
    state: Arc<AppState>,
    app: AppHandle,
) -> std::io::Result<()> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let (read_half, mut write_half) = tokio::io::split(server);
    let mut lines = BufReader::new(read_half).lines();
    while let Some(line) = lines.next_line().await? {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(line) {
            Ok(v) => {
                let op = v
                    .get("op")
                    .and_then(Value::as_str)
                    .unwrap_or("(sin op)")
                    .to_string();
                let reply = dispatch(v, state.clone(), &app).await;
                let ok = reply.get("ok").and_then(Value::as_bool).unwrap_or(false);
                crate::log_info!("ipc: op={op} ok={ok}");
                reply
            }
            Err(e) => json!({ "ok": false, "error": format!("json invalido: {e}") }),
        };
        let mut payload = reply.to_string();
        payload.push('\n');
        write_half.write_all(payload.as_bytes()).await?;
        write_half.flush().await?;
    }
    Ok(())
}

async fn dispatch(req: Value, state: Arc<AppState>, app: &AppHandle) -> Value {
    let op = req.get("op").and_then(Value::as_str).unwrap_or_default();
    match op {
        "ping" => json!({ "ok": true, "pong": true }),
        "focus" => focus(app),
        // Governance: may Q-ring RUN. Read (no `enabled`) or set (`"enabled": bool`).
        // Separate from the on/off quick action, which only starts/stops the gateway process.
        "qring" => match load_config() {
            Ok(mut cfg) => match req.get("enabled").and_then(Value::as_bool) {
                Some(want) => {
                    cfg.qring_governed = want;
                    match crate::config::save_config(&cfg) {
                        Ok(()) => json!({ "ok": true, "enabled": want, "source": "edgepass:config" }),
                        Err(e) => json!({ "ok": false, "error": e }),
                    }
                }
                None => json!({ "ok": true, "enabled": cfg.qring_governed, "source": "edgepass:config" }),
            },
            Err(e) => json!({ "ok": false, "error": e }),
        },
        // El runner local de GHA: leer su estado (`{"op":"gha"}`) o su log
        // (`{"op":"gha-logs","id":"vetgroom","tail":200,"source":"lifecycle|worker|both"}`).
        // El log es lo que un agente recupera para explicar un fallo sin abrir el widget, y
        // lo que muestra el visor que el LED abre solo.
        "gha" => match load_config() {
            Ok(cfg) => {
                let st = crate::gha::status(&cfg);
                json!({
                    "ok": true,
                    "dockerUp": st.docker_up,
                    "desktopRunning": st.desktop_running,
                    "message": st.message,
                    "runners": st.runners,
                })
            }
            Err(e) => json!({ "ok": false, "error": e }),
        },
        "gha-logs" => match load_config() {
            Ok(cfg) => {
                let id = req.get("id").and_then(Value::as_str);
                let tail = req.get("tail").and_then(Value::as_u64).unwrap_or(200) as usize;
                let source = req
                    .get("source")
                    .and_then(Value::as_str)
                    .unwrap_or("both");
                match crate::gha::logs(&cfg, id, tail, source) {
                    Ok(text) => json!({ "ok": true, "id": id, "source": source, "text": text }),
                    Err(e) => json!({ "ok": false, "error": e }),
                }
            }
            Err(e) => json!({ "ok": false, "error": e }),
        },
        "status" => match load_config() {
            Ok(cfg) => {
                let seal = seal_status(&cfg).await;
                let hold_until = state.hold_until.load(Ordering::Relaxed);
                let last = state.last_activity.load(Ordering::Relaxed);
                json!({
                    "ok": true,
                    "sealed": seal.sealed,
                    "reachable": seal.reachable,
                    "initialized": seal.initialized,
                    "message": seal.message,
                    "autoSealIdleSecs": cfg.auto_seal_idle_secs,
                    "idleSecs": if last == 0 { 0 } else { now_secs().saturating_sub(last) },
                    "holdSecsLeft": hold_until.saturating_sub(now_secs()),
                    "runs": state.runs.snapshot(),
                    "grants": state.grants.list_public(),
                    // The header LEDs, same shape the UI's `get_status` gets: a
                    // caller can read — and with `{"op":"toggle"}` drive — them
                    // without opening the widget.
                    "toggles": crate::toggles::views(
                        &cfg,
                        &seal,
                        !state.grants.list_public().is_empty()
                    ),
                    // The pack reads this to decide whether a ring may run at all.
                    "qring": { "enabled": cfg.qring_governed },
                })
            }
            Err(e) => json!({ "ok": false, "error": e }),
        },

        // Las skills de la máquina: `{"op":"skills"}` lee (`"sync": true` re-enlaza).
        // Es la puerta para scripts y agentes; mismo motor que el LED SYNC.
        "skills" => match req.get("sync").and_then(Value::as_bool).unwrap_or(false) {
            true => match tokio::task::spawn_blocking(crate::skills::sync_now).await {
                Ok(Ok((report, status))) => json!({ "ok": true, "sync": report, "skills": status }),
                Ok(Err(e)) => json!({ "ok": false, "error": e }),
                Err(e) => json!({ "ok": false, "error": format!("skills task: {e}") }),
            },
            false => match tokio::task::spawn_blocking(crate::skills::status_now).await {
                Ok(Ok(status)) => json!({ "ok": true, "skills": status }),
                Ok(Err(e)) => json!({ "ok": false, "error": e }),
                Err(e) => json!({ "ok": false, "error": format!("skills task: {e}") }),
            },
        },

        "unseal" => match load_config() {
            Ok(cfg) => match unseal(&cfg).await {                Ok(s) => {
                    touch(&state);
                    json!({ "ok": true, "sealed": s.sealed, "message": s.message })
                }
                Err(e) => json!({ "ok": false, "error": e }),
            },
            Err(e) => json!({ "ok": false, "error": e }),
        },
        "seal" => match load_config() {
            Ok(cfg) => match get_token() {
                Ok(Some(token)) => match seal(&cfg, &token).await {
                    Ok(s) => json!({ "ok": true, "sealed": s.sealed }),
                    Err(e) => json!({ "ok": false, "error": e }),
                },
                Ok(None) => json!({ "ok": false, "error": "sin token cargado" }),
                Err(e) => json!({ "ok": false, "error": e }),
            },
            Err(e) => json!({ "ok": false, "error": e }),
        },

        "hold" => {
            let secs = req
                .get("secs")
                .and_then(Value::as_u64)
                .unwrap_or(DEFAULT_HOLD_SECS)
                .min(MAX_HOLD_SECS);
            let until = now_secs() + secs;
            state.hold_until.store(until, Ordering::Relaxed);
            touch(&state);
            json!({ "ok": true, "holdSecs": secs, "holdUntil": until })
        }
        "run" => run(req, &state).await,
        "run-status" => run_status(&req, &state),
        "run-wait" => run_wait(&req, &state).await,
        "run-cancel" => run_cancel(&req, &state),
        "search" => search(&req, &state),
        "meter" => meter_form(&req, &state, app),
        // The `↻` button, reachable from an agent. The docs told callers to use
        // `sync_index` while this dispatch answered "op desconocida": now the
        // documented flow (`search` came up empty → reindex → `search` again)
        // works without opening the UI.
        "sync_index" => match sync_index_inner(&state).await {
            Ok(count) => {
                touch(&state);
                json!({ "ok": true, "count": count })
            }
            Err(e) => json!({ "ok": false, "error": e }),
        },
        "ingest-form" => ingest_form(req, &state, app),
        // Notas del secreto: leerlas y escribirlas por el pipe es lo que permite
        // que el agente deje *por qué* pidió un valor, al lado del path.
        "notes" => notes_op(&req, &state),
        "note" => note_op(&req, &state),
        "toggles" => toggles_op(&req, &state).await,
        "toggle" => toggle_op(&req, &state).await,
        "request" => request_op(req, state, app).await,
        "request-wait" => request_wait(&req, &state).await,
        "request-status" => request_status(&req, &state),
        "request-resolve" => match apply_decision(state, app, req_id(&req), req_decision(&req)).await {
            Ok(v) => v,
            Err(e) => json!({ "ok": false, "error": e }),
        },

        other => json!({ "ok": false, "error": format!("op desconocida: {other}") }),
    }
}

/// `{"op":"notes","namespace":"","path":"personal/x","key":"GATE"}` — las notas
/// del path (o del campo), de la más nueva a la más vieja.
///
/// Son comentarios **locales**: `(namespace, path, key)` enlaza la nota con el
/// secreto, pero el valor nunca pasa por acá. OpenBao no guarda comentarios (y en
/// KV v1 ni existe la metadata), así que esta es la mitad que el vault no puede
/// tener.
fn notes_op(req: &Value, state: &AppState) -> Value {
    let namespace = req
        .get("namespace")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let path = req.get("path").and_then(Value::as_str).unwrap_or_default();
    if path.trim().is_empty() {
        return json!({ "ok": false, "error": "falta 'path'" });
    }
    let key = req.get("key").and_then(Value::as_str).map(|s| s.to_string());
    let limit = req
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(20)
        .clamp(1, 100) as usize;
    let store = state.store.lock();
    match store.notes_for(
        &namespace,
        path,
        key.as_deref().filter(|k| !k.trim().is_empty()),
        limit,
    ) {
        Ok(notes) => json!({ "ok": true, "notes": notes }),
        Err(e) => json!({ "ok": false, "error": e }),
    }
}

/// `{"op":"note","path":"personal/x","body":"pedido para el túnel","actor":"gemini"}`
/// — deja una nota escrita por el agente al lado del secreto.
fn note_op(req: &Value, state: &AppState) -> Value {
    let namespace = req
        .get("namespace")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let path = req.get("path").and_then(Value::as_str).unwrap_or_default();
    let body = req.get("body").and_then(Value::as_str).unwrap_or_default();
    if path.trim().is_empty() || body.trim().is_empty() {
        return json!({ "ok": false, "error": "faltan 'path' y/o 'body'" });
    }
    let key = req
        .get("key")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let actor = req
        .get("actor")
        .and_then(Value::as_str)
        .unwrap_or("pipe")
        .to_string();
    let store = state.store.lock();
    match store.add_note(&namespace, path, &key, body, "user", &actor) {
        Ok(id) => json!({ "ok": true, "id": id }),
        Err(e) => json!({ "ok": false, "error": e }),
    }
}

/// `{"op":"search","query":"infisi cb2","limit":20}` — the local index, by/// *name*: namespace, path and variable. Never a value.
///
/// This is the "do I already have this?" half of the agent flow, which
/// `ingest-form` (the "create it" half) lacked: an LLM can look before asking
/// the human to paste anything, and it answers without opening the vault — so
/// it neither needs the vault unsealed nor counts as activity that would keep
/// an idle auto-seal waiting.
fn search(req: &Value, state: &AppState) -> Value {
    let query = req
        .get("query")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let limit = req
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(50)
        .clamp(1, MAX_SEARCH_LIMIT) as usize;

    let hits = match state.store.lock().search(&query, limit) {
        Ok(hits) => hits,
        Err(e) => return json!({ "ok": false, "error": e }),
    };

    let items: Vec<Value> = hits
        .iter()
        .map(|h| {
            let namespace = if h.namespace.is_empty() {
                "root"
            } else {
                h.namespace.as_str()
            };
            json!({
                "namespace": namespace,
                "path": h.path,
                "key": h.key,
                "useCount": h.use_count,
                "lastUsedAt": h.last_used_at,
                // Cuántas notas tiene el path: el agente puede ver de un vistazo
                // que ese secreto ya tiene historia (y leerla con `{"op":"notes"}`).
                "notes": h.notes,
            })
        })
        .collect();

    json!({ "ok": true, "count": items.len(), "hits": items })
}

/// The path a `meter` request should open: either named outright, or resolved
/// from the same free-text query `search` understands.
#[derive(Debug)]
enum MeterAddress {
    Explicit {
        namespace: String,
        path: String,
    },
    Query {
        query: String,
        limit: usize,
    },
}

/// `IngestFormDto::fill` value that means "the widget reads this path and fills
/// the Meter with its real fields" (`meter`) instead of "the human types them"
/// (`ingest-form`).
const FILL_FROM_VAULT: &str = "vault";

/// The empty namespace is the root one, and `"."` is how the docs spell it.
fn norm_namespace(raw: &str) -> String {
    let raw = raw.trim();
    if raw == "." {
        String::new()
    } else {
        raw.to_string()
    }
}

fn display_namespace(norm: &str) -> &str {
    if norm.is_empty() {
        "root"
    } else {
        norm
    }
}

fn parse_meter_address(req: &Value) -> Result<MeterAddress, String> {
    let path = req
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .trim_matches('/')
        .to_string();

    if !path.is_empty() {
        let namespace = norm_namespace(
            req.get("namespace")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        );
        return Ok(MeterAddress::Explicit { namespace, path });
    }

    let query = req
        .get("query")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    if query.is_empty() {
        return Err("meter necesita 'path' (+ 'namespace') o bien 'query'".into());
    }
    let limit = req
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(50)
        .clamp(1, MAX_SEARCH_LIMIT) as usize;
    Ok(MeterAddress::Query { query, limit })
}

fn distinct_paths(hits: &[SearchHit]) -> usize {
    let mut seen: Vec<(&str, &str)> = Vec::new();
    for hit in hits {
        let pair = (hit.namespace.as_str(), hit.path.as_str());
        if !seen.contains(&pair) {
            seen.push(pair);
        }
    }
    seen.len()
}

/// A query may match many keys; that is fine as long as every hit belongs to the
/// *same* path (`betty` hitting the six `BETTY_*` keys of one path is exactly the
/// case this op exists for). Hits spread over several paths are told apart only
/// by a human, so they are reported back instead of guessed at.
fn path_from_hits(hits: &[SearchHit]) -> Result<(String, String), Value> {
    if hits.is_empty() {
        return Err(json!({
            "ok": false,
            "error": "sin coincidencias en el índice local (puede estar viejo: prueba sync_index)",
        }));
    }
    let namespace = hits[0].namespace.clone();
    let path = hits[0].path.clone();

    if hits
        .iter()
        .any(|hit| hit.namespace != namespace || hit.path != path)
    {
        let mut candidates: Vec<Value> = Vec::new();
        for hit in hits {
            let entry = json!({
                "namespace": display_namespace(&hit.namespace),
                "path": hit.path,
                "key": hit.key,
            });
            if !candidates.contains(&entry) {
                candidates.push(entry);
            }
        }
        return Err(json!({
            "ok": false,
            "error": format!(
                "ambiguo: {} resultados en {} paths distintos; repite con 'path'",
                hits.len(),
                distinct_paths(hits)
            ),
            "candidates": candidates,
        }));
    }

    Ok((namespace, path))
}

/// Bring the widget forward, without expanding it.
fn focus(app: &AppHandle) -> Value {
    match app.get_webview_window("main") {
        Some(win) => {
            let _ = win.show();
            let _ = win.unminimize();
            let _ = win.set_focus();
            json!({ "ok": true, "focused": true })
        }
        None => json!({ "ok": false, "error": "ventana main no encontrada" }),
    }
}

/// Hand one form to the UI: remember it (the bootstrap picks it up if the
/// webview is not listening yet, so an op that arrives before the window is up
/// is not lost) and bring the widget forward.
fn push_ingest_form(state: &AppState, app: &AppHandle, form: &IngestFormDto) {
    {
        let mut slot = state.pending_ingest.lock();
        *slot = Some(form.clone());
    }
    let _ = app.emit("ingest-form", form);
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.set_focus();
        let _ = win.unminimize();
    }
    touch(state);
}

/// Open the batch secret form in the widget.
/// `{"op":"ingest-form","namespace":"buenaidee","path":"projects/x/secrets","keys":["A","B"]}`
fn ingest_form(req: Value, state: &AppState, app: &AppHandle) -> Value {
    let namespace = req
        .get("namespace")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    let namespace = if namespace == "." {
        String::new()
    } else {
        namespace
    };

    let path = req
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .trim_matches('/')
        .to_string();
    if path.is_empty() {
        return json!({ "ok": false, "error": "falta 'path'" });
    }

    let mut keys: Vec<String> = Vec::new();
    if let Some(arr) = req.get("keys").and_then(Value::as_array) {
        for item in arr {
            if let Some(s) = item.as_str() {
                let t = s.trim();
                if !t.is_empty() {
                    keys.push(t.to_string());
                }
            } else if let Some(name) = item.get("name").and_then(Value::as_str) {
                let t = name.trim();
                if !t.is_empty() {
                    keys.push(t.to_string());
                }
            }
        }
    }
    if keys.is_empty() {
        return json!({ "ok": false, "error": "faltan 'keys' (nombres de variables)" });
    }

    let title = req
        .get("title")
        .and_then(Value::as_str)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let id = now_secs();
    let form = IngestFormDto {
        id,
        namespace: namespace.clone(),
        path: path.clone(),
        keys: keys.clone(),
        title,
        fill: None,
    };

    push_ingest_form(state, app, &form);

    json!({
        "ok": true,
        "id": id,
        "namespace": if namespace.is_empty() { Value::String("root".into()) } else { Value::String(namespace) },
        "path": path,
        "keys": keys,
    })
}

/// Open the Meter tab filled with every field of one secret.
/// `{"op":"meter","query":"betty"}` or
/// `{"op":"meter","namespace":"dokploy","path":"projects/lotly/demo-users"}`.
///
/// This is the whole "muéstrame los datos de X" flow, and the reason it opens a
/// *panel* instead of answering: the values are read by the widget with its own
/// token and shown to the human there, so they never cross the pipe and the
/// caller (an agent) has nothing to paste into a chat, a log or a commit. It is
/// the sibling of `ingest-form` — the same panel, with the rows already filled
/// from the vault instead of waiting for someone to type them.
fn meter_form(req: &Value, state: &AppState, app: &AppHandle) -> Value {
    let address = match parse_meter_address(req) {
        Ok(address) => address,
        Err(e) => return json!({ "ok": false, "error": e }),
    };

    let (namespace, path) = match address {
        MeterAddress::Explicit { namespace, path } => (namespace, path),
        MeterAddress::Query { query, limit } => {
            let hits = match state.store.lock().search(&query, limit) {
                Ok(hits) => hits,
                Err(e) => return json!({ "ok": false, "error": e }),
            };
            match path_from_hits(&hits) {
                Ok(pair) => pair,
                Err(reply) => return reply,
            }
        }
    };

    let title = req
        .get("title")
        .and_then(Value::as_str)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let id = now_secs();
    let form = IngestFormDto {
        id,
        namespace: namespace.clone(),
        path: path.clone(),
        keys: Vec::new(),
        title,
        fill: Some(FILL_FROM_VAULT.to_string()),
    };
    push_ingest_form(state, app, &form);

    // Names and the path only: the widget is about to show the values, and this
    // reply is read by an agent that must not end up holding them.
    crate::log_info!(
        "meter: abro {}/{} en el widget (valores solo en pantalla)",
        display_namespace(&namespace),
        path
    );
    json!({
        "ok": true,
        "id": id,
        "namespace": display_namespace(&namespace),
        "path": path,
    })
}

fn req_id(req: &Value) -> &str {
    req.get("id")
        .and_then(Value::as_str)
        .or_else(|| req.get("requestId").and_then(Value::as_str))
        .unwrap_or("")
}

fn req_decision(req: &Value) -> &str {
    req.get("decision")
        .and_then(Value::as_str)
        .unwrap_or("")
}

fn emit_grant(app: &AppHandle, dto: &crate::grant::GrantRequestDto) {
    let _ = app.emit("grant-request", dto);
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.set_focus();
        let _ = win.unminimize();
    }
}

fn spawn_granted_run(state: Arc<AppState>, app: AppHandle, id: String, run_req: Value) {
    tauri::async_runtime::spawn(async move {
        let reply = run(run_req, &state).await;
        let next = state.grants.finish(&id, reply);
        crate::log_info!("grant: {id} finished");
        if let Some(dto) = next {
            emit_grant(&app, &dto);
        }
    });
}

async fn request_op(req: Value, state: Arc<AppState>, app: &AppHandle) -> Value {
    let parsed = match crate::grant::parse_request(&req) {
        Ok(p) => p,
        Err(e) => return json!({ "ok": false, "error": e }),
    };
    crate::log_info!(
        "grant: request actor={} {}/{} cmd={}",
        parsed.actor,
        crate::grant::norm_ns(&parsed.namespace),
        crate::grant::norm_path(&parsed.path),
        parsed.cmd.first().map(String::as_str).unwrap_or("?")
    );
    match state.grants.submit(parsed) {
        Err(e) => json!({ "ok": false, "error": e }),
        Ok(out) => {
            if let Some(run_req) = out.run_req {
                crate::log_info!("grant: {} auto-allow (2h) → run", out.id);
                spawn_granted_run(state.clone(), app.clone(), out.id.clone(), run_req);
            } else if out.state == crate::grant::RequestState::Pending {
                emit_grant(app, &out.dto);
            }
            state
                .grants
                .public_json(&out.id)
                .unwrap_or_else(|| json!({ "ok": true, "requestId": out.id, "status": out.state.as_str() }))
        }
    }
}

async fn request_wait(req: &Value, state: &AppState) -> Value {
    let id = req_id(req);
    if id.is_empty() {
        return json!({ "ok": false, "error": "falta 'id'" });
    }
    let timeout = req
        .get("timeoutSecs")
        .and_then(Value::as_u64)
        .unwrap_or(25)
        .min(crate::grant::MAX_WAIT_SECS);
    let deadline = Instant::now() + Duration::from_secs(timeout);
    loop {
        match state.grants.public_json(id) {
            None => return json!({ "ok": false, "error": "unknown requestId" }),
            Some(snap) => {
                let status = snap.get("status").and_then(Value::as_str).unwrap_or("");
                if status != "pending" && status != "queued" && status != "running" {
                    return snap;
                }
                if Instant::now() >= deadline {
                    return snap;
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
}

fn request_status(req: &Value, state: &AppState) -> Value {
    let id = req_id(req);
    if id.is_empty() {
        return json!({ "ok": true, "grants": state.grants.list_public() });
    }
    state
        .grants
        .public_json(id)
        .unwrap_or_else(|| json!({ "ok": false, "error": "unknown requestId" }))
}

/// UI / pipe: deny | allow-once | allow-2h. Never returns secret values.
pub async fn apply_decision(
    state: Arc<AppState>,
    app: &AppHandle,
    id: &str,
    decision: &str,
) -> Result<Value, String> {
    let id = id.trim();
    if id.is_empty() {
        return Err("falta 'id'".into());
    }
    let decision = decision.trim().to_ascii_lowercase();
    match decision.as_str() {
        "deny" => {
            let next = state.grants.deny(id)?;
            crate::log_info!("grant: {id} denied");
            if let Some(dto) = next {
                emit_grant(app, &dto);
            }
            Ok(state
                .grants
                .public_json(id)
                .unwrap_or_else(|| json!({ "ok": false, "status": "denied", "requestId": id })))
        }
        "allow-once" | "allow-2h" => {
            let two_h = decision == "allow-2h";
            let run_req = state.grants.begin_allow(id, two_h)?;
            crate::log_info!(
                "grant: {id} {}",
                if two_h { "allow-2h → run" } else { "allow-once → run" }
            );
            spawn_granted_run(state.clone(), app.clone(), id.to_string(), run_req);
            Ok(state
                .grants
                .public_json(id)
                .unwrap_or_else(|| json!({ "ok": true, "status": "running", "requestId": id })))
        }
        other => Err(format!("decision desconocida: {other}")),
    }
}

/// `{"op":"run","entries":[...],"cmd":["npm","test"],"cwd":"...","keys":["A"],
///   "envMap":{"api_key":"DEEPSEEK_API_KEY"},"detach":true,"timeoutSecs":3600}`
///
/// Resolves secrets into the child's **environment** only. With `detach:true`
/// the pipe returns in <2s (`accepted`); otherwise waits for the child (sync).
async fn run(req: Value, state: &AppState) -> Value {
    let detach = req
        .get("detach")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let cmd: Vec<String> = req
        .get("cmd")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(|s| s.to_string())
                .collect()
        })
        .unwrap_or_default();
    if cmd.is_empty() {
        return json!({ "ok": false, "error": "falta 'cmd' (array: [programa, args…])" });
    }

    let entries = req
        .get("entries")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if entries.is_empty() {
        return json!({ "ok": false, "error": "faltan 'entries'" });
    }

    let allow: Vec<String> = req
        .get("keys")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();

    let env_map = parse_env_map(req.get("envMap"));

    let cwd = req
        .get("cwd")
        .and_then(Value::as_str)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    if detach && state.runs.running_count() >= MAX_DETACHED_SLOTS {
        return json!({ "ok": false, "error": "run slots full" });
    }

    // Seal check before spawn: callers need `sealed:true`, not a generic error.
    match load_config() {
        Ok(cfg) => {
            let seal = seal_status(&cfg).await;
            if seal.sealed && !cfg.auto_unseal {
                return json!({
                    "ok": false,
                    "sealed": true,
                    "error": "OpenBao sellado (auto-unseal desactivado)",
                });
            }
        }
        Err(e) => return json!({ "ok": false, "error": e }),
    }

    let (cfg, token) = match ensure_ready(state).await {
        Ok(v) => v,
        Err(e) => {
            let sealed = e.to_ascii_lowercase().contains("sellado")
                || e.to_ascii_lowercase().contains("sealed");
            return if sealed {
                json!({ "ok": false, "sealed": true, "error": e })
            } else {
                json!({ "ok": false, "error": e })
            };
        }
    };

    let mut vault_pairs: Vec<(String, String)> = Vec::new();
    for entry in entries {
        let namespace = entry
            .get("namespace")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();
        let path = entry
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();
        if path.is_empty() {
            return json!({ "ok": false, "error": "entry sin 'path'" });
        }
        match read_secret_map(&cfg, &token, &namespace, &path).await {
            Ok(map) => {
                // Nota autogenerada del acceso: quién pidió el secreto y para qué
                // comando. Es lo que el founder quiere poder leer después —
                // "¿por qué se leyó esto?"— sin que el vault tenga que guardar
                // comentarios.
                {
                    let used: Vec<String> = map
                        .keys()
                        .filter(|k| allow.is_empty() || allow.iter().any(|a| a == *k))
                        .cloned()
                        .collect();
                    let store = state.store.lock();
                    let _ = store.add_note(
                        &namespace,
                        &path,
                        "",
                        &format!(
                            "pedido para `{}`{}",
                            cmd.first().cloned().unwrap_or_default(),
                            if used.is_empty() {
                                String::new()
                            } else {
                                format!(" · {}", used.join(", "))
                            }
                        ),
                        "auto",
                        "agent",
                    );
                }
                for (k, v) in map {
                    if !allow.is_empty() && !allow.iter().any(|a| a == &k) {
                        continue;
                    }
                    vault_pairs.push((k, v));
                }
            }
            Err(e) => {
                return json!({ "ok": false, "error": format!("{namespace}/{path}: {e}") });
            }
        }
    }

    if !allow.is_empty() {
        for a in &allow {
            if !vault_pairs.iter().any(|(k, _)| k == a) {
                return json!({ "ok": false, "error": format!("key missing: {a}") });
            }
        }
    }

    let env = apply_env_map(&vault_pairs, &env_map);
    if env.is_empty() {
        return json!({ "ok": false, "error": "no hay variables que inyectar" });
    }

    let injected: Vec<String> = env.iter().map(|(k, _)| k.clone()).collect();
    let program = cmd[0].clone();
    let args = cmd[1..].to_vec();
    let timeout_secs = run_timeout_secs(&req);

    if detach {
        return spawn_detached(state, program, args, env, injected, cwd, timeout_secs);
    }

    crate::log_info!(
        "run: {} ({} vars inyectadas al entorno del hijo, timeout {}s)",
        program,
        injected.len(),
        timeout_secs
    );

    let program_for_child = program.clone();
    let spawned = tokio::task::spawn_blocking(move || {
        run_child(
            &program_for_child,
            &args,
            &env,
            cwd.as_deref(),
            Duration::from_secs(timeout_secs),
            None,
        )
    })
    .await;

    match spawned {
        Ok(Ok(out)) => {
            let stdout = clip(&out.stdout, MAX_OUTPUT_BYTES);
            let stderr = clip(&out.stderr, MAX_OUTPUT_BYTES);
            if out.timed_out {
                crate::log_warn!(
                    "run: {program} excedió {timeout_secs}s; mato el árbol de procesos"
                );
                return json!({
                    "ok": false,
                    "timeout": true,
                    "timeoutSecs": timeout_secs,
                    "error": format!("timeout tras {timeout_secs}s: proceso terminado"),
                    "code": Value::Null,
                    "env": injected,
                    "stdout": stdout,
                    "stderr": stderr,
                });
            }
            if out.canceled {
                return json!({
                    "ok": false,
                    "canceled": true,
                    "code": out.code,
                    "env": injected,
                    "stdout": stdout,
                    "stderr": stderr,
                });
            }
            json!({
                "ok": out.success,
                "code": out.code,
                "env": injected,
                "stdout": stdout,
                "stderr": stderr,
            })
        }
        Ok(Err(e)) => json!({ "ok": false, "error": e }),
        Err(e) => json!({ "ok": false, "error": format!("fallo al esperar al hijo: {e}") }),
    }
}

fn spawn_detached(
    state: &AppState,
    program: String,
    args: Vec<String>,
    env: Vec<(String, String)>,
    injected: Vec<String>,
    cwd: Option<String>,
    timeout_secs: u64,
) -> Value {
    state.runs.prune_finished_over_cap();

    let mut child = match spawn_child_process(&program, &args, &env, cwd.as_deref()) {
        Ok(c) => c,
        Err(e) => return json!({ "ok": false, "error": e }),
    };
    let pid = child.id();
    let run_id = next_run_id();
    let cancel = Arc::new(AtomicBool::new(false));
    let outcome = Arc::new(ParkMutex::new(DetachedOutcome::Running));

    let slot = DetachedSlot {
        pid,
        cmd0: program.clone(),
        env_names: injected.clone(),
        timeout_secs,
        cancel: cancel.clone(),
        outcome: outcome.clone(),
    };
    state.runs.insert(run_id.clone(), slot);

    crate::log_info!(
        "run detach: runId={run_id} cmd={} pid={pid} env=[{}] timeout={}s",
        program,
        injected.join(","),
        timeout_secs
    );

    let program_log = program.clone();
    let run_id_log = run_id.clone();
    std::thread::spawn(move || {
        let out_reader = std::thread::spawn({
            let pipe = child.stdout.take();
            move || read_pipe(pipe)
        });
        let err_reader = std::thread::spawn({
            let pipe = child.stderr.take();
            move || read_pipe(pipe)
        });

        let result = wait_child(
            &mut child,
            Duration::from_secs(timeout_secs),
            Some(cancel.as_ref()),
        );
        let stdout = out_reader.join().unwrap_or_default();
        let stderr = err_reader.join().unwrap_or_default();

        let final_out = match result {
            Ok(mut out) => {
                out.stdout = stdout;
                out.stderr = stderr;
                out
            }
            Err(e) => {
                crate::log_error!("run detach: runId={run_id_log} cmd={program_log} error (no values)");
                *outcome.lock() = DetachedOutcome::Failed { error: e };
                return;
            }
        };

        let state_label = if final_out.canceled {
            "canceled"
        } else if final_out.timed_out {
            "timeout"
        } else {
            "done"
        };
        crate::log_info!(
            "run detach: runId={run_id_log} cmd={program_log} state={state_label} exit={:?}",
            final_out.code
        );

        *outcome.lock() = if final_out.canceled {
            DetachedOutcome::Canceled {
                stdout: final_out.stdout,
                stderr: final_out.stderr,
            }
        } else if final_out.timed_out {
            DetachedOutcome::Timeout {
                stdout: final_out.stdout,
                stderr: final_out.stderr,
            }
        } else {
            DetachedOutcome::Done {
                code: final_out.code,
                stdout: final_out.stdout,
                stderr: final_out.stderr,
            }
        };
    });

    json!({
        "ok": true,
        "accepted": true,
        "detached": true,
        "runId": run_id,
        "pid": pid,
        "env": injected,
        "timeoutSecs": timeout_secs,
    })
}

fn run_status(req: &Value, state: &AppState) -> Value {
    let run_id = req
        .get("runId")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if run_id.is_empty() {
        return json!({ "ok": false, "error": "falta 'runId'" });
    }
    let include_output = req
        .get("includeOutput")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    match state.runs.get(run_id) {
        Some(slot) => detached_status_json(run_id, &slot, include_output),
        None => json!({
            "ok": true,
            "runId": run_id,
            "state": "unknown",
            "code": Value::Null,
        }),
    }
}

async fn run_wait(req: &Value, state: &AppState) -> Value {
    let run_id = req
        .get("runId")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if run_id.is_empty() {
        return json!({ "ok": false, "error": "falta 'runId'" });
    }
    let wait_secs = req
        .get("timeoutSecs")
        .and_then(Value::as_u64)
        .unwrap_or(1)
        .clamp(1, MAX_RUN_WAIT_SECS);
    let include_output = req
        .get("includeOutput")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let deadline = Instant::now() + Duration::from_secs(wait_secs);
    loop {
        match state.runs.get(&run_id) {
            None => {
                return json!({
                    "ok": true,
                    "runId": run_id,
                    "state": "unknown",
                    "code": Value::Null,
                });
            }
            Some(slot) => {
                let running = matches!(*slot.outcome.lock(), DetachedOutcome::Running);
                if !running {
                    return detached_status_json(&run_id, &slot, include_output);
                }
            }
        }
        if Instant::now() >= deadline {
            if let Some(slot) = state.runs.get(&run_id) {
                return detached_status_json(&run_id, &slot, include_output);
            }
            return json!({
                "ok": true,
                "runId": run_id,
                "state": "unknown",
                "code": Value::Null,
            });
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn run_cancel(req: &Value, state: &AppState) -> Value {
    let run_id = req
        .get("runId")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if run_id.is_empty() {
        return json!({ "ok": false, "error": "falta 'runId'" });
    }
    let Some(slot) = state.runs.get(run_id) else {
        return json!({
            "ok": true,
            "runId": run_id,
            "state": "unknown",
            "canceled": false,
        });
    };
    if !matches!(*slot.outcome.lock(), DetachedOutcome::Running) {
        return detached_status_json(run_id, &slot, false);
    }
    slot.cancel.store(true, Ordering::SeqCst);
    kill_tree_pid(slot.pid);
    crate::log_info!("run-cancel: runId={run_id} cmd={} pid={}", slot.cmd0, slot.pid);
    // Waiter thread will flip outcome to Canceled; give it a moment for status.
    json!({
        "ok": true,
        "runId": run_id,
        "canceled": true,
        "state": "canceled",
        "pid": slot.pid,
    })
}

fn detached_status_json(run_id: &str, slot: &DetachedSlot, include_output: bool) -> Value {
    let mut body = json!({
        "ok": true,
        "runId": run_id,
        "pid": slot.pid,
        "cmd": slot.cmd0,
        "env": slot.env_names,
        "timeoutSecs": slot.timeout_secs,
    });
    let obj = body.as_object_mut().unwrap();
    match &*slot.outcome.lock() {
        DetachedOutcome::Running => {
            obj.insert("state".into(), json!("running"));
            obj.insert("code".into(), Value::Null);
        }
        DetachedOutcome::Done {
            code,
            stdout,
            stderr,
        } => {
            obj.insert("state".into(), json!("done"));
            obj.insert("code".into(), json!(code));
            maybe_attach_output(obj, include_output, stdout, stderr);
        }
        DetachedOutcome::Failed { error } => {
            obj.insert("state".into(), json!("failed"));
            obj.insert("code".into(), Value::Null);
            obj.insert("error".into(), json!(error));
        }
        DetachedOutcome::Timeout { stdout, stderr } => {
            obj.insert("state".into(), json!("timeout"));
            obj.insert("code".into(), Value::Null);
            maybe_attach_output(obj, include_output, stdout, stderr);
        }
        DetachedOutcome::Canceled { stdout, stderr } => {
            obj.insert("state".into(), json!("canceled"));
            obj.insert("code".into(), Value::Null);
            maybe_attach_output(obj, include_output, stdout, stderr);
        }
    }
    body
}

fn maybe_attach_output(
    obj: &mut serde_json::Map<String, Value>,
    include: bool,
    stdout: &[u8],
    stderr: &[u8],
) {
    if !include {
        return;
    }
    let out = clip(stdout, MAX_DETACH_OUTPUT_BYTES);
    let err = clip(stderr, MAX_DETACH_OUTPUT_BYTES);
    if looks_like_secret_output(&out) || looks_like_secret_output(&err) {
        obj.insert("outputRedacted".into(), json!(true));
        return;
    }
    obj.insert("stdout".into(), json!(out));
    obj.insert("stderr".into(), json!(err));
}

/// `envMap`: vault key → destination env var name.
fn parse_env_map(v: Option<&Value>) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let Some(obj) = v.and_then(Value::as_object) else {
        return map;
    };
    for (k, val) in obj {
        if let Some(dest) = val.as_str() {
            let src = k.trim();
            let dest = dest.trim();
            if !src.is_empty() && !dest.is_empty() {
                map.insert(src.to_string(), dest.to_string());
            }
        }
    }
    map
}

fn apply_env_map(
    vault_pairs: &[(String, String)],
    env_map: &HashMap<String, String>,
) -> Vec<(String, String)> {
    vault_pairs
        .iter()
        .map(|(k, v)| {
            let dest = env_map.get(k).cloned().unwrap_or_else(|| k.clone());
            (dest, v.clone())
        })
        .collect()
}

fn looks_like_secret_output(s: &str) -> bool {
    if s.contains("BEGIN ") && s.contains("PRIVATE") {
        return true;
    }
    if s.contains("sk-") || s.contains("AKIA") || s.contains("ghp_") {
        return true;
    }
    for line in s.lines() {
        let t = line.trim();
        if let Some((k, v)) = t.split_once('=') {
            let k = k.trim();
            let v = v.trim().trim_matches('"').trim_matches('\'');
            if k.len() >= 2
                && v.len() >= 16
                && k.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            {
                return true;
            }
        }
    }
    false
}

/// Timeout for `run`, clamped: `0` is not "no timeout" but "1s", because the
/// point of the limit is that a caller can always get an answer back.
fn run_timeout_secs(req: &Value) -> u64 {
    req.get("timeoutSecs")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_RUN_TIMEOUT_SECS)
        .clamp(1, MAX_RUN_TIMEOUT_SECS)
}

/// What a finished — or killed — child produced.
#[derive(Debug)]
struct ChildRun {
    code: Option<i32>,
    success: bool,
    timed_out: bool,
    canceled: bool,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn spawn_child_process(
    program: &str,
    args: &[String],
    env: &[(String, String)],
    cwd: Option<&str>,
) -> Result<std::process::Child, String> {
    use std::process::Stdio;
    #[cfg(windows)]
    use std::os::windows::process::CommandExt;

    let resolved = resolve_program(program);
    let mut cmd = Command::new(match resolved.as_deref() {
        Some(path) => path.as_os_str(),
        None => std::ffi::OsStr::new(program),
    });
    cmd.args(args)
        .envs(env.iter().map(|(k, v)| (k.clone(), v.clone())))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    cmd.spawn()
        .map_err(|e| format!("no pude ejecutar {program}: {e}"))
}

/// Spawn `program`, feed it `env`, and give it `timeout` to finish.
///
/// Pipes are drained on their own threads: a child that fills a pipe buffer
/// would otherwise block on write while we sit in `try_wait`, and the pair
/// would deadlock. `stdin` is closed, like `Command::output()` does.
fn run_child(
    program: &str,
    args: &[String],
    env: &[(String, String)],
    cwd: Option<&str>,
    timeout: Duration,
    cancel: Option<&AtomicBool>,
) -> Result<ChildRun, String> {
    let mut child = spawn_child_process(program, args, env, cwd)?;
    let out_reader = std::thread::spawn({
        let pipe = child.stdout.take();
        move || read_pipe(pipe)
    });
    let err_reader = std::thread::spawn({
        let pipe = child.stderr.take();
        move || read_pipe(pipe)
    });

    let mut result = wait_child(&mut child, timeout, cancel)?;
    result.stdout = out_reader.join().unwrap_or_default();
    result.stderr = err_reader.join().unwrap_or_default();
    Ok(result)
}

fn wait_child(
    child: &mut std::process::Child,
    timeout: Duration,
    cancel: Option<&AtomicBool>,
) -> Result<ChildRun, String> {
    let deadline = Instant::now() + timeout;
    let mut timed_out = false;
    let mut canceled = false;
    let status = loop {
        if cancel.is_some_and(|c| c.load(Ordering::SeqCst)) {
            canceled = true;
            kill_tree(child);
            break None;
        }
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {
                if Instant::now() >= deadline {
                    timed_out = true;
                    kill_tree(child);
                    break None;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => {
                kill_tree(child);
                return Err(format!("fallo al esperar al hijo: {e}"));
            }
        }
    };

    Ok(ChildRun {
        code: status.and_then(|s| s.code()),
        success: status.map(|s| s.success()).unwrap_or(false),
        timed_out,
        canceled,
        stdout: Vec::new(),
        stderr: Vec::new(),
    })
}

fn read_pipe(pipe: Option<impl std::io::Read>) -> Vec<u8> {
    let mut buf = Vec::new();
    if let Some(mut pipe) = pipe {
        let _ = std::io::Read::read_to_end(&mut pipe, &mut buf);
    }
    buf
}

/// Where `program` actually lives, the way a shell would find it.
///
/// Windows' `CreateProcess` only appends `.exe` to an extension-less name, so
/// `npm` — which is really `npm.cmd` — comes back as "the system cannot find
/// the file specified". Since `cmd: ["npm", "run", "migrate"]` is the whole
/// point of `run`, resolve through `PATH` × `PATHEXT` first instead of making
/// every caller wrap things in `cmd /c`.
#[cfg(windows)]
fn resolve_program(program: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    let pathext = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
    resolve_program_with(program, &path, &pathext)
}

/// On Unix the OS search already handles this (there are no extensions to try),
/// so this deliberately does nothing rather than inventing semantics.
#[cfg(not(windows))]
fn resolve_program(_program: &str) -> Option<std::path::PathBuf> {
    None
}

#[cfg(windows)]
fn resolve_program_with(
    program: &str,
    path_var: &std::ffi::OsStr,
    pathext: &str,
) -> Option<std::path::PathBuf> {
    use std::path::Path;

    let bare = Path::new(program);
    if program.contains('\\') || program.contains('/') {
        let dir = bare.parent().unwrap_or_else(|| Path::new("."));
        return candidate_in(dir, bare.file_name()?.to_str()?, pathext);
    }
    std::env::split_paths(path_var)
        .filter_map(|dir| candidate_in(&dir, program, pathext))
        .next()
}

#[cfg(windows)]
fn candidate_in(dir: &std::path::Path, name: &str, pathext: &str) -> Option<std::path::PathBuf> {
    if name.is_empty() {
        return None;
    }
    // An explicit extension is respected as given: never extend `tool.exe`.
    if std::path::Path::new(name).extension().is_some() {
        let direct = dir.join(name);
        return direct.is_file().then_some(direct);
    }
    // A bare name goes through PATHEXT, in order. Deliberately *not* the
    // extension-less file first: the `npm` that sits next to `npm.cmd` in
    // the Node install is a Unix shell script, and handing that to
    // CreateProcess is worse than not finding anything.
    for ext in pathext.split(';') {
        let ext = ext.trim();
        if ext.is_empty() {
            continue;
        }
        let candidate = dir.join(format!("{name}{ext}"));
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Kill the child **and its descendants**.
///
/// `Child::kill` only kills the process we spawned, and on Windows that is
/// usually a launcher: killing `npm.cmd` leaves its `node` grandchild running,
/// still holding the pipes (so our readers would block until it exits on its
/// own) and still holding whatever it was doing. `taskkill /T` walks the tree.
fn kill_tree(child: &mut std::process::Child) {
    kill_tree_pid(child.id());
    let _ = child.wait();
}

fn kill_tree_pid(pid: u32) {
    #[cfg(windows)]
    {
        let pid_s = pid.to_string();
        let killed = crate::winservice::run_hidden("taskkill.exe", &["/PID", &pid_s, "/T", "/F"])
            .map(|out| out.status.success())
            .unwrap_or(false);
        if killed {
            return;
        }
    }
    // Best-effort fallback when taskkill is unavailable (or non-Windows).
    #[cfg(unix)]
    {
        let _ = Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .status();
    }
    let _ = pid;
}

fn clip(bytes: &[u8], max: usize) -> String {
    let text = String::from_utf8_lossy(bytes);
    if text.len() <= max {
        return text.to_string();
    }
    let mut cut = max;
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}…[recortado]", &text[..cut])
}

/// `{"op":"toggles"}` — the header LEDs as data, plus the vocabulary that makes
/// them editable from outside. `{"op":"toggles","set":[…]}` replaces the whole
/// list; anything that does not validate comes back as an error *with* the
/// catalog, so a caller (usually an agent that guessed an id) can fix itself in
/// one round trip instead of reading the source.
async fn toggles_op(req: &Value, state: &AppState) -> Value {
    let cfg = match load_config() {
        Ok(c) => c,
        Err(e) => return json!({ "ok": false, "error": e }),
    };
    let Some(raw) = req.get("set") else {
        return toggles_reply(&cfg, state).await;
    };
    let specs: Vec<crate::toggles::ToggleSpec> = match serde_json::from_value(raw.clone()) {
        Ok(s) => s,
        Err(e) => {
            return json!({
                "ok": false,
                "error": format!("toggles inválidos: {e}"),
                "catalog": crate::toggles::catalog(),
            })
        }
    };
    let specs = match crate::toggles::validate(specs) {
        Ok(s) => s,
        Err(e) => {
            return json!({
                "ok": false,
                "error": e,
                "catalog": crate::toggles::catalog(),
            })
        }
    };
    let mut next = cfg.clone();
    next.toggles = specs;
    // `save_config`, not the `save_settings` path: the pipe replaces one field and
    // must not touch the rest of the file.
    if let Err(e) = crate::config::save_config(&next) {
        return json!({ "ok": false, "error": e });
    }
    crate::log_info!(
        "ipc: toggles set -> [{}]",
        next.toggles
            .iter()
            .map(|t| t.id.as_str())
            .collect::<Vec<_>>()
            .join(",")
    );
    toggles_reply(&next, state).await
}

/// Read shape: what the LEDs are and what they see. The catalog always rides
/// along, so `{"op":"toggles"}` doubles as the discovery call.
async fn toggles_reply(cfg: &crate::config::AppConfig, state: &AppState) -> Value {
    let seal = seal_status(cfg).await;
    json!({
        "ok": true,
        "toggles": crate::toggles::views(cfg, &seal, !state.grants.list_public().is_empty()),
        "catalog": crate::toggles::catalog(),
        "max": crate::toggles::MAX_TOGGLES,
    })
}

/// `{"op":"toggle","id":"kv","dir":"on|off|alt|flip"}` — a click, from a script.
/// Which action that means is read from the config, never taken from the caller.
async fn toggle_op(req: &Value, state: &AppState) -> Value {
    let cfg = match load_config() {
        Ok(c) => c,
        Err(e) => return json!({ "ok": false, "error": e }),
    };
    let id = req
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    if id.is_empty() {
        return json!({
            "ok": false,
            "error": "falta id (qué LED disparar)",
            "ids": cfg.toggles.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
        });
    }
    let dir = req.get("dir").and_then(Value::as_str).unwrap_or("flip");
    match crate::toggles::fire(&cfg, id, dir, state).await {
        Ok(out) => json!({
            "ok": true,
            "id": id,
            "action": out.action,
            "detail": out.detail,
            "toast": out.toast,
        }),
        Err(e) => json!({ "ok": false, "error": e }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bare_state() -> crate::AppState {
        crate::AppState {
            store: parking_lot::Mutex::new(
                crate::store::Store::open(std::path::Path::new(":memory:"))
                    .expect("in-memory index"),
            ),
            last_activity: std::sync::atomic::AtomicU64::new(0),
            hold_until: std::sync::atomic::AtomicU64::new(0),
            pending_ingest: parking_lot::Mutex::new(None),
            runs: DetachedRuns::new(),
            grants: crate::grant::GrantBoard::new(),
            gha_announced: parking_lot::Mutex::new(std::collections::HashMap::new()),
        }
    }

    /// An agent that forgets `id` must get the list of LEDs back, not a silent
    /// no-op — and nothing may be fired or written on the way.
    #[tokio::test]
    async fn toggle_op_without_an_id_answers_with_the_leds() {
        let state = bare_state();
        let reply = toggle_op(&json!({ "op": "toggle" }), &state).await;
        assert_eq!(reply["ok"], json!(false));
        assert!(reply["error"].as_str().unwrap().contains("id"));
        assert!(reply["ids"].is_array());
    }

    /// A read of `toggles` never touches the config file and always carries the
    /// vocabulary the caller needs to write one.
    #[tokio::test]
    async fn toggles_read_publishes_the_catalogue() {
        let state = bare_state();
        let reply = toggles_op(&json!({ "op": "toggles" }), &state).await;
        assert_eq!(reply["ok"], json!(true));
        assert_eq!(reply["max"], json!(crate::toggles::MAX_TOGGLES));
        let probes = reply["catalog"]["probes"].as_array().expect("probes");
        assert!(probes.iter().any(|p| p["id"] == json!("vault.seal")));
        for probe in probes {
            // Every probe says what a new toggle on it should do, so the editor
            // (and an agent) never has to combine two tables by hand.
            assert!(probe["on"].is_string(), "{probe}");
            assert!(probe["off"].is_string(), "{probe}");
        }
        assert!(reply["toggles"].is_array());
    }

    /// A `set` with an id nobody implements is refused *and* handed the catalog.
    #[tokio::test]
    async fn toggles_set_refuses_unknown_ids_with_the_catalogue() {
        let state = bare_state();
        let reply = toggles_op(
            &json!({ "op": "toggles", "set": [
                { "id": "x", "label": "X", "probe": "vault.mood", "on": "vault.unseal", "off": "vault.seal" }
            ]}),
            &state,
        )
        .await;
        assert_eq!(reply["ok"], json!(false));
        assert!(reply["error"].as_str().unwrap().contains("vault.seal"));
        assert!(reply["catalog"]["probes"].is_array());
    }

    /// …and one that only breaks the cap is refused too, without writing.
    #[tokio::test]
    async fn toggles_set_enforces_the_cap() {
        let state = bare_state();
        let one = json!({ "id": "kv", "label": "KV", "probe": "vault.seal", "on": "vault.unseal", "off": "vault.seal" });
        let six: Vec<Value> = (0..crate::toggles::MAX_TOGGLES + 1).map(|_| one.clone()).collect();
        let reply = toggles_op(&json!({ "op": "toggles", "set": six }), &state).await;
        assert_eq!(reply["ok"], json!(false));
        assert!(reply["error"].as_str().unwrap().contains("máximo"));
    }

    #[test]
    fn long_output_is_clipped_on_a_char_boundary() {
        assert_eq!(clip(b"hola", MAX_OUTPUT_BYTES), "hola");
        let big = vec![b'a'; MAX_OUTPUT_BYTES + 64];
        let out = clip(&big, MAX_OUTPUT_BYTES);
        assert!(out.ends_with("…[recortado]"));
        assert!(out.len() <= MAX_OUTPUT_BYTES + "…[recortado]".len());
    }

    #[test]
    fn run_timeout_is_clamped_and_defaulted() {
        assert_eq!(run_timeout_secs(&json!({})), DEFAULT_RUN_TIMEOUT_SECS);
        assert_eq!(run_timeout_secs(&json!({ "timeoutSecs": 45 })), 45);
        assert_eq!(run_timeout_secs(&json!({ "timeoutSecs": 0 })), 1);
        assert_eq!(
            run_timeout_secs(&json!({ "timeoutSecs": 99_999 })),
            MAX_RUN_TIMEOUT_SECS
        );
        // A string is not a number: fall back to the default, don't guess.
        assert_eq!(
            run_timeout_secs(&json!({ "timeoutSecs": "60" })),
            DEFAULT_RUN_TIMEOUT_SECS
        );
    }

    #[test]
    fn search_says_names_never_values() {
        // The index carries (namespace, path, key) and usage, and that is all a
        // `search` may ever answer with.
        let state = crate::AppState {
            store: parking_lot::Mutex::new(
                crate::store::Store::open(std::path::Path::new(":memory:"))
                    .expect("in-memory index"),
            ),
            last_activity: std::sync::atomic::AtomicU64::new(0),
            hold_until: std::sync::atomic::AtomicU64::new(0),
            pending_ingest: parking_lot::Mutex::new(None),
            runs: DetachedRuns::new(),
            grants: crate::grant::GrantBoard::new(),
            gha_announced: parking_lot::Mutex::new(std::collections::HashMap::new()),
        };
        {
            let store = state.store.lock();
            store
                .replace_index(&[crate::openbao::SecretRef {
                    namespace: "buenaidee".into(),
                    path: "projects/cb2/infisical".into(),
                    key: "DATABASE_URL".into(),
                }])
                .expect("index one secret");
        }

        let reply = search(&json!({ "query": "infisi" }), &state);
        assert_eq!(reply["ok"], json!(true));
        assert_eq!(reply["count"], json!(1));
        let hit = &reply["hits"][0];
        assert_eq!(hit["key"], json!("DATABASE_URL"));
        assert_eq!(hit["namespace"], json!("buenaidee"));
        // No value field, and nothing value-shaped: the whole payload is the
        // index columns plus the note counter (a number, never text).
        assert_eq!(
            hit.as_object().map(|o| o.len()),
            Some(6),
            "hits must carry exactly namespace/path/key/useCount/lastUsedAt/notes"
        );
        assert_eq!(hit["useCount"], json!(0));
        assert_eq!(hit["notes"], json!(0));
        assert!(hit["lastUsedAt"].is_null());
    }

    #[test]
    fn search_honours_limit_and_root_label() {
        let state = crate::AppState {
            store: parking_lot::Mutex::new(
                crate::store::Store::open(std::path::Path::new(":memory:"))
                    .expect("in-memory index"),
            ),
            last_activity: std::sync::atomic::AtomicU64::new(0),
            hold_until: std::sync::atomic::AtomicU64::new(0),
            pending_ingest: parking_lot::Mutex::new(None),
            runs: DetachedRuns::new(),
            grants: crate::grant::GrantBoard::new(),
            gha_announced: parking_lot::Mutex::new(std::collections::HashMap::new()),
        };
        {
            let store = state.store.lock();
            let items: Vec<crate::openbao::SecretRef> = (0..5)
                .map(|i| crate::openbao::SecretRef {
                    namespace: String::new(),
                    path: "database".into(),
                    key: format!("API_TOKEN_{i}"),
                })
                .collect();
            store.replace_index(&items).expect("index secrets");
        }

        let reply = search(&json!({ "query": "", "limit": 3 }), &state);
        assert_eq!(reply["count"], json!(3));
        // Empty namespace is "root" on the wire, like everywhere else in the API.
        assert_eq!(reply["hits"][0]["namespace"], json!("root"));

        // An absurd limit is clamped instead of asking SQLite for everything.
        let all = search(&json!({ "query": "", "limit": 1_000_000 }), &state);
        assert_eq!(all["count"], json!(5));
    }

    fn hit(namespace: &str, path: &str, key: &str) -> SearchHit {
        SearchHit {
            namespace: namespace.into(),
            path: path.into(),
            key: key.into(),
            use_count: 0,
            last_used_at: None,
            notes: 0,
            score: 1.0,
        }
    }

    #[test]
    fn meter_takes_a_path_or_a_query_but_not_neither() {
        // An explicit address wins over the index, and `"."` is the root
        // namespace.
        match parse_meter_address(&json!({ "path": "/projects/x/", "namespace": "." }))
            .expect("explicit address")
        {
            MeterAddress::Explicit { namespace, path } => {
                assert_eq!(namespace, "");
                assert_eq!(path, "projects/x");
            }
            other => panic!("a 'path' must address explicitly, got {other:?}"),
        }

        match parse_meter_address(&json!({ "query": " betty ", "limit": 1_000_000 }))
            .expect("query address")
        {
            MeterAddress::Query { query, limit } => {
                assert_eq!(query, "betty");
                // The same clamp `search` applies: an absurd limit is not a
                // request to dump the whole index.
                assert_eq!(limit, MAX_SEARCH_LIMIT as usize);
            }
            other => panic!("a 'query' must resolve through the index, got {other:?}"),
        }

        // Neither shape: an error, never a guess at some path in the vault.
        let err = parse_meter_address(&json!({})).expect_err("nothing to open");
        assert!(err.contains("'path'"), "unhelpful error: {err}");
        let err = parse_meter_address(&json!({ "key": "A" })).expect_err("a key is not a path");
        assert!(err.contains("'path'"), "unhelpful error: {err}");
    }

    #[test]
    fn a_query_opens_the_meter_only_when_it_names_one_path() {
        // `betty` matching several keys of the same path is exactly the case:
        // the human asked for that path, so the widget opens it whole.
        let (namespace, path) = path_from_hits(&[
            hit("dokploy", "projects/lotly/demo-users", "BETTY_EMAIL"),
            hit("dokploy", "projects/lotly/demo-users", "BETTY_PASSWORD"),
        ])
        .expect("one path");
        assert_eq!(namespace, "dokploy");
        assert_eq!(path, "projects/lotly/demo-users");

        // Nothing found: say the index may be stale rather than opening an
        // arbitrary secret.
        let err = path_from_hits(&[]).expect_err("no hits");
        assert_eq!(err["ok"], json!(false));
        assert!(
            err["error"].as_str().unwrap().contains("sync_index"),
            "the stale-index path must be spelled out: {err}"
        );

        // Two paths: only a human can pick, so report the candidates instead of
        // guessing at whichever the index happened to rank first.
        let err = path_from_hits(&[
            hit("dokploy", "projects/lotly/demo-users", "BETTY_PASSWORD"),
            hit("dokploy", "projects/lotly/api", "BETTY_PASSWORD"),
        ])
        .expect_err("ambiguous");
        assert_eq!(err["ok"], json!(false));
        assert!(err["error"].as_str().unwrap().contains("ambiguo"));
        assert_eq!(err["candidates"].as_array().map(Vec::len), Some(2));
    }

    #[test]
    fn the_form_handed_to_the_ui_never_carries_a_value() {
        // The invariant this op exists to keep: `meter` tells the UI *which
        // path* to read, and the widget does the reading. Pinning the whole
        // serialized shape is the point — a future field carrying a value would
        // have to fail here.
        let form = crate::IngestFormDto {
            id: 7,
            namespace: "dokploy".into(),
            path: "projects/lotly/demo-users".into(),
            keys: vec!["BETTY_PASSWORD".into()],
            title: None,
            fill: Some("vault".into()),
        };
        let wire = serde_json::to_value(&form).expect("serialize");
        let mut fields: Vec<String> = wire
            .as_object()
            .expect("object")
            .keys()
            .map(|k| k.to_string())
            .collect();
        fields.sort();
        assert_eq!(
            fields,
            vec![
                "fill",
                "id",
                "keys",
                "namespace",
                "path",
                "title"
            ]
        );
        assert_eq!(wire["fill"], json!("vault"));

        // `ingest-form` keeps the shape it always had: no `fill` at all when the
        // human is the one typing the values.
        let plain = crate::IngestFormDto {
            fill: None,
            ..form
        };
        assert!(serde_json::to_value(&plain)
            .expect("serialize")
            .get("fill")
            .is_none());
    }

    #[cfg(windows)]
    #[test]
    fn child_exit_code_and_stdout_are_reported() {
        let out = run_child(
            "cmd.exe",
            &["/c".into(), "echo hola & exit 7".into()],
            &[],
            None,
            Duration::from_secs(30),
            None,
        )
        .expect("spawn cmd");
        assert!(!out.timed_out);
        assert_eq!(out.code, Some(7));
        assert!(!out.success);
        assert!(String::from_utf8_lossy(&out.stdout).contains("hola"));
    }

    #[cfg(windows)]
    #[test]
    fn secrets_reach_the_child_environment() {
        let env = vec![("EDGEPASS_TEST_TOKEN".to_string(), "s3creto".to_string())];
        let out = run_child(
            "cmd.exe",
            &["/c".into(), "echo %EDGEPASS_TEST_TOKEN%".into()],
            &env,
            None,
            Duration::from_secs(30),
            None,
        )
        .expect("spawn cmd");
        assert!(out.success);
        assert!(String::from_utf8_lossy(&out.stdout).contains("s3creto"));
    }

    #[cfg(windows)]
    #[test]
    fn a_hanging_child_is_killed_at_the_deadline() {
        // `cmd /c ping` is the shape that used to hang forever *and* the shape
        // that hides a grandchild: ping inherits the pipes, so if only cmd.exe
        // were killed the readers below would block for the full 20s.
        let started = std::time::Instant::now();
        let out = run_child(
            "cmd.exe",
            &["/c".into(), "ping -n 20 127.0.0.1".into()],
            &[],
            None,
            Duration::from_millis(400),
            None,
        )
        .expect("spawn cmd");
        assert!(out.timed_out, "must report the timeout");
        assert_eq!(out.code, None);
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "must cut at the deadline, not wait out the child ({:?})",
            started.elapsed()
        );
    }

    #[cfg(windows)]
    #[test]
    fn a_shim_name_resolves_through_path_and_pathext() {
        let dir = std::env::temp_dir().join(format!("edgepass-pathext-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let shim = dir.join("edgepass-tool.cmd");
        std::fs::write(
            &shim,
            "@echo off
echo shim-ok
",
        )
        .expect("write shim");

        let path_var = std::env::join_paths([dir.as_os_str()]).expect("join paths");
        let found = resolve_program_with("edgepass-tool", &path_var, ".EXE;.CMD")
            .expect("must find the .cmd shim");
        // PATHEXT supplies the extension, so the spelling may be upper-case:
        // what matters is that it is the shim on disk.
        assert_eq!(
            std::fs::canonicalize(&found).expect("canonical resolved"),
            std::fs::canonicalize(&shim).expect("canonical shim")
        );

        // An explicit extension is respected, never extended further.
        assert!(resolve_program_with("edgepass-tool.exe", &path_var, ".EXE;.CMD").is_none());
        // Nothing anywhere: no invented path.
        assert!(resolve_program_with("edgepass-nope", &path_var, ".EXE;.CMD").is_none());

        // And the resolved shim really runs — this is the `npm` → `npm.cmd`
        // path, which a bare `CreateProcess` lookup cannot do.
        let out = run_child(
            found.to_str().expect("utf-8 temp path"),
            &[],
            &[],
            None,
            Duration::from_secs(30),
            None,
        )
        .expect("run resolved shim");
        assert!(
            out.success,
            "stderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(String::from_utf8_lossy(&out.stdout).contains("shim-ok"));

        // The bare name is not on PATH, so it still fails — with the name the
        // caller typed, not with the resolved candidate.
        let err = run_child(
            "edgepass-tool",
            &[],
            &[],
            Some(dir.to_str().expect("utf-8 temp path")),
            Duration::from_secs(10),
            None,
        )
        .expect_err("bare name must fail");
        assert!(err.contains("edgepass-tool"), "unhelpful error: {err}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(windows)]
    #[test]
    fn npm_resolves_to_its_shim_when_node_is_installed() {
        // Not an assertion about this machine having Node: an assertion that the
        // documented `cmd: ["npm", ...]` is resolvable wherever it does.
        match resolve_program("npm") {
            Some(found) => {
                let name = found.file_name().unwrap().to_string_lossy().to_lowercase();
                assert!(
                    name == "npm.cmd" || name == "npm.exe",
                    "unexpected npm: {}",
                    found.display()
                );
            }
            None => eprintln!("npm no está en PATH; se omite"),
        }
    }

    #[cfg(windows)]
    #[test]
    fn a_missing_program_is_an_error_not_a_panic() {
        let err = run_child(
            "edgepass-no-existe.exe",
            &[],
            &[],
            None,
            Duration::from_secs(5),
            None,
        )
        .expect_err("must fail");
        assert!(err.contains("edgepass-no-existe.exe"));
    }

    #[test]
    fn env_map_renames_vault_keys_to_dest_env_names() {
        let pairs = vec![
            ("api_key".into(), "secret-value".into()),
            ("OTHER".into(), "x".into()),
        ];
        let map = parse_env_map(Some(&json!({ "api_key": "DEEPSEEK_API_KEY" })));
        let env = apply_env_map(&pairs, &map);
        assert!(env.iter().any(|(k, v)| k == "DEEPSEEK_API_KEY" && v == "secret-value"));
        assert!(env.iter().any(|(k, _)| k == "OTHER"));
        assert!(!env.iter().any(|(k, _)| k == "api_key"));
    }

    #[test]
    fn secretish_stdout_is_redacted_for_include_output() {
        assert!(looks_like_secret_output("API_KEY=abcdefghijklmnopqrstuvwxyz012345"));
        assert!(looks_like_secret_output("token sk-abc123"));
        assert!(!looks_like_secret_output("build ok\n3 tests passed"));
    }

    #[cfg(windows)]
    #[test]
    fn detached_cancel_marks_slot_and_does_not_block() {
        let runs = DetachedRuns::new();
        let mut child = spawn_child_process(
            "cmd.exe",
            &["/c".into(), "ping -n 30 127.0.0.1".into()],
            &[],
            None,
        )
        .expect("spawn");
        let pid = child.id();
        let cancel = Arc::new(AtomicBool::new(false));
        let outcome = Arc::new(ParkMutex::new(DetachedOutcome::Running));
        let run_id = "r-test-cancel".to_string();
        runs.insert(
            run_id.clone(),
            DetachedSlot {
                pid,
                cmd0: "cmd.exe".into(),
                env_names: vec!["X".into()],
                timeout_secs: 60,
                cancel: cancel.clone(),
                outcome: outcome.clone(),
            },
        );

        let cancel_w = cancel.clone();
        let outcome_w = outcome.clone();
        let waiter = std::thread::spawn(move || {
            let out_r = std::thread::spawn({
                let pipe = child.stdout.take();
                move || read_pipe(pipe)
            });
            let err_r = std::thread::spawn({
                let pipe = child.stderr.take();
                move || read_pipe(pipe)
            });
            let mut result =
                wait_child(&mut child, Duration::from_secs(60), Some(cancel_w.as_ref()))
                    .expect("wait");
            result.stdout = out_r.join().unwrap_or_default();
            result.stderr = err_r.join().unwrap_or_default();
            *outcome_w.lock() = if result.canceled {
                DetachedOutcome::Canceled {
                    stdout: result.stdout,
                    stderr: result.stderr,
                }
            } else {
                DetachedOutcome::Done {
                    code: result.code,
                    stdout: result.stdout,
                    stderr: result.stderr,
                }
            };
        });

        std::thread::sleep(Duration::from_millis(100));
        cancel.store(true, Ordering::SeqCst);
        kill_tree_pid(pid);
        waiter.join().expect("waiter");
        assert!(matches!(*outcome.lock(), DetachedOutcome::Canceled { .. }));
        let snap = runs.snapshot();
        assert_eq!(snap[0]["state"], json!("canceled"));
        assert_eq!(snap[0]["runId"], json!(run_id));
        // Never leak env values into the snapshot.
        let text = snap[0].to_string();
        assert!(!text.contains("secret"));
    }

    #[test]
    fn detached_slots_cap_is_four_running() {
        let runs = DetachedRuns::new();
        for i in 0..4 {
            let cancel = Arc::new(AtomicBool::new(false));
            let outcome = Arc::new(ParkMutex::new(DetachedOutcome::Running));
            runs.insert(
                format!("r-{i}"),
                DetachedSlot {
                    pid: i as u32 + 1,
                    cmd0: "x".into(),
                    env_names: vec![],
                    timeout_secs: 1,
                    cancel,
                    outcome,
                },
            );
        }
        assert_eq!(runs.running_count(), 4);
    }

    /// The pipe as it is *actually created*: the kernel's own ACE list has this
    /// user and nobody else, and this user can still open it — both the plain
    /// `std::fs` probe a second launch uses to hand over focus, and a tokio
    /// client. Hardening that locks the widget out of its own pipe is not
    /// hardening.
    #[cfg(windows)]
    #[tokio::test]
    async fn the_pipe_is_restricted_to_one_ace_and_still_openable() {
        use crate::pipe_acl::{current_user_sid, ReadBack, UserOnly, ACCESS_ALLOWED_ACE_TYPE};
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Foundation::HANDLE;
        use windows_sys::Win32::Security::EqualSid;

        let name = format!(r"\\.\pipe\edgepass-acl-test-{}", std::process::id());
        let mut security = UserOnly::new().expect("user-only DACL");
        let server =
            create_pipe_instance(&name, true, Some(&mut security)).expect("create the pipe");

        // What the kernel stored, not what was intended.
        let dacl = unsafe { ReadBack::from_handle(server.as_raw_handle() as HANDLE) }
            .expect("read the DACL back");
        assert_eq!(dacl.aces.len(), 1, "one ACE, got {:?}", dacl.aces.len());
        let (ace_type, ace_sid) = dacl.aces[0];
        assert_eq!(ace_type, ACCESS_ALLOWED_ACE_TYPE);
        let sid = current_user_sid().expect("token SID");
        assert_ne!(
            unsafe { EqualSid(ace_sid, sid.as_ptr()) },
            0,
            "the only ACE must be this user"
        );

        // `ipc::notify_existing` opens the pipe with read+write before Tauri
        // boots; if the mask were too narrow, a second launch would silently
        // start a second widget. One instance serves one client, so each check
        // below gets an instance of its own.
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&name)
            .expect("this user must still be able to open the pipe");
        server.connect().await.expect("server accepts the client");

        // And the way scripts and agents connect.
        let mut next_security = UserOnly::new().expect("a DACL for the second instance");
        let second = create_pipe_instance(&name, false, Some(&mut next_security))
            .expect("create a second instance");
        let client = tokio::net::windows::named_pipe::ClientOptions::new()
            .open(&name)
            .expect("a client of this user must still connect");
        second
            .connect()
            .await
            .expect("second server accepts the client");
        drop(client);
    }
}
