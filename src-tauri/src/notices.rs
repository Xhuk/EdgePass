//! The committee's notices, read from where the hook and the watcher leave them.
//!
//! Two PowerShell ends write them: `edgepass-mcp-hook.ps1` for what goes
//! through the MCP (the committee's `thread_post_steering`, an agent's
//! `board_post`) and `edgepass-steering-watch.ps1` for what does not (a note
//! copied by hand, a board touched by another agent). Both end in
//! `edgepass-steering-notify.ps1`, which appends one JSON object per line to
//! `%APPDATA%\edgePass\steering-notices.jsonl` after deduping by
//! path + size + mtime.
//!
//! That `.jsonl` is *their* evidence and stays theirs: this module only reads
//! it, never rewrites it, and never runs either script. What it adds is the one
//! thing the file cannot answer — how many of those lines the human has already
//! looked at, which is the counter on the pill.
//!
//! Counting lines is enough for that because the file is append-only, so "seen"
//! is a mark that only moves forward. It lives in its own file
//! (`steering-notices.seen.json`) rather than in the scripts' dedupe state:
//! neither side has to parse the other's, and losing ours costs one counter.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// How many rows the panel keeps. The file grows forever (nothing prunes it),
/// so the panel is a window onto its tail; only the counter needs the total.
const DEFAULT_LIMIT: usize = 50;

/// One line of `steering-notices.jsonl`, as the two scripts write it.
///
/// Every field is optional on purpose: a line from a hand-edited file, or from
/// a writer that learns a new field later, has to degrade to a row with blanks
/// instead of taking the whole panel down.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Notice {
    /// When it happened, ISO-8601 with offset (`2026-10-06T16:32:27-06:00`).
    #[serde(default)]
    pub at: String,
    /// `steering` (a note) or `board` (a message on the board).
    #[serde(default)]
    pub kind: String,
    /// Which end saw it: `mcp` (the hook), `watch` (the folder watcher), or a
    /// free-form tag a manual run passed in (`manual`, `test`, `catchup`).
    #[serde(default)]
    pub source: String,
    /// The MCP tool, when one was involved: `thread_post_steering`, `board_post`.
    #[serde(default)]
    pub tool: String,
    /// Project root the notice belongs to.
    #[serde(default)]
    pub root: String,
    /// Full path of the file that changed.
    #[serde(default)]
    pub path: String,
    /// Its file name, as the scripts already trimmed it for the toast.
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub bytes: u64,
}

/// What the panel paints and what the pill's counter shows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Notices {
    /// Newest first.
    pub items: Vec<Notice>,
    /// Lines the human has not looked at yet — the counter.
    pub unseen: usize,
    /// Everything the file holds, so the panel can say "50 of 213".
    pub total: usize,
    /// The `.jsonl` exists on this machine. False means the two writer scripts
    /// are not installed, which is the normal state of a fresh public install:
    /// the panel then hides the tab instead of showing internal vocabulary
    /// (comité, hook, watcher, board) to someone who has none of it.
    #[serde(default)]
    pub available: bool,
}

impl Notices {
    /// No file, no data dir or an unreadable mark: the widget shows "no
    /// notices", which is also the honest answer.
    fn empty() -> Self {
        Self {
            items: Vec::new(),
            unseen: 0,
            total: 0,
            available: false,
        }
    }
}

/// The mark, as stored. `{"seen": 12}`.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Seen {
    #[serde(default)]
    seen: usize,
}

fn jsonl_path() -> Result<PathBuf, String> {
    Ok(crate::config::data_dir()?.join("steering-notices.jsonl"))
}

fn seen_path() -> Result<PathBuf, String> {
    Ok(crate::config::data_dir()?.join("steering-notices.seen.json"))
}

/// The last `limit` notices, newest first, plus the counter. Never fails: a
/// missing or unreadable file is "no notices", not a broken header.
pub fn list(limit: Option<usize>) -> Notices {
    let limit = limit.unwrap_or(DEFAULT_LIMIT);
    match (jsonl_path(), seen_path()) {
        (Ok(jsonl), Ok(seen)) => list_from(&jsonl, &seen, limit),
        (Err(e), _) => {
            crate::log_warn!("no pude resolver steering-notices.jsonl: {e}");
            Notices::empty()
        }
        (_, Err(e)) => {
            crate::log_warn!("no pude resolver steering-notices.seen.json: {e}");
            Notices::empty()
        }
    }
}

