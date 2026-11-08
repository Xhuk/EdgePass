use crate::config::AppConfig;
use parking_lot::Mutex;
use reqwest::header::{HeaderMap, HeaderValue};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::OnceLock;

#[derive(Debug, thiserror::Error)]
pub enum BaoError {
    #[error("{0}")]
    Msg(String),
    #[error(transparent)]
    Http(#[from] reqwest::Error),
}

impl From<BaoError> for String {
    fn from(value: BaoError) -> Self {
        value.to_string()
    }
}

#[derive(Debug, Clone)]
pub struct SecretRef {
    pub namespace: String,
    pub path: String,
    pub key: String,
}

#[derive(Debug, Deserialize)]
struct ListBody {
    data: ListData,
}

#[derive(Debug, Deserialize)]
struct ListData {
    keys: Vec<String>,
}

/// Which shape the configured mount answers with.
///
/// KV **v2** keeps data and metadata apart (`/v1/<mount>/data/…`,
/// `/v1/<mount>/metadata/…`) and hides the fields one level deeper
/// (`{"data": {"data": {…}}}`); KV **v1** reads and writes the path itself
/// (`/v1/<mount>/…`, `{"data": {…}}`). The same mount name can be either, per
/// namespace: assuming v2 everywhere made a v1 namespace list *nothing*,
/// silently (the v2 metadata LIST answers 404 there).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KvVersion {
    V1,
    V2,
}

impl KvVersion {
    pub fn label(self) -> &'static str {
        match self {
            KvVersion::V1 => "v1",
            KvVersion::V2 => "v2",
        }
    }
}

/// What `sys/mounts` said about the configured mount.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MountLookup {
    Found(KvVersion),
    /// `sys/mounts` answered and the mount is not there (or is another engine).
    Missing,
    /// `sys/mounts` could not answer: probe the list shapes instead.
    Unavailable,
}

/// `sys/mounts` → the version of `mount` (`"kv"`), without any HTTP.
fn version_from_mounts(body: &Value, mount: &str) -> MountLookup {
    let Some(data) = body.get("data").and_then(Value::as_object) else {
        return MountLookup::Unavailable;
    };
    let Some(entry) = data.get(&format!("{}/", mount.trim_matches('/'))) else {
        return MountLookup::Missing;
    };
    match entry.get("type").and_then(Value::as_str) {
        // `generic` is the pre-0.8 name of the same KV v1 backend.
        Some("kv") | Some("generic") => {}
        _ => return MountLookup::Missing,
    }
    // The option travels as a string, but accept a number too.
    let v2 = match entry.get("options").and_then(|o| o.get("version")) {
        Some(Value::String(s)) => s.trim() == "2",
        Some(Value::Number(n)) => n.as_u64() == Some(2),
        _ => false,
    };
    MountLookup::Found(if v2 { KvVersion::V2 } else { KvVersion::V1 })
}

/// (addr, mount, namespace) → version, so changing any of them in Settings
/// re-resolves instead of reusing an answer about a different engine.
type VersionKey = (String, String, String);

fn version_cache() -> &'static Mutex<HashMap<VersionKey, KvVersion>> {
    static CACHE: OnceLock<Mutex<HashMap<VersionKey, KvVersion>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Forget every remembered version. One call per scan, so remounting or
/// upgrading the engine is picked up without restarting the widget.
pub fn forget_kv_versions() {
    version_cache().lock().clear();
}

fn version_key(cfg: &AppConfig, namespace: &str) -> VersionKey {
    (
        cfg.bao_addr.clone(),
        cfg.mount.clone(),
        namespace.to_string(),
    )
}

/// Version of the configured mount inside `namespace`, resolved once per run.
async fn kv_version(
    http: &reqwest::Client,
    cfg: &AppConfig,
    token: &str,
    namespace: &str,
) -> KvVersion {
    // Bind first: the guard must not be held across the await below.
    let cached = version_cache()
        .lock()
        .get(&version_key(cfg, namespace))
        .copied();
    if let Some(version) = cached {
        return version;
    }
    let version = detect_version(http, cfg, token, namespace).await;
    let label = if namespace.is_empty() {
        "root"
    } else {
        namespace
    };
    crate::log_info!(
        "kv: namespace {label} has mount '{}' on KV {}",
        cfg.mount,
        version.label()
    );
    version_cache()
        .lock()
        .insert(version_key(cfg, namespace), version);
    version
}

