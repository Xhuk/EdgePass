//! Where the widget was left.
//!
//! Without this, every launch dropped the pill back at the OS default spot, so
//! a docked edge or a hand-picked floating spot was lost on each restart.
//!
//! The placement is stored **relative** — the monitor it lived on plus its
//! centre as a fraction of that monitor's work area — and not as absolute
//! pixels. A resolution or DPI change would otherwise park the pill off screen,
//! and this widget has no taskbar entry and no tray icon to drag it back from.
//!
//! Only the *numbers* live here. Which edge, what size and how to clamp it are
//! `dock.ts`'s rules, and Rust does not keep a second copy of them.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Dock edges the UI can stick to. Kept as strings so a value from a newer or
/// hand-edited file degrades to "floating" instead of failing the whole load.
const DOCK_EDGES: [&str; 3] = ["left", "right", "top"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Placement {
    /// Monitor the pill was on. Empty = unknown, the UI falls back to the
    /// primary one.
    #[serde(default)]
    pub monitor: String,
    /// `None` = free-floating pill (not glued to an edge).
    #[serde(default)]
    pub dock: Option<String>,
    /// Centre of the collapsed pill as a fraction of the work area, 0..=1.
    pub x: f64,
    pub y: f64,
}

impl Placement {
    /// Reject what could put the pill nowhere: a truncated file, a hand-edited
    /// ratio outside the screen, a non-finite number. A dock edge this version
    /// does not know about is downgraded to "floating" rather than dropped —
    /// the position is still good.
    fn sanitized(mut self) -> Option<Self> {
        if !self.x.is_finite() || !self.y.is_finite() {
            return None;
        }
        self.x = self.x.clamp(0.0, 1.0);
        self.y = self.y.clamp(0.0, 1.0);
        if let Some(edge) = self.dock.as_deref() {
            if !DOCK_EDGES.contains(&edge) {
                self.dock = None;
            }
        }
        Some(self)
    }
}

pub fn path() -> Result<PathBuf, String> {
    Ok(crate::config::local_data_dir()?.join("placement.json"))
}

/// Where <= 0.4.x kept it (Roaming). Read once and moved, so the old location
/// stops being a second source of truth.
fn legacy_path() -> Option<PathBuf> {
    crate::config::data_dir()
        .ok()
        .map(|dir| dir.join("placement.json"))
}

/// `None` covers both "first run" and "unreadable file": neither is worth
/// blocking a launch over, the widget just opens where the OS puts it.
pub fn load() -> Option<Placement> {
    let path = match path() {
        Ok(p) => p,
        Err(e) => {
            crate::log_warn!("could not resolve placement.json: {e}");
            return None;
        }
    };
    let placement = load_from(&path).or_else(|| {
        let old = legacy_path()?;
        if old == path {
            return None;
        }
        let placement = load_from(&old)?;
        if fs::rename(&old, &path).is_err() {
            // Different volume (or the file is briefly locked): copy instead of
            // losing the spot.
            let _ = fs::copy(&old, &path);
            let _ = fs::remove_file(&old);
        }
        Some(placement)
    })?;
    crate::log_info!(
        "starting where I left off: dock={:?} monitor='{}' centre=({:.3}, {:.3})",
        placement.dock,
        placement.monitor,
        placement.x,
        placement.y
    );
    Some(placement)
}

pub fn save(placement: &Placement) -> Result<(), String> {
    save_to(&path()?, placement)
}

fn load_from(path: &Path) -> Option<Placement> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            crate::log_warn!("could not read {}: {e}", path.display());
            return None;
        }
    };
    // A BOM is what PowerShell 5.1 writes by default (`Set-Content -Encoding
    // UTF8`), and serde_json refuses it as "expected value at line 1 column 1".
    // Same tolerance as the OpenBao init file in `config.rs`: a file a human (or
    // an agent) can hand-edit must not be this easy to break.
    let raw = raw.trim_start_matches('\u{feff}');
    match serde_json::from_str::<Placement>(raw) {
        Ok(placement) => placement.sanitized(),
        Err(e) => {
            crate::log_warn!(
                "{} is not a valid placement ({e}); starting where the OS puts me",
                path.display()
            );
            None
        }
    }
}

fn save_to(path: &Path, placement: &Placement) -> Result<(), String> {
    let raw = serde_json::to_string_pretty(placement).map_err(|e| e.to_string())?;
    fs::write(path, raw).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Distinct file per test: they run in parallel and must not share state.
    fn temp_path(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("edgepass-placement-{tag}-{}.json", std::process::id()))
    }

    #[test]
    fn round_trips_through_the_file() {
        let path = temp_path("roundtrip");
        let wrote = Placement {
            monitor: r"\\.\DISPLAY2".into(),
            dock: Some("right".into()),
            x: 0.98,
            y: 0.31,
        };
        save_to(&path, &wrote).expect("save");
        assert_eq!(load_from(&path), Some(wrote));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn missing_file_is_first_run_not_an_error() {
        assert_eq!(load_from(&temp_path("absent")), None);
    }

    #[test]
    fn unknown_dock_edge_keeps_the_position() {
        let path = temp_path("unknown-dock");
        fs::write(
            &path,
            r#"{"monitor":"","dock":"diagonal","x":0.5,"y":0.4}"#,
        )
        .expect("write");
        let loaded = load_from(&path).expect("loads");
        assert_eq!(loaded.dock, None, "degrades to floating");
        assert_eq!((loaded.x, loaded.y), (0.5, 0.4));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn ratios_outside_the_work_area_are_clamped() {
        let clamped = Placement {
            monitor: String::new(),
            dock: None,
            x: -3.0,
            y: 12.5,
        }
        .sanitized()
        .expect("finite");
        assert_eq!((clamped.x, clamped.y), (0.0, 1.0));
    }

    #[test]
    fn non_finite_ratios_are_refused() {
        for (x, y) in [(f64::NAN, 0.5), (0.5, f64::INFINITY)] {
            let bad = Placement {
                monitor: String::new(),
                dock: None,
                x,
                y,
            };
            assert_eq!(bad.sanitized(), None, "({x}, {y})");
        }
    }

    #[test]
    fn a_byte_order_mark_is_tolerated() {
        let path = temp_path("bom");
        // What PowerShell 5.1's `Set-Content -Encoding UTF8` writes.
        fs::write(
            &path,
            "\u{feff}{\"monitor\":\"m\",\"dock\":\"top\",\"x\":0.4,\"y\":0.0}",
        )
        .expect("write");
        let loaded = load_from(&path).expect("loads");
        assert_eq!(loaded.dock.as_deref(), Some("top"));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn truncated_file_is_ignored() {
        let path = temp_path("truncated");
        fs::write(&path, r#"{"dock":"left","x":0.5"#).expect("write");
        assert_eq!(load_from(&path), None);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn an_old_file_without_optional_fields_still_loads() {
        let path = temp_path("sparse");
        fs::write(&path, r#"{"x":0.25,"y":0.75}"#).expect("write");
        let loaded = load_from(&path).expect("loads");
        assert_eq!(loaded.monitor, "");
        assert_eq!(loaded.dock, None);
        let _ = fs::remove_file(&path);
    }
}
