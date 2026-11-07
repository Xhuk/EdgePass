//! Human approval for GW `request` → `run` on this node.
//!
//! Grok never receives secret values. A live 2h grant is `(actor, namespace, path)`
//! in RAM only. Status snapshots strip `values` / `stdout` / `stderr`.

use parking_lot::Mutex;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::now_secs;

pub const GRANT_TTL_SECS: u64 = 2 * 60 * 60;
pub const REQUEST_TTL_SECS: u64 = 5 * 60;
pub const MAX_WAIT_SECS: u64 = 60;
pub const MAX_QUEUE: usize = 8;

static REQ_SEQ: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestState {
    Queued,
    Pending,
    Running,
    Granted,
    Denied,
    Timeout,
}

impl RequestState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Granted => "granted",
            Self::Denied => "denied",
            Self::Timeout => "timeout",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrantRequestDto {
    pub id: String,
    pub actor: String,
    pub action: String,
    pub namespace: String,
    pub path: String,
    pub keys: Vec<String>,
    pub cmd: Vec<String>,
    pub cwd: Option<String>,
    pub reason: String,
    pub detach: bool,
    pub created_at: u64,
    pub expires_at: u64,
}

#[derive(Debug, Clone)]
pub struct ParsedRequest {
    pub actor: String,
    pub action: String,
    pub namespace: String,
    pub path: String,
    pub keys: Vec<String>,
    pub cmd: Vec<String>,
    pub cwd: Option<String>,
    pub reason: String,
    pub detach: bool,
    pub run_req: Value,
}

#[derive(Debug, Clone)]
pub struct SubmitOutcome {
    pub id: String,
    pub state: RequestState,
    pub dto: GrantRequestDto,
    pub run_req: Option<Value>,
}

struct LiveGrant {
    actor: String,
    namespace: String,
    path: String,
    until: u64,
}

struct Record {
    dto: GrantRequestDto,
    actor_norm: String,
    ns_norm: String,
    path_norm: String,
    run_req: Value,
    state: RequestState,
    run_reply: Option<Value>,
}

#[derive(Default)]
struct Inner {
    records: HashMap<String, Record>,
    order: Vec<String>,
    grants: Vec<LiveGrant>,
}

pub struct GrantBoard {
    inner: Mutex<Inner>,
}

impl Default for GrantBoard {
    fn default() -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
        }
    }
}

impl GrantBoard {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn submit(&self, parsed: ParsedRequest) -> Result<SubmitOutcome, String> {
        let now = now_secs();
        let mut inner = self.inner.lock();
        expire_locked(&mut inner, now);

        let pending_or_queued = inner
            .records
            .values()
            .filter(|r| matches!(r.state, RequestState::Pending | RequestState::Queued))
            .count();
        if pending_or_queued >= MAX_QUEUE {
            return Err("grant queue full".into());
        }

        let id = next_id(now);
        let actor_norm = norm_actor(&parsed.actor);
        let ns_norm = norm_ns(&parsed.namespace);
        let path_norm = norm_path(&parsed.path);
        if path_norm.is_empty() {
            return Err("falta 'path'".into());
        }
        if parsed.cmd.is_empty() {
            return Err("falta 'cmd' (array: [programa, args…])".into());
        }
        if parsed.action != "run" {
            return Err("action debe ser 'run'".into());
        }

        let dto = GrantRequestDto {
            id: id.clone(),
            actor: parsed.actor.clone(),
            action: parsed.action.clone(),
            namespace: display_ns(&ns_norm),
            path: path_norm.clone(),
            keys: parsed.keys.clone(),
            cmd: parsed.cmd.clone(),
            cwd: parsed.cwd.clone(),
            reason: parsed.reason.clone(),
            detach: parsed.detach,
            created_at: now,
            expires_at: now + REQUEST_TTL_SECS,
        };

        let auto = matching_grant(&inner.grants, &actor_norm, &ns_norm, &path_norm, now);
        let has_pending = inner
            .records
            .values()
            .any(|r| r.state == RequestState::Pending);

        let state = if auto {
            RequestState::Running
        } else if has_pending {
            RequestState::Queued
        } else {
            RequestState::Pending
        };

        inner.records.insert(
            id.clone(),
            Record {
                dto: dto.clone(),
                actor_norm,
                ns_norm,
                path_norm,
                run_req: parsed.run_req.clone(),
                state,
                run_reply: None,
            },
        );
        inner.order.push(id.clone());
        prune_history(&mut inner);

        Ok(SubmitOutcome {
            id,
            state,
            dto,
            run_req: if auto { Some(parsed.run_req) } else { None },
        })
    }