/// `sys/mounts` first — it never lies about a mount that happens to be empty
/// (both list shapes answer 404 there); probing only when the token cannot
/// read it.
async fn detect_version(
    http: &reqwest::Client,
    cfg: &AppConfig,
    token: &str,
    namespace: &str,
) -> KvVersion {
    let url = match join_url(&cfg.bao_addr, &["v1", "sys", "mounts"]) {
        Ok(url) => url,
        Err(_) => return KvVersion::V2,
    };
    if let Ok(res) = http
        .get(&url)
        .headers(headers(token, namespace).unwrap_or_default())
        .send()
        .await
    {
        if res.status().is_success() {
            if let Ok(body) = res.json::<Value>().await {
                match version_from_mounts(&body, &cfg.mount) {
                    MountLookup::Found(version) => return version,
                    // Not mounted here: no shape to guess, and v2 keeps the
                    // failure we already had (a 404 read) instead of inventing
                    // a different one.
                    MountLookup::Missing => return KvVersion::V2,
                    MountLookup::Unavailable => {}
                }
            }
        }
    }
    probe_version(http, cfg, token, namespace)
        .await
        .unwrap_or(KvVersion::V2)
}

/// Last resort: ask each list shape at the mount root. A v2 mount 404s the v1
/// URL and the other way round, so the first 2xx answers it.
async fn probe_version(
    http: &reqwest::Client,
    cfg: &AppConfig,
    token: &str,
    namespace: &str,
) -> Option<KvVersion> {
    let hdr = headers(token, namespace).ok()?;
    for version in [KvVersion::V2, KvVersion::V1] {
        let url = list_url(cfg, version, "").ok()?;
        let ok = http
            .get(&url)
            .headers(hdr.clone())
            .send()
            .await
            .map(|res| res.status().is_success())
            .unwrap_or(false);
        if ok {
            return Some(version);
        }
    }
    None
}

/// URL that LISTs one directory in the shape `version` speaks.
fn list_url(cfg: &AppConfig, version: KvVersion, relative: &str) -> Result<String, BaoError> {
    let rel = relative.trim_matches('/');
    let mut url = match version {
        KvVersion::V2 => join_url(&cfg.bao_addr, &["v1", cfg.mount.as_str(), "metadata", rel])?,
        KvVersion::V1 => {
            let mut url = join_url(&cfg.bao_addr, &["v1", cfg.mount.as_str(), rel])?;
            if rel.is_empty() {
                // Canonical form of the mount root (`/v1/kv/?list=true`).
                url.push('/');
            }
            url
        }
    };
    url.push_str("?list=true");
    Ok(url)
}

/// URL that reads (GET) or writes (POST) one secret.
fn data_url(cfg: &AppConfig, version: KvVersion, relative: &str) -> Result<String, BaoError> {
    let rel = relative.trim_matches('/');
    match version {
        KvVersion::V2 => join_url(&cfg.bao_addr, &["v1", cfg.mount.as_str(), "data", rel]),
        KvVersion::V1 => join_url(&cfg.bao_addr, &["v1", cfg.mount.as_str(), rel]),
    }
}

/// URL that destroys one path: v2 deletes the metadata, v1 the path itself.
fn destroy_url(cfg: &AppConfig, version: KvVersion, relative: &str) -> Result<String, BaoError> {
    let rel = relative.trim_matches('/');
    Ok(match version {
        KvVersion::V2 => join_url(&cfg.bao_addr, &["v1", cfg.mount.as_str(), "metadata", rel])?,
        KvVersion::V1 => join_url(&cfg.bao_addr, &["v1", cfg.mount.as_str(), rel])?,
    })
}

/// Fields of a read reply, whichever shape came back.
async fn parse_fields(
    res: reqwest::Response,
    version: KvVersion,
) -> Result<serde_json::Map<String, Value>, BaoError> {
    Ok(fields_from_json(version, res.json::<Value>().await?))
}