/// Call everything currently in the file "seen" and hand back what the panel
/// should paint next, so the caller needs a single round trip.
pub fn mark_seen() -> Result<Notices, String> {
    let jsonl = jsonl_path()?;
    let seen = seen_path()?;
    mark_seen_at(&jsonl, &seen)?;
    Ok(list_from(&jsonl, &seen, DEFAULT_LIMIT))
}

fn list_from(jsonl: &Path, seen: &Path, limit: usize) -> Notices {
    let all = read_notices(jsonl);
    let total = all.len();
    // A truncated or recreated file leaves the mark ahead of the content: the
    // counter reads zero rather than going negative.
    let seen = seen_count(seen).min(total);
    let items = all.into_iter().rev().take(limit).collect();
    Notices {
        items,
        unseen: total - seen,
        total,
        // The file exists but is empty counts as installed: the scripts are
        // there and simply have nothing to report yet.
        available: jsonl.exists(),
    }
}

fn mark_seen_at(jsonl: &Path, seen: &Path) -> Result<(), String> {
    let total = read_notices(jsonl).len();
    let raw = serde_json::to_string(&Seen { seen: total }).map_err(|e| e.to_string())?;
    fs::write(seen, raw).map_err(|e| format!("{}: {e}", seen.display()))
}

/// `0` for a missing or unreadable mark: the worst case is a counter that shows
/// everything as new, and one click fixes it.
fn seen_count(path: &Path) -> usize {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(_) => return 0,
    };
    match serde_json::from_str::<Seen>(strip_bom(&raw)) {
        Ok(seen) => seen.seen,
        Err(e) => {
            crate::log_warn!("{} no es un mark válido ({e}); cuento todo como nuevo", path.display());
            0
        }
    }
}

fn read_notices(path: &Path) -> Vec<Notice> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(e) => {
            crate::log_warn!("no pude leer {}: {e}", path.display());
            return Vec::new();
        }
    };
    parse_lines(strip_bom(&raw))
}

/// One JSON object per line, skipping blanks and whatever does not parse: a
/// half-written last line (the scripts append with `Add-Content`) must cost
/// that line only.
fn parse_lines(raw: &str) -> Vec<Notice> {
    raw.lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| match serde_json::from_str::<Notice>(line) {
            Ok(notice) => Some(notice),
            Err(e) => {
                crate::log_warn!("línea ilegible en steering-notices.jsonl ({e})");
                None
            }
        })
        .collect()
}