    pub fn peek_pending(&self) -> Option<GrantRequestDto> {
        let now = now_secs();
        let mut inner = self.inner.lock();
        expire_locked(&mut inner, now);
        inner
            .records
            .values()
            .find(|r| r.state == RequestState::Pending)
            .map(|r| r.dto.clone())
    }

    pub fn begin_allow(&self, id: &str, two_hours: bool) -> Result<Value, String> {
        let now = now_secs();
        let mut inner = self.inner.lock();
        expire_locked(&mut inner, now);
        let (run_req, actor, ns, path) = {
            let rec = inner
                .records
                .get_mut(id)
                .ok_or_else(|| "unknown requestId".to_string())?;
            if rec.state != RequestState::Pending {
                return Err(format!("request no está pendiente ({})", rec.state.as_str()));
            }
            rec.state = RequestState::Running;
            (
                rec.run_req.clone(),
                rec.actor_norm.clone(),
                rec.ns_norm.clone(),
                rec.path_norm.clone(),
            )
        };
        if two_hours {
            inner.grants.push(LiveGrant {
                actor,
                namespace: ns,
                path,
                until: now + GRANT_TTL_SECS,
            });
        }
        Ok(run_req)
    }

    pub fn deny(&self, id: &str) -> Result<Option<GrantRequestDto>, String> {
        let now = now_secs();
        let mut inner = self.inner.lock();
        expire_locked(&mut inner, now);
        let rec = inner
            .records
            .get_mut(id)
            .ok_or_else(|| "unknown requestId".to_string())?;
        if rec.state != RequestState::Pending {
            return Err(format!("request no está pendiente ({})", rec.state.as_str()));
        }
        rec.state = RequestState::Denied;
        promote_locked(&mut inner);
        Ok(inner
            .records
            .values()
            .find(|r| r.state == RequestState::Pending)
            .map(|r| r.dto.clone()))
    }

    pub fn finish(&self, id: &str, reply: Value) -> Option<GrantRequestDto> {
        let mut inner = self.inner.lock();
        if let Some(rec) = inner.records.get_mut(id) {
            if rec.state == RequestState::Running {
                rec.state = RequestState::Granted;
                rec.run_reply = Some(sanitize_run_reply(reply));
            }
        }
        promote_locked(&mut inner);
        inner
            .records
            .values()
            .find(|r| r.state == RequestState::Pending)
            .map(|r| r.dto.clone())
    }

    pub fn public_json(&self, id: &str) -> Option<Value> {
        let now = now_secs();
        let mut inner = self.inner.lock();
        expire_locked(&mut inner, now);
        inner.records.get(id).map(record_to_public)
    }

    pub fn list_public(&self) -> Vec<Value> {
        let now = now_secs();
        let mut inner = self.inner.lock();
        expire_locked(&mut inner, now);
        let mut rows: Vec<Value> = inner.order.iter().filter_map(|id| {
            inner.records.get(id).map(record_to_public)
        }).collect();
        rows.truncate(16);
        rows
    }

    pub fn has_live_grant(&self, actor: &str, namespace: &str, path: &str) -> bool {
        let now = now_secs();
        let inner = self.inner.lock();
        matching_grant(
            &inner.grants,
            &norm_actor(actor),
            &norm_ns(namespace),
            &norm_path(path),
            now,
        )
    }
}

pub fn parse_request(req: &Value) -> Result<ParsedRequest, String> {
    let action = req
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("run")
        .trim()
        .to_ascii_lowercase();
    if action != "run" {
        return Err("action debe ser 'run'".into());
    }

    let actor = {
        let raw = req
            .get("actor")
            .and_then(Value::as_str)
            .unwrap_or("grok")
            .trim();
        if raw.is_empty() {
            "grok".into()
        } else {
            raw.chars().take(40).collect()
        }
    };

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
        return Err("falta 'cmd' (array: [programa, args…])".into());
    }

    let keys: Vec<String> = req
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

    let (namespace, path, entries) = resolve_scope(req)?;
    let cwd = req
        .get("cwd")
        .and_then(Value::as_str)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let reason = req
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or("")
        .chars()
        .take(400)
        .collect::<String>()
        .trim()
        .to_string();
    let detach = req.get("detach").and_then(Value::as_bool).unwrap_or(false);

    let mut run_req = json!({
        "op": "run",
        "cmd": cmd,
        "entries": entries,
        "detach": detach,
    });
    if !keys.is_empty() {
        run_req["keys"] = json!(keys);
    }
    if let Some(ref c) = cwd {
        run_req["cwd"] = json!(c);
    }
    if let Some(secs) = req.get("timeoutSecs") {
        run_req["timeoutSecs"] = secs.clone();
    }
    if let Some(map) = req.get("envMap") {
        run_req["envMap"] = map.clone();
    }

    Ok(ParsedRequest {
        actor,
        action,
        namespace,
        path,
        keys,
        cmd,
        cwd,
        reason,
        detach,
        run_req,
    })
}