/// The fields inside a read reply: v2 hides them under `data.data`, v1 keeps
/// them straight under `data`.
fn fields_from_json(version: KvVersion, body: Value) -> serde_json::Map<String, Value> {
    let node = match version {
        KvVersion::V2 => body.get("data").and_then(|d| d.get("data")),
        KvVersion::V1 => body.get("data"),
    };
    node.and_then(Value::as_object).cloned().unwrap_or_default()
}

/// Body a write sends: v2 wraps the fields, v1 sends them bare.
fn write_body(version: KvVersion, fields: serde_json::Map<String, Value>) -> Value {
    let obj = Value::Object(fields);
    match version {
        KvVersion::V2 => serde_json::json!({ "data": obj }),
        KvVersion::V1 => obj,
    }
}

/// Every request goes through the loopback-only, proxy-free client in
/// [`crate::net`], which validates the address first: a hostile `baoAddr` in
/// config.json can therefore never receive the vault token.
fn client(cfg: &AppConfig) -> Result<reqwest::Client, BaoError> {
    crate::net::client(&cfg.bao_addr, 20).map_err(BaoError::Msg)
}

fn headers(token: &str, namespace: &str) -> Result<HeaderMap, BaoError> {
    let mut map = HeaderMap::new();
    map.insert(
        "X-Vault-Token",
        HeaderValue::from_str(token).map_err(|e| BaoError::Msg(e.to_string()))?,
    );
    if !namespace.is_empty() {
        map.insert(
            "X-Vault-Namespace",
            HeaderValue::from_str(namespace).map_err(|e| BaoError::Msg(e.to_string()))?,
        );
    }
    Ok(map)
}

/// Thin wrapper so call sites keep their `?` ergonomics; the shape lives in
/// [`crate::net::join_url`].
fn join_url(addr: &str, parts: &[&str]) -> Result<String, BaoError> {
    crate::net::join_url(addr, parts).map_err(BaoError::Msg)
}

async fn list_keys(
    http: &reqwest::Client,
    cfg: &AppConfig,
    token: &str,
    namespace: &str,
    relative: &str,
) -> Result<Vec<String>, BaoError> {
    let url = list_url(cfg, kv_version(http, cfg, token, namespace).await, relative)?;

    let res = http
        .get(&url)
        .headers(headers(token, namespace)?)
        .send()
        .await?;

    if res.status().as_u16() == 404 {
        return Ok(vec![]);
    }
    if !res.status().is_success() {
        let status = res.status();
        let body = res.text().await.unwrap_or_default();
        return Err(BaoError::Msg(format!(
            "list failed ({status}): {}",
            error_text(&body, 180)
        )));
    }

    let body: ListBody = res.json().await?;
    Ok(body.data.keys)
}

async fn read_secret_keys(
    http: &reqwest::Client,
    cfg: &AppConfig,
    token: &str,
    namespace: &str,
    relative: &str,
) -> Result<Vec<String>, BaoError> {
    let version = kv_version(http, cfg, token, namespace).await;
    let url = data_url(cfg, version, relative)?;

    let res = http
        .get(&url)
        .headers(headers(token, namespace)?)
        .send()
        .await?;

    if !res.status().is_success() {
        let status = res.status();
        let body = res.text().await.unwrap_or_default();
        return Err(BaoError::Msg(format!(
            "read failed ({status}): {}",
            error_text(&body, 180)
        )));
    }

    let mut keys: Vec<String> = parse_fields(res, version)
        .await?
        .into_iter()
        .map(|(key, _)| key)
        .collect();
    keys.sort();
    Ok(keys)
}

