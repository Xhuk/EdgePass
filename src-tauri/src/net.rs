//! The single gate for every HTTP call to OpenBao.
//!
//! `assert_local_addr` used to guard only `seal`/`unseal`, while reads and
//! writes used `cfg.bao_addr` unchecked. Editing one field of `config.json`
//! was therefore enough to make the stored root token travel to a remote host
//! in `X-Vault-Token`. Every request now goes through [`client`], which
//! validates the address *before* building a client.
//!
//! The client also sets `.no_proxy()`: this is a loopback-only client, and a
//! stray `HTTP_PROXY` in the environment must never see the vault token. It
//! also sets `.redirect(none)`: `X-Vault-Token` is a custom header, and a
//! custom header is *not* stripped when a redirect changes host, so a hostile
//! answer on 127.0.0.1:8200 could hand the token to a remote site.
//!
//! [`join_url`] validates every segment of the relative path. A caller that
//! speaks the local pipe controls part of that path, and the URL parser folds
//! `..` away, so `surfaces/demo-runtime/../saas-runtime` used to read a
//! different path than the one it declared.

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

/// Validated, proxy-free, redirect-free client for the configured address.
pub fn client(addr: &str, secs: u64) -> Result<reqwest::Client, String> {
    assert_local(addr)?;
    reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(secs))
        .build()
        .map_err(|e| e.to_string())
}

/// Decodes `%XX` escapes. Invalid escapes are kept as they are, so a literal
/// `%` in a key name still passes through untouched.
fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(v) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Rejects one path segment that could escape its mount or bend the request.
///
/// The check runs on the percent-decoded form, so `%2e%2e` and `%2f` are
/// caught the same way as `..` and `/`. A normal key name — dots, dashes,
/// underscores, spaces, unicode — passes untouched.
fn check_segment(seg: &str) -> Result<(), String> {
    let decoded = percent_decode(seg);
    let verboten = if decoded == "." || decoded == ".." {
        Some("no puede ser '.' ni '..'")
    } else if decoded.contains('/') || decoded.contains('\\') {
        Some("no puede contener '/' ni '\\'")
    } else if decoded.contains('?') || decoded.contains('#') {
        Some("no puede contener '?' ni '#'")
    } else if decoded.contains('\0') || seg.chars().any(char::is_control) {
        Some("no puede contener caracteres de control")
    } else {
        None
    };
    match verboten {
        None => Ok(()),
        Some(why) => Err(format!(
            "Ruta rechazada: el segmento '{seg}' {why}. edgePass no permite salir del path declarado."
        )),
    }
}

/// `join_url("http://127.0.0.1:8200/", &["v1", "kv", "data", rel])`.
/// Empty segments are skipped so an empty relative path stays clean.
///
/// Every `/`-separated piece is validated by [`check_segment`] before it lands
/// in the URL, so a caller cannot read or write a path it did not declare.
pub fn join_url(addr: &str, parts: &[&str]) -> Result<String, String> {
    let mut url = addr.trim_end_matches('/').to_string();
    for part in parts {
        let seg = part.trim_matches('/');
        if seg.is_empty() {
            continue;
        }
        for piece in seg.split('/') {
            if !piece.is_empty() {
                check_segment(piece)?;
            }
        }
        url.push('/');
        url.push_str(seg);
    }
    Ok(url)
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
            join_url("http://127.0.0.1:8200/", &["v1", "kv", "data", "a/b"]).unwrap(),
            "http://127.0.0.1:8200/v1/kv/data/a/b"
        );
        assert_eq!(
            join_url("http://127.0.0.1:8200", &["v1", "kv", "data", ""]).unwrap(),
            "http://127.0.0.1:8200/v1/kv/data"
        );
    }

    #[test]
    fn refuses_to_escape_the_declared_path() {
        for bad in [
            "surfaces/demo-runtime/../saas-runtime",
            "../saas-runtime",
            "a/../../sys/seal",
            "./sys",
            "%2e%2e/sys",
            "%2E%2E/sys",
            "a/%2f/b",
            "a/\\..\\b",
        ] {
            assert!(
                join_url("http://127.0.0.1:8200/", &["v1", "kv", "data", bad]).is_err(),
                "debio rechazar {bad}"
            );
        }
    }

    #[test]
    fn refuses_query_and_fragment_injection() {
        for bad in ["sys/health?x=1", "a#b", "a/?#b"] {
            assert!(join_url("http://127.0.0.1:8200/", &["v1", bad]).is_err());
        }
    }

    #[test]
    fn accepts_real_key_names() {
        for good in [
            "mi-api.example.com",
            "personal/logins/developer",
            "integrations/status/push-token",
            "a-b_c d",
            "100%",
            "mi-app/surfaces/demo-runtime",
        ] {
            assert!(
                join_url("http://127.0.0.1:8200/", &["v1", "kv", "data", good]).is_ok(),
                "debio aceptar {good}"
            );
        }
    }
}
