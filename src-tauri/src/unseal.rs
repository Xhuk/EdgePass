use crate::config::AppConfig;
use serde::Serialize;
use serde_json::json;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SealStatusDto {
    pub reachable: bool,
    pub health_code: u16,
    pub initialized: bool,
    pub sealed: bool,
    pub progress: u32,
    pub threshold: u32,
    pub version: Option<String>,
    pub message: String,
}

/// Delegates to the shared loopback guard. The old local copy covered only
/// seal/unseal, so a read or write could still ship the token off-box.
fn assert_local_addr(addr: &str) -> Result<(), String> {
    crate::net::assert_local(addr)
}

fn join_url(addr: &str, parts: &[&str]) -> Result<String, String> {
    crate::net::join_url(addr, parts)
}

/// Loopback-only and proxy-free, like every other call to the vault.
fn http_client(cfg: &AppConfig, secs: u64) -> Result<reqwest::Client, String> {
    crate::net::client(&cfg.bao_addr, secs)
}

pub async fn seal_status(cfg: &AppConfig) -> SealStatusDto {
    // Refuse before any request: the UI then shows "down" with the reason
    // instead of silently talking to whatever host config.json points at.
    if let Err(refusal) = crate::net::assert_local(&cfg.bao_addr) {
        return SealStatusDto {
            reachable: false,
            health_code: 0,
            initialized: false,
            sealed: true,
            progress: 0,
            threshold: 0,
            version: None,
            message: refusal,
        };
    }

    let http = match http_client(cfg, 5) {
        Ok(c) => c,
        Err(e) => {
            return SealStatusDto {
                reachable: false,
                health_code: 0,
                initialized: false,
                sealed: true,
                progress: 0,
                threshold: 0,
                version: None,
                message: e,
            };
        }
    };

    // Constant parts: the validator cannot reject them, but the gate is the
    // same one every other call goes through, so refuse instead of panic.
    let (health_url, status_url) = match (
        join_url(&cfg.bao_addr, &["v1", "sys", "health"]),
        join_url(&cfg.bao_addr, &["v1", "sys", "seal-status"]),
    ) {
        (Ok(health), Ok(status)) => (health, status),
        (Err(e), _) | (_, Err(e)) => {
            return SealStatusDto {
                reachable: false,
                health_code: 0,
                initialized: false,
                sealed: true,
                progress: 0,
                threshold: 0,
                version: None,
                message: e,
            }
        }
    };
    let health_code = match http.get(&health_url).send().await {
        Ok(res) => res.status().as_u16(),
        Err(_) => {
            return SealStatusDto {
                reachable: false,
                health_code: 0,
                initialized: false,
                sealed: true,
                progress: 0,
                threshold: 0,
                version: None,
                message: "OpenBao no responde (¿servicio caído?)".into(),
            };
        }
    };

    let body: serde_json::Value = match http.get(&status_url).send().await {
        Ok(res) => match res.json().await {
            Ok(v) => v,
            Err(e) => {
                return SealStatusDto {
                    reachable: true,
                    health_code,
                    initialized: false,
                    sealed: true,
                    progress: 0,
                    threshold: 0,
                    version: None,
                    message: e.to_string(),
                };
            }
        },
        Err(e) => {
            return SealStatusDto {
                reachable: health_code != 0,
                health_code,
                initialized: false,
                sealed: true,
                progress: 0,
                threshold: 0,
                version: None,
                message: e.to_string(),
            };
        }
    };

    let sealed = body.get("sealed").and_then(|v| v.as_bool()).unwrap_or(true);
    let initialized = body
        .get("initialized")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let progress = body.get("progress").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let threshold = body.get("t").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let version = body
        .get("version")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let message = if !initialized {
        "No inicializado".into()
    } else if sealed {
        format!("Sellado ({progress}/{threshold})")
    } else {
        "Abierto".into()
    };

    SealStatusDto {
        reachable: true,
        health_code,
        initialized,
        sealed,
        progress,
        threshold,
        version,
        message,
    }
}

fn read_unseal_keys(init_path: &str) -> Result<Vec<String>, String> {
    let path = PathBuf::from(init_path);
    if !path.exists() {
        return Err(format!("init file not found: {init_path}"));
    }
    let raw = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let raw = raw.trim_start_matches('\u{feff}');
    let init: serde_json::Value = serde_json::from_str(raw).map_err(|e| e.to_string())?;

    // OpenBao local init uses unseal_keys_b64 first.
    let keys: Vec<String> = init
        .get("unseal_keys_b64")
        .or_else(|| init.get("keys"))
        .or_else(|| init.get("unseal_keys_hex"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect()
        })
        .ok_or_else(|| "No unseal keys found in init file".to_string())?;

    if keys.is_empty() {
        return Err("Init file has empty keys array".into());
    }
    Ok(keys)
}