pub async fn read_secret_value(
    cfg: &AppConfig,
    token: &str,
    namespace: &str,
    path: &str,
    key: &str,
) -> Result<String, BaoError> {
    let http = client(cfg)?;
    let version = kv_version(&http, cfg, token, namespace).await;
    let url = data_url(cfg, version, path)?;

    let res = http
        .get(&url)
        .headers(headers(token, namespace)?)
        .send()
        .await?;

    if !res.status().is_success() {
        let status = res.status();
        let body = res.text().await.unwrap_or_default();
        return Err(BaoError::Msg(format!(
            "read value failed ({status}): {}",
            error_text(&body, 180)
        )));
    }

    let fields = parse_fields(res, version).await?;
    let value = fields
        .get(key)
        .ok_or_else(|| BaoError::Msg(format!("key '{key}' not found")))?;

    match value {
        Value::String(s) => Ok(s.clone()),
        other => Ok(other.to_string()),
    }
}

/// Read every field of a KV secret as `name -> value` (either engine version).
pub async fn read_secret_map(
    cfg: &AppConfig,
    token: &str,
    namespace: &str,
    path: &str,
) -> Result<std::collections::BTreeMap<String, String>, BaoError> {
    let http = client(cfg)?;
    let version = kv_version(&http, cfg, token, namespace).await;
    let url = data_url(cfg, version, path)?;

    let res = http
        .get(&url)
        .headers(headers(token, namespace)?)
        .send()
        .await?;

    if !res.status().is_success() {
        let status = res.status();
        let body = res.text().await.unwrap_or_default();
        return Err(BaoError::Msg(format!(
            "read map failed ({status}): {}",
            error_text(&body, 180)
        )));
    }

    let mut out = std::collections::BTreeMap::new();
    for (k, v) in parse_fields(res, version).await? {
        let s = match v {
            Value::String(s) => s,
            other => other.to_string(),
        };
        out.insert(k, s);
    }
    Ok(out)
}
/// Patch (merge) a single field into a KV secret. Creates the path if missing
/// via put. v1 has no partial write, so the merge is read-modify-write there too.
pub async fn upsert_secret_field(
    cfg: &AppConfig,
    token: &str,
    namespace: &str,
    path: &str,
    key: &str,
    value: &str,
    replace_all: bool,
) -> Result<(), BaoError> {
    let http = client(cfg)?;
    let rel = path.trim_matches('/');
    if rel.is_empty() {
        return Err(BaoError::Msg("path is required".into()));
    }
    if key.trim().is_empty() {
        return Err(BaoError::Msg("key is required".into()));
    }

    let version = kv_version(&http, cfg, token, namespace).await;
    let url = data_url(cfg, version, rel)?;

    let data = if replace_all {
        let mut obj = serde_json::Map::new();
        obj.insert(key.to_string(), Value::String(value.to_string()));
        obj
    } else {
        // Try merge with existing
        let existing = http
            .get(&url)
            .headers(headers(token, namespace)?)
            .send()
            .await?;
        let mut obj = if existing.status().is_success() {
            parse_fields(existing, version).await?
        } else if existing.status().as_u16() == 404 {
            serde_json::Map::new()
        } else {
            let status = existing.status();
            let body = existing.text().await.unwrap_or_default();
            return Err(BaoError::Msg(format!(
                "read before write failed ({status}): {}",
                error_text(&body, 180)
            )));
        };
        obj.insert(key.to_string(), Value::String(value.to_string()));
        obj
    };

    let payload = write_body(version, data);
    post_kv(&http, &url, token, namespace, &payload).await
}

/// `true` for the one 500 that is nobody's fault: a node that has not finished
/// becoming the active one.
///
/// Right after an unseal (or a raft election) OpenBao answers
/// `local node not active but active cluster node not found` for a moment. The
/// write is perfectly valid — the node is not listening for it *yet* — so this is
/// worth repeating instead of showing the user a red banner over a secret they
/// already typed.
fn is_not_active(status: u16, body: &str) -> bool {
    status == 500 && body.contains("not active")
}