/// A BOM is what PowerShell 5.1 writes when it creates the file, and serde_json
/// refuses it as "expected value at line 1 column 1". Same tolerance as
/// `placement.rs`: a file humans and scripts both write must not be this easy
/// to break.
fn strip_bom(raw: &str) -> &str {
    raw.trim_start_matches('\u{feff}')
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Distinct file per test: they run in parallel and must not share state.
    fn temp_path(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("edgepass-notices-{tag}-{}.jsonl", std::process::id()))
    }

    fn write(path: &Path, raw: &str) {
        fs::write(path, raw).expect("write");
    }

    fn line(at: &str, name: &str) -> String {
        format!(
            r#"{{"at":"{at}","kind":"steering","source":"watch","tool":"","root":"C:\\proyectos\\lotly","path":"C:\\proyectos\\lotly\\.reasonix\\steering\\{name}","name":"{name}","bytes":7}}"#
        )
    }

    #[test]
    fn reads_the_lines_newest_first() {
        let jsonl = temp_path("order");
        let seen = temp_path("order-seen");
        write(
            &jsonl,
            &format!(
                "{}\n{}\n",
                line("2026-10-06T16:28:14-06:00", "003-nota.md"),
                line("2026-10-06T16:32:27-06:00", "016-nota.md")
            ),
        );
        let got = list_from(&jsonl, &seen, 50);
        assert_eq!(got.total, 2);
        assert_eq!(got.unseen, 2, "sin mark, todo es nuevo");
        assert_eq!(got.items[0].name, "016-nota.md");
        assert_eq!(got.items[1].name, "003-nota.md");
        let _ = fs::remove_file(&jsonl);
    }

    #[test]
    fn the_limit_keeps_the_tail_and_the_total_is_still_everything() {
        let jsonl = temp_path("limit");
        let seen = temp_path("limit-seen");
        let body: String = (1..=5)
            .map(|i| format!("{}\n", line("2026-10-06T16:00:00-06:00", &format!("00{i}.md"))))
            .collect();
        write(&jsonl, &body);
        let got = list_from(&jsonl, &seen, 2);
        assert_eq!(got.items.len(), 2);
        assert_eq!(got.items[0].name, "005.md");
        assert_eq!(got.total, 5, "el contador no se limita al panel");
        let _ = fs::remove_file(&jsonl);
    }

    #[test]
    fn a_bom_and_a_broken_line_cost_only_that_line() {
        let jsonl = temp_path("bom");
        let seen = temp_path("bom-seen");
        // The BOM is what PowerShell 5.1 leaves when it creates the file; the
        // middle line is what a half-finished append looks like.
        let first = line("2026-10-06T16:28:14-06:00", "003-nota.md");
        let broken = "{\"at\": 2026-10-06T16";
        let last = line("2026-10-06T16:32:27-06:00", "016-nota.md");
        write(&jsonl, &format!("\u{feff}{first}\n{broken}\n\n{last}\n"));
        let got = list_from(&jsonl, &seen, 50);
        assert_eq!(got.total, 2, "el BOM, la línea cortada y el blanco no cuentan");
        assert_eq!(got.items[0].name, "016-nota.md");
        let _ = fs::remove_file(&jsonl);
    }

    #[test]
    fn missing_files_are_no_notices_not_an_error() {
        let got = list_from(&temp_path("absent"), &temp_path("absent-seen"), 50);
        assert_eq!(got, Notices::empty());
    }

    #[test]
    fn marking_seen_zeroes_the_counter_and_is_what_gets_painted() {
        let jsonl = temp_path("mark");
        let seen = temp_path("mark-seen");
        write(
            &jsonl,
            &format!(
                "{}\n{}\n",
                line("2026-10-06T16:28:14-06:00", "003-nota.md"),
                line("2026-10-06T16:32:27-06:00", "016-nota.md")
            ),
        );
        mark_seen_at(&jsonl, &seen).expect("mark");
        let got = list_from(&jsonl, &seen, 50);
        assert_eq!(got.unseen, 0);
        assert_eq!(got.total, 2);
        assert_eq!(got.items.len(), 2, "vistos y todo: la lista no se vacía");
        let _ = fs::remove_file(&jsonl);
        let _ = fs::remove_file(&seen);
    }

    #[test]
    fn only_the_lines_after_the_mark_count_as_new() {
        let jsonl = temp_path("append");
        let seen = temp_path("append-seen");
        let first = line("2026-10-06T16:28:14-06:00", "003-nota.md");
        write(&jsonl, &format!("{first}\n"));
        mark_seen_at(&jsonl, &seen).expect("mark");
        // The scripts only ever append, so the same first line is still there.
        write(&jsonl, &format!("{first}\n{}\n", line("2026-10-06T16:32:27-06:00", "016-nota.md")));
        let got = list_from(&jsonl, &seen, 50);
        assert_eq!(got.unseen, 1);
        assert_eq!(got.items[0].name, "016-nota.md");
        let _ = fs::remove_file(&jsonl);
        let _ = fs::remove_file(&seen);
    }

    #[test]
    fn a_file_that_shrank_does_not_go_negative() {
        let jsonl = temp_path("shrank");
        let seen = temp_path("shrank-seen");
        write(&jsonl, &format!("{}\n", line("2026-10-06T16:28:14-06:00", "003-nota.md")));
        write(&seen, r#"{"seen":9}"#);
        let got = list_from(&jsonl, &seen, 50);
        assert_eq!(got.unseen, 0);
        assert_eq!(got.total, 1);
        let _ = fs::remove_file(&jsonl);
        let _ = fs::remove_file(&seen);
    }

    #[test]
    fn a_corrupt_mark_counts_everything_as_new() {
        let jsonl = temp_path("corrupt-mark");
        let seen = temp_path("corrupt-mark-seen");
        write(&jsonl, &format!("{}\n", line("2026-10-06T16:28:14-06:00", "003-nota.md")));
        write(&seen, "{ no soy json");
        assert_eq!(list_from(&jsonl, &seen, 50).unseen, 1);
        let _ = fs::remove_file(&jsonl);
        let _ = fs::remove_file(&seen);
    }
}
