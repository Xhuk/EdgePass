//! The single gate for every HTTP call to OpenBao.
//!
//! `assert_local_addr` used to guard only `seal`/`unseal`, while reads and
//! writes used `cfg.bao_addr` unchecked. Editing one field of `config.json`
//! was therefore enough to make the stored root token travel to a remote host
//! in `X-Vault-Token`. Every request now goes through [`client`], which
//! validates the address *before* building a client.
//!
//! The client also sets `.no_proxy()`: this is a loopback-only client, and a
//! stray `HTTP_PROXY` in the environment must never see the vault token.

use std::time::Duration;
use url::{Host, Url};

/// True only for addresses that resolve to this machine's loopback.
pub fn is_loopback(addr: &str) -> bool {
    match Url::parse(addr) {
        Ok(u) => match u.host() {
            Some(Host::Ipv4(ip)) => ip.is_loopback(),
            Some(Host::Ipv6(ip)) => ip.is_loopback(),
            Some(Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
            None => false,
        },
        Err(_) => false,
    }
}

pub fn assert_local(addr: &str) -> Result<(), String> {
    if is_loopback(addr) {
        return Ok(());
    }
    let host = Url::parse(addr)
        .ok()
        .and_then(|u| u.host_str().map(|s| s.to_string()))
        .unwrap_or_else(|| addr.to_string());
    Err(format!(
        "Rechazado '{host}': edgePass solo habla con OpenBao en loopback (127.0.0.1 / ::1 / localhost). \
         Así el token nunca sale de esta máquina — cámbialo en Ajustes si de verdad quieres otro host."
    ))
}

/// Validated, proxy-free client for the configured OpenBao address.
pub fn client(addr: &str, secs: u64) -> Result<reqwest::Client, String> {
    assert_local(addr)?;
    reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(secs))
        .build()
        .map_err(|e| e.to_string())
}

/// `join_url("http://127.0.0.1:8200/", &["v1", "kv", "data", rel])`.
/// Empty segments are skipped so an empty relative path stays clean.
pub fn join_url(addr: &str, parts: &[&str]) -> String {
    let mut url = addr.trim_end_matches('/').to_string();
    for part in parts {
        let seg = part.trim_matches('/');
        if seg.is_empty() {
            continue;
        }
        url.push('/');
        url.push_str(seg);
    }
    url
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_only() {
        assert!(is_loopback("http://127.0.0.1:8200"));
        assert!(is_loopback("http://localhost:8200"));
        assert!(is_loopback("http://[::1]:8200"));
        assert!(is_loopback("http://127.5.5.5:8200"));
        assert!(!is_loopback("http://192.168.1.10:8200"));
        assert!(!is_loopback("https://vault.example.com"));
        assert!(!is_loopback("not a url"));
    }

    #[test]
    fn joins_without_double_slashes() {
        assert_eq!(
            join_url("http://127.0.0.1:8200/", &["v1", "kv", "data", "a/b"]),
            "http://127.0.0.1:8200/v1/kv/data/a/b"
        );
        assert_eq!(
            join_url("http://127.0.0.1:8200", &["v1", "kv", "data", ""]),
            "http://127.0.0.1:8200/v1/kv/data"
        );
    }
}