/// Writes a KV secret, retrying the transient "not active yet" window.
async fn post_kv(
    http: &reqwest::Client,
    url: &str,
    token: &str,
    namespace: &str,
    payload: &serde_json::Value,
) -> Result<(), BaoError> {
    const ATTEMPTS: u32 = 4;
    let mut last = String::new();
    for attempt in 0..ATTEMPTS {
        let res = http
            .post(url)
            .headers(headers(token, namespace)?)
            .json(payload)
            .send()
            .await?;
        if res.status().is_success() {
            return Ok(());
        }
        let status = res.status();
        let body = res.text().await.unwrap_or_default();
        if is_not_active(status.as_u16(), &body) && attempt + 1 < ATTEMPTS {
            // 200, 400, 600 ms: enough for a local node to finish becoming
            // active, short enough that a real outage still reports itself.
            tokio::time::sleep(std::time::Duration::from_millis(
                200 * u64::from(attempt + 1),
            ))
            .await;
            last = error_text(&body, 120);
            continue;
        }
        return Err(BaoError::Msg(format!(
            "write failed ({status}): {}",
            error_text(&body, 180)
        )));
    }
    Err(BaoError::Msg(format!(
        "write failed: the OpenBao node never became active ({last})"
    )))
}

/// Merge many fields into a KV secret in one write.
pub async fn upsert_secret_map(
    cfg: &AppConfig,
    token: &str,
    namespace: &str,
    path: &str,
    fields: &[(String, String)],
    replace_all: bool,
) -> Result<(), BaoError> {
    let http = client(cfg)?;
    let rel = path.trim_matches('/');
    if rel.is_empty() {
        return Err(BaoError::Msg("path is required".into()));
    }
    if fields.is_empty() {
        return Err(BaoError::Msg("no fields to write".into()));
    }

    let version = kv_version(&http, cfg, token, namespace).await;
    let url = data_url(cfg, version, rel)?;

    let mut obj = if replace_all {
        serde_json::Map::new()
    } else {
        let existing = http
            .get(&url)
            .headers(headers(token, namespace)?)
            .send()
            .await?;
        if existing.status().is_success() {
            parse_fields(existing, version).await?
        } else if existing.status().as_u16() == 404 {
            serde_json::Map::new()
        } else {
            let status = existing.status();
            let body = existing.text().await.unwrap_or_default();
            return Err(BaoError::Msg(format!(
                "read before write failed ({status}): {}",
                error_text(&body, 180)
            )));
        }
    };

    for (k, v) in fields {
        let key = k.trim();
        if key.is_empty() {
            continue;
        }
        obj.insert(key.to_string(), Value::String(v.clone()));
    }

    let payload = write_body(version, obj);
    post_kv(&http, &url, token, namespace, &payload).await
}

/// Remove one key from a path. If nothing remains, destroy the path metadata.
pub async fn delete_secret_key(
    cfg: &AppConfig,
    token: &str,
    namespace: &str,
    path: &str,
    key: &str,
) -> Result<(), BaoError> {
    let rel = path.trim_matches('/');
    let key = key.trim();
    if rel.is_empty() || key.is_empty() {
        return Err(BaoError::Msg("path and key are required".into()));
    }

    let mut map = read_secret_map(cfg, token, namespace, rel).await?;
    if map.remove(key).is_none() {
        return Err(BaoError::Msg(format!("key '{key}' does not exist in {rel}")));
    }

    if map.is_empty() {
        return delete_secret_path(cfg, token, namespace, rel).await;
    }

    let fields: Vec<(String, String)> = map.into_iter().collect();
    upsert_secret_map(cfg, token, namespace, rel, &fields, true).await
}

/// Destroy a path: v2 kills every version through its metadata, v1 the path itself.
pub async fn delete_secret_path(
    cfg: &AppConfig,
    token: &str,
    namespace: &str,
    path: &str,
) -> Result<(), BaoError> {
    let http = client(cfg)?;
    let rel = path.trim_matches('/');
    if rel.is_empty() {
        return Err(BaoError::Msg("path is required".into()));
    }
    let version = kv_version(&http, cfg, token, namespace).await;
    let url = destroy_url(cfg, version, rel)?;
    let res = http
        .delete(&url)
        .headers(headers(token, namespace)?)
        .send()
        .await?;
    if !res.status().is_success() && res.status().as_u16() != 404 {
        let status = res.status();
        let body = res.text().await.unwrap_or_default();
        return Err(BaoError::Msg(format!(
            "delete path failed ({status}): {}",
            error_text(&body, 180)
        )));
    }
    Ok(())
}