/// Unseal using keys from local init JSON. Never returns key material.
pub async fn unseal(cfg: &AppConfig) -> Result<SealStatusDto, String> {
    assert_local_addr(&cfg.bao_addr)?;
    let keys = read_unseal_keys(&cfg.init_path)?;

    let before = seal_status(cfg).await;
    if !before.reachable {
        return Err(before.message);
    }
    if !before.initialized {
        return Err("OpenBao is not initialized".into());
    }
    if !before.sealed {
        return Ok(before);
    }

    let threshold = if before.threshold > 0 {
        before.threshold as usize
    } else {
        keys.len().clamp(1, 3)
    };

    let http = http_client(cfg, 10)?;
    let url = join_url(&cfg.bao_addr, &["v1", "sys", "unseal"])?;
    for key in keys.iter().take(threshold) {
        let res = http
            .put(&url)
            .json(&json!({ "key": key }))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !res.status().is_success() {
            let status = res.status();
            let body = res.text().await.unwrap_or_default();
            let safe = body.replace('\n', " ");
            let clipped: String = safe.chars().take(120).collect();
            return Err(format!("unseal HTTP {status}: {clipped}"));
        }
        let body: serde_json::Value = res.json().await.map_err(|e| e.to_string())?;
        let sealed = body.get("sealed").and_then(|v| v.as_bool()).unwrap_or(true);
        if !sealed {
            break;
        }
    }

    let after = seal_status(cfg).await;
    // Unsealed is not the same as *active*: after the last key the node still has
    // to become the active one, and anything sent in that window is answered with
    // `500 local node not active`. Waiting here is what stops the first write of
    // the day from failing on a vault that is perfectly fine.
    wait_until_active(cfg, &http).await;
    crate::log_info!("unseal: sellado={} ({})", after.sealed, after.message);
    Ok(after)
}

/// Waits for the node to report itself active (`standby: false`).
///
/// Best effort and short on purpose: a single-node install becomes active in
/// milliseconds, and a cluster that is genuinely stuck must report its own error
/// rather than hang the unseal (the write path also retries — see
/// `openbao::post_kv`).
async fn wait_until_active(cfg: &AppConfig, http: &reqwest::Client) {
    let url = match join_url(&cfg.bao_addr, &["v1", "sys", "health"]) {
        Ok(url) => url,
        Err(_) => return,
    };
    for _ in 0..20 {
        let active = match http.get(&url).send().await {
            Ok(res) => match res.json::<serde_json::Value>().await {
                Ok(body) => {
                    let sealed = body.get("sealed").and_then(|v| v.as_bool()).unwrap_or(true);
                    let standby = body.get("standby").and_then(|v| v.as_bool()).unwrap_or(true);
                    !sealed && !standby
                }
                Err(_) => false,
            },
            Err(_) => false,
        };
        if active {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    crate::log_warn!("unseal: el nodo no se anunció activo en 2s; sigo igual");
}

/// Seal vault (requires token). Local-only.
pub async fn seal(cfg: &AppConfig, token: &str) -> Result<SealStatusDto, String> {
    assert_local_addr(&cfg.bao_addr)?;
    let before = seal_status(cfg).await;
    if !before.reachable {
        return Err(before.message);
    }
    if before.sealed {
        return Ok(before);
    }

    let http = http_client(cfg, 10)?;
    let url = join_url(&cfg.bao_addr, &["v1", "sys", "seal"])?;
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        "X-Vault-Token",
        reqwest::header::HeaderValue::from_str(token).map_err(|e| e.to_string())?,
    );
    let res = http
        .put(&url)
        .headers(headers)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        let status = res.status();
        let body = res.text().await.unwrap_or_default();
        let clipped: String = body.replace('\n', " ").chars().take(120).collect();
        return Err(format!("seal HTTP {status}: {clipped}"));
    }
    let after = seal_status(cfg).await;
    crate::log_warn!(
        "seal manual: vault sellado para TODOS sus clientes (sellado={})",
        after.sealed
    );
    Ok(after)
}

/// If sealed → unseal. Returns final status. No-op if already open.
pub async fn ensure_unsealed(cfg: &AppConfig) -> Result<SealStatusDto, String> {
    let status = seal_status(cfg).await;
    if !status.reachable {
        return Err(status.message);
    }
    if !status.initialized {
        return Err("OpenBao no inicializado".into());
    }
    if !status.sealed {
        return Ok(status);
    }
    unseal(cfg).await
}