fn resolve_scope(req: &Value) -> Result<(String, String, Value), String> {
    if let Some(arr) = req.get("entries").and_then(Value::as_array) {
        if arr.is_empty() {
            return Err("entries vacío".into());
        }
        let first = &arr[0];
        let namespace = first
            .get("namespace")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let path = first
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if norm_path(&path).is_empty() {
            return Err("entry sin 'path'".into());
        }
        return Ok((namespace, path, Value::Array(arr.clone())));
    }

    let namespace = req
        .get("namespace")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let path = req
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if norm_path(&path).is_empty() {
        return Err("falta 'path'".into());
    }
    let ns_json = if norm_ns(&namespace).is_empty() {
        String::new()
    } else {
        namespace.trim().to_string()
    };
    let path_norm = norm_path(&path);
    Ok((
        namespace,
        path,
        json!([{ "namespace": ns_json, "path": path_norm }]),
    ))
}

pub fn norm_actor(raw: &str) -> String {
    raw.trim().to_ascii_lowercase()
}

pub fn norm_ns(raw: &str) -> String {
    let t = raw.trim();
    if t.is_empty() || t == "." || t.eq_ignore_ascii_case("root") {
        String::new()
    } else {
        t.to_string()
    }
}

pub fn norm_path(raw: &str) -> String {
    raw.trim().trim_matches('/').to_string()
}

fn display_ns(norm: &str) -> String {
    if norm.is_empty() {
        "root".into()
    } else {
        norm.to_string()
    }
}

fn next_id(now: u64) -> String {
    let n = REQ_SEQ.fetch_add(1, Ordering::Relaxed);
    format!("g{now}-{n}")
}

fn matching_grant(grants: &[LiveGrant], actor: &str, ns: &str, path: &str, now: u64) -> bool {
    grants.iter().any(|g| {
        g.until > now && g.actor == actor && g.namespace == ns && g.path == path
    })
}

fn expire_locked(inner: &mut Inner, now: u64) {
    inner.grants.retain(|g| g.until > now);
    let mut changed = false;
    for rec in inner.records.values_mut() {
        if rec.state == RequestState::Pending && now >= rec.dto.expires_at {
            rec.state = RequestState::Timeout;
            changed = true;
        }
    }
    if changed {
        promote_locked(inner);
    }
}

fn promote_locked(inner: &mut Inner) {
    if inner
        .records
        .values()
        .any(|r| r.state == RequestState::Pending)
    {
        return;
    }
    if let Some(id) = inner.order.iter().find(|id| {
        inner
            .records
            .get(*id)
            .is_some_and(|r| r.state == RequestState::Queued)
    }).cloned() {
        if let Some(rec) = inner.records.get_mut(&id) {
            rec.state = RequestState::Pending;
        }
    }
}

fn prune_history(inner: &mut Inner) {
    if inner.records.len() <= 32 {
        return;
    }
    let finished: Vec<String> = inner
        .order
        .iter()
        .filter(|id| {
            inner.records.get(*id).is_some_and(|r| {
                matches!(
                    r.state,
                    RequestState::Granted | RequestState::Denied | RequestState::Timeout
                )
            })
        })
        .cloned()
        .collect();
    let drop_n = inner.records.len().saturating_sub(24);
    for id in finished.into_iter().take(drop_n) {
        inner.records.remove(&id);
        inner.order.retain(|x| x != &id);
    }
}

fn sanitize_run_reply(reply: Value) -> Value {
    let mut out = reply;
    if let Some(obj) = out.as_object_mut() {
        obj.remove("values");
        obj.remove("value");
        obj.remove("secrets");
        obj.remove("stdout");
        obj.remove("stderr");
    }
    out
}