/// Immediate child namespaces under `parent` (empty = root). Trailing `/` stripped.
async fn list_child_namespaces(
    http: &reqwest::Client,
    cfg: &AppConfig,
    token: &str,
    parent: &str,
) -> Result<Vec<String>, BaoError> {
    let mut url = join_url(&cfg.bao_addr, &["v1", "sys", "namespaces"])?;
    url.push_str("?list=true");

    let res = http
        .get(&url)
        .headers(headers(token, parent)?)
        .send()
        .await?;

    if res.status().as_u16() == 404 {
        return Ok(vec![]);
    }
    if !res.status().is_success() {
        let status = res.status();
        let body = res.text().await.unwrap_or_default();
        return Err(BaoError::Msg(format!(
            "namespaces list failed ({status}): {}",
            error_text(&body, 180)
        )));
    }

    let body: ListBody = res.json().await?;
    Ok(body
        .data
        .keys
        .into_iter()
        .map(|k| k.trim_matches('/').to_string())
        .filter(|k| !k.is_empty())
        .collect())
}

/// Depth-first discovery of every namespace path under `parent` (not including parent).
async fn discover_namespaces_under(
    http: &reqwest::Client,
    cfg: &AppConfig,
    token: &str,
    parent: &str,
) -> Result<Vec<String>, BaoError> {
    let mut out = Vec::new();
    let children = list_child_namespaces(http, cfg, token, parent).await?;
    for child in children {
        let full = if parent.is_empty() {
            child
        } else {
            format!("{}/{}", parent.trim_matches('/'), child)
        };
        out.push(full.clone());
        match Box::pin(discover_namespaces_under(http, cfg, token, &full)).await {
            Ok(nested) => out.extend(nested),
            Err(e) => {
                crate::log_warn!("sync: could not list child namespaces of {full}: {e}");
            }
        }
    }
    Ok(out)
}

/// Resolve the configured namespace list into concrete scan targets.
///
/// `.` (empty string) means **root plus every child namespace** discovered via
/// `sys/namespaces`. Explicit names are kept as-is. Order is stable: root first,
/// then configured/discovered names sorted.
pub(crate) async fn resolve_scan_namespaces(
    http: &reqwest::Client,
    cfg: &AppConfig,
    token: &str,
    configured: &[String],
) -> Vec<String> {
    let mut want_all = false;
    let mut explicit = Vec::new();
    for ns in configured {
        if ns.is_empty() {
            want_all = true;
        } else {
            explicit.push(ns.clone());
        }
    }

    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();

    if want_all || configured.is_empty() {
        if seen.insert(String::new()) {
            out.push(String::new());
        }
        match discover_namespaces_under(http, cfg, token, "").await {
            Ok(discovered) => {
                crate::log_info!(
                    "sync: `.` expands to root + {} namespace(s)",
                    discovered.len()
                );
                let mut sorted = discovered;
                sorted.sort();
                for ns in sorted {
                    if seen.insert(ns.clone()) {
                        out.push(ns);
                    }
                }
            }
            Err(e) => {
                crate::log_warn!(
                    "sync: could not discover child namespaces ({e}); indexing root only"
                );
            }
        }
    }

    for ns in explicit {
        if seen.insert(ns.clone()) {
            out.push(ns);
        }
    }

    if out.is_empty() {
        out.push(String::new());
    }
    out
}

async fn scan_namespace(
    http: &reqwest::Client,
    cfg: &AppConfig,
    token: &str,
    ns: &str,
) -> Result<Vec<SecretRef>, BaoError> {
    let mut out = Vec::new();
    let mut stack = vec![String::new()];
    while let Some(prefix) = stack.pop() {
        let keys = list_keys(http, cfg, token, ns, &prefix).await?;
        for key in keys {
            if key.ends_with('/') {
                let child = if prefix.is_empty() {
                    key.trim_end_matches('/').to_string()
                } else {
                    format!(
                        "{}/{}",
                        prefix.trim_end_matches('/'),
                        key.trim_end_matches('/')
                    )
                };
                stack.push(child);
            } else {
                let path = if prefix.is_empty() {
                    key
                } else {
                    format!("{}/{}", prefix.trim_end_matches('/'), key)
                };
                match read_secret_keys(http, cfg, token, ns, &path).await {
                    Ok(field_keys) => {
                        for field in field_keys {
                            out.push(SecretRef {
                                namespace: ns.to_string(),
                                path: path.clone(),
                                key: field,
                            });
                        }
                    }
                    Err(e) => {
                        let label = if ns.is_empty() { "root" } else { ns };
                        crate::log_warn!("sync: skipping {label}/{path}: {e}");
                    }
                }
            }
        }
    }
    Ok(out)
}

/// Walk KV metadata recursively and collect secret path + field names (never values).
///
/// When the config includes `.` (root), every OpenBao namespace under `/` is
/// discovered and scanned — not only the root KV tree.
pub async fn scan_index(cfg: &AppConfig, token: &str) -> Result<Vec<SecretRef>, BaoError> {
    let http = client(cfg)?;
    // A remount or an engine upgrade must be seen without restarting the widget.
    forget_kv_versions();
    let targets = resolve_scan_namespaces(&http, cfg, token, &cfg.namespaces).await;
    let mut out = Vec::new();

    for ns in targets {
        let label = if ns.is_empty() { "root" } else { ns.as_str() };
        match scan_namespace(&http, cfg, token, &ns).await {
            Ok(refs) => out.extend(refs),
            Err(e) => {
                // A missing mount or ACL on one namespace must not abort the rest.
                crate::log_warn!("sync: skipping namespace {label}: {e}");
            }
        }
    }

    Ok(out)
}

fn truncate(s: &str, max: usize) -> String {
    let t = s.replace('\n', " ");
    if t.len() <= max {
        t
    } else {
        format!("{}…", &t[..max])
    }
}

/// The vault's error body is JSON: `errors[]` is the message, but the envelope
/// also carries `request_id`, `warnings` and other metadata. Forward only the
/// human part, never the whole document.
fn error_text(body: &str, max: usize) -> String {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(body) {
        if let Some(errs) = v.get("errors").and_then(|e| e.as_array()) {
            let text = errs
                .iter()
                .filter_map(|e| e.as_str())
                .collect::<Vec<_>>()
                .join("; ");
            if !text.is_empty() {
                return truncate(&text, max);
            }
        }
        return "unexpected error body".to_string();
    }
    truncate(body, max)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn error_text_forwards_only_the_vault_message() {
        let body = r#"{"errors":["permission denied","on path kv/data/x"],"request_id":"3f2b","warnings":["lease soon"]}"#;
        assert_eq!(
            error_text(body, 180),
            "permission denied; on path kv/data/x",
            "request_id and warnings must not reach the UI, the log or an agent"
        );
        assert_eq!(error_text(r#"{"request_id":"3f2b"}"#, 180), "unexpected error body");
        assert_eq!(error_text("plain text", 180), "plain text");
    }

    #[test]
    fn a_not_active_node_is_retried_but_a_real_error_is_not() {
        // The only 500 that is retried: the node that has not finished
        // becoming active yet (right after an unseal or a raft election).
        assert!(is_not_active(
            500,
            r#"{"errors":["local node not active but active cluster node not found"]}"#
        ));
        // A real 500 (broken storage, permission) is reported as it is.
        assert!(!is_not_active(500, r#"{"errors":["internal error"]}"#));
        assert!(!is_not_active(403, "permission denied"));
        assert!(!is_not_active(200, "not active"));
    }

    fn cfg_with(mount: &str) -> AppConfig {
        AppConfig {
            bao_addr: "http://127.0.0.1:8200".into(),
            mount: mount.into(),
            ..AppConfig::default()
        }
    }

    /// The shape of `sys/mounts` at the root of this machine: v2 carries the
    /// option, v1 simply does not.
    fn mounts_body() -> Value {
        json!({
            "data": {
                "kv/": { "type": "kv", "options": { "version": "2" } },
                "kv1/": { "type": "kv" },
                "transit/": { "type": "transit" },
                "numeric/": { "type": "kv", "options": { "version": 2 } }
            }
        })
    }

    #[test]
    fn reads_the_mount_version_out_of_sys_mounts() {
        let body = mounts_body();
        assert_eq!(
            version_from_mounts(&body, "kv"),
            MountLookup::Found(KvVersion::V2)
        );
        assert_eq!(
            version_from_mounts(&body, "kv1"),
            MountLookup::Found(KvVersion::V1),
            "without options.version the mount is v1 - the legacy backend case",
        );
        assert_eq!(
            version_from_mounts(&body, "numeric"),
            MountLookup::Found(KvVersion::V2),
            "the number counts too"
        );
        assert_eq!(
            version_from_mounts(&body, "other"),
            MountLookup::Missing,
            "without the mount there is no way to read"
        );
        assert_eq!(
            version_from_mounts(&body, "transit"),
            MountLookup::Missing,
            "another engine under that name is not KV"
        );
        assert_eq!(
            version_from_mounts(&json!({ "errors": ["permission denied"] }), "kv"),
            MountLookup::Unavailable,
            "without data (ACL) it has to be probed"
        );
    }

    #[test]
    fn list_url_speaks_each_version() {
        let cfg = cfg_with("kv");
        assert_eq!(
            list_url(&cfg, KvVersion::V2, "").unwrap(),
            "http://127.0.0.1:8200/v1/kv/metadata?list=true"
        );
        assert_eq!(
            list_url(&cfg, KvVersion::V2, "agents/").unwrap(),
            "http://127.0.0.1:8200/v1/kv/metadata/agents?list=true"
        );
        assert_eq!(
            list_url(&cfg, KvVersion::V1, "").unwrap(),
            "http://127.0.0.1:8200/v1/kv/?list=true"
        );
        assert_eq!(
            list_url(&cfg, KvVersion::V1, "agents").unwrap(),
            "http://127.0.0.1:8200/v1/kv/agents?list=true"
        );
    }

    #[test]
    fn data_and_destroy_urls_speak_each_version() {
        let cfg = cfg_with("kv");
        assert_eq!(
            data_url(&cfg, KvVersion::V2, "/a/b").unwrap(),
            "http://127.0.0.1:8200/v1/kv/data/a/b"
        );
        assert_eq!(
            data_url(&cfg, KvVersion::V1, "/a/b").unwrap(),
            "http://127.0.0.1:8200/v1/kv/a/b"
        );
        assert_eq!(
            destroy_url(&cfg, KvVersion::V2, "a/b").unwrap(),
            "http://127.0.0.1:8200/v1/kv/metadata/a/b"
        );
        assert_eq!(
            destroy_url(&cfg, KvVersion::V1, "a/b").unwrap(),
            "http://127.0.0.1:8200/v1/kv/a/b",
            "v1 has no metadata: it deletes the path"
        );
    }

    #[test]
    fn a_v2_reply_is_unwrapped_and_a_v1_reply_is_not() {
        let v2 = json!({
            "data": {
                "data": { "K": "v" },
                "metadata": { "version": 3 }
            }
        });
        let v1 = json!({ "data": { "K": "v" } });

        assert_eq!(
            fields_from_json(KvVersion::V2, v2.clone())
                .get("K")
                .and_then(Value::as_str),
            Some("v")
        );
        assert_eq!(
            fields_from_json(KvVersion::V1, v1.clone())
                .get("K")
                .and_then(Value::as_str),
            Some("v")
        );
        assert!(
            fields_from_json(KvVersion::V1, v2.clone()).get("K").is_none(),
            "reading a v2 with v1 paths does not find the field"
        );
        assert!(
            fields_from_json(KvVersion::V2, v1).is_empty(),
            "reading a v1 with v2 paths does not invent fields"
        );
        assert!(fields_from_json(KvVersion::V2, json!({ "data": Value::Null })).is_empty());
    }

    #[test]
    fn writes_are_wrapped_only_for_v2() {
        let mut fields = serde_json::Map::new();
        fields.insert("K".into(), Value::String("v".into()));

        assert_eq!(
            write_body(KvVersion::V2, fields.clone()),
            json!({ "data": { "K": "v" } })
        );
        assert_eq!(write_body(KvVersion::V1, fields), json!({ "K": "v" }));
    }
}