fn record_to_public(rec: &Record) -> Value {
    let mut body = json!({
        "ok": !matches!(rec.state, RequestState::Denied | RequestState::Timeout),
        "pending": rec.state == RequestState::Pending,
        "status": rec.state.as_str(),
        "requestId": rec.dto.id,
        "actor": rec.dto.actor,
        "action": rec.dto.action,
        "namespace": rec.dto.namespace,
        "path": rec.dto.path,
        "keys": rec.dto.keys,
        "cmd": rec.dto.cmd,
        "reason": rec.dto.reason,
        "detach": rec.dto.detach,
        "expiresAt": rec.dto.expires_at,
    });
    match rec.state {
        RequestState::Denied => {
            body["ok"] = json!(false);
            body["error"] = json!("denied");
        }
        RequestState::Timeout => {
            body["ok"] = json!(false);
            body["error"] = json!("timeout");
        }
        RequestState::Granted => {
            if let Some(reply) = &rec.run_reply {
                if let Some(obj) = reply.as_object() {
                    if let Some(ok) = obj.get("ok") {
                        body["ok"] = ok.clone();
                    }
                    if let Some(env) = obj.get("env") {
                        body["env"] = env.clone();
                    }
                    if let Some(code) = obj.get("code") {
                        body["code"] = code.clone();
                    }
                    if let Some(run_id) = obj.get("runId") {
                        body["runId"] = run_id.clone();
                    }
                    if let Some(accepted) = obj.get("accepted") {
                        body["accepted"] = accepted.clone();
                    }
                    if let Some(sealed) = obj.get("sealed") {
                        body["sealed"] = sealed.clone();
                    }
                    if let Some(timeout) = obj.get("timeout") {
                        body["timeout"] = timeout.clone();
                    }
                    if let Some(err) = obj.get("error") {
                        body["error"] = err.clone();
                    }
                }
            }
        }
        _ => {}
    }
    body
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(actor: &str, ns: &str, path: &str, cmd0: &str) -> ParsedRequest {
        parse_request(&json!({
            "actor": actor,
            "action": "run",
            "namespace": ns,
            "path": path,
            "cmd": [cmd0, "--version"],
            "reason": "test",
        }))
        .expect("parse")
    }

    #[test]
    fn two_hour_grant_matches_same_path_only() {
        let board = GrantBoard::new();
        let first = parsed("Grok", "integraciones", "projects/cb2/infisical", "npm");
        let a = board.submit(first).unwrap();
        assert_eq!(a.state, RequestState::Pending);
        let run = board.begin_allow(&a.id, true).unwrap();
        assert_eq!(run["op"], "run");
        board.finish(&a.id, json!({"ok":true,"code":0,"env":["DATABASE_URL"],"stdout":"SECRET=leak"}));

        let again = parsed("grok", "integraciones", "projects/cb2/infisical", "npm");
        let b = board.submit(again).unwrap();
        assert_eq!(b.state, RequestState::Running);
        assert!(b.run_req.is_some());

        let other = parsed("grok", "integraciones", "projects/other/infisical", "npm");
        let c = board.submit(other).unwrap();
        assert_eq!(c.state, RequestState::Pending);
        assert!(c.run_req.is_none());
    }

    #[test]
    fn public_snapshot_strips_values_and_stdio() {
        let board = GrantBoard::new();
        let a = board.submit(parsed("grok", ".", "projects/x", "echo")).unwrap();
        board.begin_allow(&a.id, false).unwrap();
        board.finish(
            &a.id,
            json!({
                "ok": true,
                "code": 0,
                "env": ["API_TOKEN"],
                "stdout": "super-secret",
                "stderr": "also-secret",
                "values": {"API_TOKEN": "nope"},
            }),
        );
        let snap = board.public_json(&a.id).unwrap();
        let text = snap.to_string();
        assert_eq!(snap["status"], "granted");
        assert_eq!(snap["env"], json!(["API_TOKEN"]));
        assert_eq!(snap["code"], 0);
        assert!(!text.contains("super-secret"));
        assert!(!text.contains("also-secret"));
        assert!(!text.contains("nope"));
        assert!(snap.get("values").is_none());
        assert!(snap.get("stdout").is_none());
    }

    #[test]
    fn deny_does_not_start_run() {
        let board = GrantBoard::new();
        let a = board.submit(parsed("grok", "ns", "p/a", "true")).unwrap();
        board.deny(&a.id).unwrap();
        let snap = board.public_json(&a.id).unwrap();
        assert_eq!(snap["status"], "denied");
        assert_eq!(snap["ok"], false);
        assert!(snap.get("env").is_none());
    }

    #[test]
    fn namespace_aliases_match() {
        assert_eq!(norm_ns(""), "");
        assert_eq!(norm_ns("."), "");
        assert_eq!(norm_ns("root"), "");
        assert_eq!(norm_ns("integraciones"), "integraciones");
        assert_eq!(norm_path("/projects/x/"), "projects/x");
        assert_eq!(norm_actor("Grok"), "grok");
    }

    #[test]
    fn queued_promotes_after_deny() {
        let board = GrantBoard::new();
        let a = board.submit(parsed("grok", "ns", "p/a", "true")).unwrap();
        let b = board.submit(parsed("grok", "ns", "p/b", "true")).unwrap();
        assert_eq!(a.state, RequestState::Pending);
        assert_eq!(b.state, RequestState::Queued);
        board.deny(&a.id).unwrap();
        let next = board.peek_pending().expect("promoted");
        assert_eq!(next.id, b.id);
    }

    #[test]
    fn search_parser_rejects_export_action() {
        let err = parse_request(&json!({
            "action": "export",
            "path": "x",
            "cmd": ["true"],
        }))
        .unwrap_err();
        assert!(err.contains("run"));
    }
}
