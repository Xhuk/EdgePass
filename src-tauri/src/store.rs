use crate::openbao::SecretRef;
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

/// One row of the search index. Deliberately has **no** database `id`:
/// `replace_index` rewrites the whole table, so rowids are reassigned on every
/// sync and a hit list held by the UI could resolve to a *different* secret —
/// i.e. clicking a row could copy the wrong value. `(namespace, path, key)` is
/// `UNIQUE` in the schema and is the stable identity the frontend sends back.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub namespace: String,
    pub path: String,
    pub key: String,
    pub use_count: i64,
    pub last_used_at: Option<String>,
    /// Cuántas notas tiene su path (autogeneradas y escritas a mano). El valor
    /// nunca sale de acá: sólo la referencia.
    pub notes: i64,
    pub score: f64,
}

pub struct Store {
    conn: Connection,
}

/// Una nota ligada a un secreto — o al path entero, cuando `key` viene vacío.
///
/// Vive en esta base y **no** en OpenBao a propósito: el vault guarda valores y
/// su metadata es del motor KV; meterle comentarios obliga a reescribir la
/// versión, ensucia el path y en KV v1 ni existe. Acá sólo se guarda la
/// *referencia* (`namespace`, `path`, `key`), nunca el valor.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Note {
    pub id: i64,
    pub namespace: String,
    pub path: String,
    pub key: String,
    pub body: String,
    /// `auto` = la escribió edgePass al entregar el secreto · `user` = humana o
    /// del agente.
    pub kind: String,
    /// Quién la pidió: `ui`, `pipe`, el nombre del tool, la sesión…
    pub actor: String,
    pub created_at: String,
}

/// Cuántas notas se conservan por path. Un historial de accesos sin tope crece
/// para siempre y termina siendo un log, no una nota.
pub const NOTES_PER_PATH: usize = 50;

impl Store {
    pub fn open(path: &std::path::Path) -> Result<Self, String> {
        let conn = Connection::open(path).map_err(|e| e.to_string())?;
        let store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> Result<(), String> {
        self.conn
            .execute_batch(
                r#"
                PRAGMA journal_mode=WAL;
                CREATE TABLE IF NOT EXISTS secrets (
                  id INTEGER PRIMARY KEY,
                  namespace TEXT NOT NULL,
                  path TEXT NOT NULL,
                  key TEXT NOT NULL,
                  use_count INTEGER NOT NULL DEFAULT 0,
                  last_used_at TEXT,
                  UNIQUE(namespace, path, key)
                );
                CREATE INDEX IF NOT EXISTS idx_secrets_search
                  ON secrets(namespace, path, key);
                CREATE TABLE IF NOT EXISTS meta (
                  k TEXT PRIMARY KEY,
                  v TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS notes (
                  id INTEGER PRIMARY KEY,
                  namespace TEXT NOT NULL,
                  path TEXT NOT NULL,
                  key TEXT NOT NULL DEFAULT '',
                  body TEXT NOT NULL,
                  kind TEXT NOT NULL DEFAULT 'user',
                  actor TEXT NOT NULL DEFAULT '',
                  created_at TEXT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_notes_target
                  ON notes(namespace, path);
                "#,
            )
            .map_err(|e| e.to_string())
    }

    /// Escribe una nota. `body` vacío no se guarda: una nota sin texto es ruido.
    pub fn add_note(
        &self,
        namespace: &str,
        path: &str,
        key: &str,
        body: &str,
        kind: &str,
        actor: &str,
    ) -> Result<i64, String> {
        let body = body.trim();
        if body.is_empty() {
            return Err("una nota vacía no se guarda".into());
        }
        let created = Utc::now().to_rfc3339();
        self.conn
            .execute(
                "INSERT INTO notes (namespace, path, key, body, kind, actor, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    namespace.trim(),
                    path.trim().trim_matches('/'),
                    key.trim(),
                    body,
                    kind,
                    actor,
                    created
                ],
            )
            .map_err(|e| e.to_string())?;
        let id = self.conn.last_insert_rowid();
        self.prune_notes(namespace, path)?;
        Ok(id)
    }

    /// Las últimas notas de un path, de la más nueva a la más vieja.
    pub fn notes_for(
        &self,
        namespace: &str,
        path: &str,
        key: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Note>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, namespace, path, key, body, kind, actor, created_at
                   FROM notes
                  WHERE namespace = ?1 AND path = ?2
                    AND (?3 IS NULL OR key = '' OR key = ?3)
                  ORDER BY id DESC LIMIT ?4",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(
                params![
                    namespace.trim(),
                    path.trim().trim_matches('/'),
                    key,
                    limit as i64
                ],
                |row| {
                    Ok(Note {
                        id: row.get(0)?,
                        namespace: row.get(1)?,
                        path: row.get(2)?,
                        key: row.get(3)?,
                        body: row.get(4)?,
                        kind: row.get(5)?,
                        actor: row.get(6)?,
                        created_at: row.get(7)?,
                    })
                },
            )
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
    }

    /// Cuántas notas tiene cada path de un namespace — lo que pinta el contador
    /// en la lista de secretos, en una sola consulta.
    pub fn note_counts(&self, namespace: &str) -> Result<Vec<(String, i64)>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT path, COUNT(*) FROM notes WHERE namespace = ?1 GROUP BY path",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![namespace.trim()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
    }

    /// Deja las últimas [`NOTES_PER_PATH`] de ese path y borra el resto.
    fn prune_notes(&self, namespace: &str, path: &str) -> Result<(), String> {
        self.conn
            .execute(
                "DELETE FROM notes
                  WHERE namespace = ?1 AND path = ?2
                    AND id NOT IN (
                      SELECT id FROM notes
                       WHERE namespace = ?1 AND path = ?2
                       ORDER BY id DESC LIMIT ?3
                    )",
                params![
                    namespace.trim(),
                    path.trim().trim_matches('/'),
                    NOTES_PER_PATH as i64
                ],
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// Rebuilds the whole index in one transaction, carrying `use_count` and
    /// `last_used_at` over for references that still exist. Rowids are NOT
    /// preserved — nothing may depend on them (see [`SearchHit`]).
    pub fn replace_index(&self, items: &[SecretRef]) -> Result<usize, String> {
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| e.to_string())?;

        tx.execute(
            "CREATE TEMP TABLE IF NOT EXISTS _keep AS SELECT namespace, path, key, use_count, last_used_at FROM secrets",
            [],
        )
        .map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM secrets", [])
            .map_err(|e| e.to_string())?;

        {
            let mut insert = tx
                .prepare(
                    "INSERT INTO secrets(namespace, path, key, use_count, last_used_at)
                     VALUES (?1, ?2, ?3,
                       COALESCE((SELECT use_count FROM _keep WHERE namespace=?1 AND path=?2 AND key=?3), 0),
                       (SELECT last_used_at FROM _keep WHERE namespace=?1 AND path=?2 AND key=?3)
                     )",
                )
                .map_err(|e| e.to_string())?;

            for item in items {
                insert
                    .execute(params![item.namespace, item.path, item.key])
                    .map_err(|e| e.to_string())?;
            }
        }

        tx.execute("DROP TABLE IF EXISTS _keep", [])
            .map_err(|e| e.to_string())?;
        tx.execute(
            "INSERT INTO meta(k,v) VALUES('last_sync', ?1)
             ON CONFLICT(k) DO UPDATE SET v=excluded.v",
            params![Utc::now().to_rfc3339()],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(items.len())
    }

    pub fn last_sync(&self) -> Result<Option<String>, String> {
        self.conn
            .query_row("SELECT v FROM meta WHERE k='last_sync'", [], |r| {
                r.get::<_, String>(0)
            })
            .optional()
            .map_err(|e| e.to_string())
    }

    pub fn count(&self) -> Result<i64, String> {
        self.conn
            .query_row("SELECT COUNT(*) FROM secrets", [], |r| r.get(0))
            .map_err(|e| e.to_string())
    }

    /// Bump usage for a secret identified by its stable reference.
    pub fn record_use_ref(&self, namespace: &str, path: &str, key: &str) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE secrets SET use_count = use_count + 1, last_used_at = ?1
                 WHERE namespace = ?2 AND path = ?3 AND key = ?4",
                params![Utc::now().to_rfc3339(), namespace, path, key],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchHit>, String> {
        let groups = tokenize_query(query);
        let mut stmt = self
            .conn
            .prepare(
                "SELECT namespace, path, key, use_count, last_used_at,
                        (SELECT COUNT(*) FROM notes n
                          WHERE n.namespace = s.namespace AND n.path = s.path)
                   FROM secrets s
                 ORDER BY use_count DESC, last_used_at DESC NULLS LAST, namespace, path, key",
            )
            .map_err(|e| e.to_string())?;

        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, i64>(5)?,
                ))
            })
            .map_err(|e| e.to_string())?;

        let now = Utc::now().timestamp();
        let mut hits = Vec::new();

        for row in rows {
            let (namespace, path, key, use_count, last_used_at, notes) =
                row.map_err(|e| e.to_string())?;

            let match_score = if groups.is_empty() {
                1.0
            } else {
                match score_complementary(&groups, &namespace, &path, &key) {
                    Some(s) => s,
                    None => continue,
                }
            };

            let recency = last_used_at
                .as_ref()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| {
                    let age_hours = ((now - dt.timestamp()).max(0) as f64) / 3600.0;
                    (48.0 - age_hours.min(48.0)) / 48.0 * 25.0
                })
                .unwrap_or(0.0);

            let score = match_score + (use_count as f64) * 8.0 + recency;
            hits.push(SearchHit {
                namespace: namespace.clone(),
                path: path.clone(),
                key,
                use_count,
                last_used_at,
                notes,
                score,
            });
        }

        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        hits.truncate(limit);
        Ok(hits)
    }
}

/// Space-separated terms are AND'd. Each term expands to aliases (OR).
/// Example: "infisi cb2" → (infisi|infisical) AND cb2 across namespace/path/key.
fn tokenize_query(query: &str) -> Vec<Vec<String>> {
    query
        .split(|c: char| c.is_whitespace() || c == ',' || c == '|')
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .map(|tok| expand_aliases(&tok))
        .collect()
}

fn expand_aliases(tok: &str) -> Vec<String> {
    let mut out = vec![tok.to_string()];
    let aliases: &[(&str, &[&str])] = &[
        ("infisi", &["infisical"]),
        ("infis", &["infisical"]),
        ("infiscal", &["infisical"]),
        ("bao", &["openbao", "vault"]),
        ("ob", &["openbao"]),
        ("cf", &["cloudflare"]),
        ("sb", &["supabase"]),
        ("pg", &["postgres", "postgresql"]),
        ("db", &["database", "postgres"]),
        ("dk", &["dokploy"]),
        ("dok", &["dokploy"]),
        ("vg", &["vetgroom"]),
    ];

    for (short, fulls) in aliases {
        let short_hit = tok == *short || (tok.len() >= 3 && short.starts_with(tok));
        for f in *fulls {
            if short_hit || (tok.len() >= 3 && f.starts_with(tok)) {
                out.push((*f).to_string());
            }
        }
    }

    out.sort();
    out.dedup();
    out
}

fn score_complementary(
    groups: &[Vec<String>],
    namespace: &str,
    path: &str,
    key: &str,
) -> Option<f64> {
    let mut total = 0.0;
    for group in groups {
        let mut best: Option<f64> = None;
        for alt in group {
            if let Some(s) = field_match_score(alt, namespace, path, key) {
                best = Some(best.map_or(s, |b| b.max(s)));
            }
        }
        match best {
            Some(s) => total += s,
            None => return None,
        }
    }
    Some(total + (groups.len() as f64) * 5.0)
}

fn field_match_score(needle: &str, namespace: &str, path: &str, key: &str) -> Option<f64> {
    let ns = namespace.to_lowercase();
    let p = path.to_lowercase();
    let k = key.to_lowercase();
    let hay = format!("{ns}/{p}/{k}");
    let segs: Vec<String> = hay
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();

    if k == needle {
        return Some(100.0);
    }
    if k.starts_with(needle) {
        return Some(88.0);
    }
    if k.contains(needle) {
        return Some(72.0);
    }
    if segs.iter().any(|s| s == needle) {
        return Some(70.0);
    }
    if segs.iter().any(|s| s.starts_with(needle)) {
        return Some(62.0);
    }
    if p.contains(needle) {
        return Some(55.0);
    }
    if ns.contains(needle) {
        return Some(48.0);
    }
    if hay.contains(needle) {
        return Some(40.0);
    }

    // Fuzzy: "infisi" as subsequence of "infisical"
    if needle.len() >= 3 {
        for seg in &segs {
            let compact = seg.replace('_', "");
            if is_subsequence(needle, seg) || is_subsequence(needle, &compact) {
                let ratio = needle.len() as f64 / seg.len().max(1) as f64;
                if ratio >= 0.4 {
                    return Some(36.0 + ratio * 20.0);
                }
            }
        }
    }
    None
}

fn is_subsequence(needle: &str, hay: &str) -> bool {
    let mut it = hay.chars();
    for nc in needle.chars() {
        loop {
            match it.next() {
                Some(hc) if hc == nc => break,
                Some(_) => continue,
                None => return false,
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `:memory:` keeps the tests off the machine's real index (and lets them
    /// run while the widget is open, holding the WAL on the real file).
    fn store() -> Store {
        Store::open(std::path::Path::new(":memory:")).expect("in-memory index")
    }

    fn refs(items: &[(&str, &str, &str)]) -> Vec<SecretRef> {
        items
            .iter()
            .map(|(namespace, path, key)| SecretRef {
                namespace: (*namespace).into(),
                path: (*path).into(),
                key: (*key).into(),
            })
            .collect()
    }

    fn keys(hits: &[SearchHit]) -> Vec<&str> {
        hits.iter().map(|h| h.key.as_str()).collect()
    }

    fn sample() -> Store {
        let s = store();
        s.replace_index(&refs(&[
            ("buenaidee", "projects/cb2/infisical", "DATABASE_URL"),
            ("buenaidee", "projects/cb2/infisical", "API_TOKEN"),
            ("dokploy", "apps/vetgroom", "POSTGRES_PASSWORD"),
        ]))
        .expect("index sample secrets");
        s
    }

    #[test]
    fn empty_query_lists_everything() {
        let s = sample();
        let hits = s.search("", 80).expect("search");
        assert_eq!(hits.len(), 3);
        assert_eq!(s.count().unwrap(), 3);
        assert!(s.last_sync().unwrap().is_some());
    }

    #[test]
    fn terms_are_and_ed_and_aliases_expand() {
        let s = sample();
        // "infisi" is an alias of "infisical" (in the path), "cb2" is another.
        let hits = s.search("infisi cb2", 80).expect("search");
        assert_eq!(keys(&hits), vec!["API_TOKEN", "DATABASE_URL"]);
        // AND, not OR: adding a term that no row satisfies empties the result.
        assert!(s
            .search("infisi cb2 vetgroom", 80)
            .expect("search")
            .is_empty());
        // A term that matches nothing at all is still empty, not an error.
        assert!(s.search("zzzz-nada", 80).expect("search").is_empty());
    }

    #[test]
    fn exact_key_wins_and_limit_is_honoured() {
        let s = sample();
        let hits = s.search("DATABASE_URL", 80).expect("search");
        assert_eq!(keys(&hits)[0], "DATABASE_URL");
        assert!(hits[0].score > 100.0, "an exact key hit scores highest");

        let one = s.search("", 1).expect("search");
        assert_eq!(one.len(), 1);
    }

    #[test]
    fn usage_survives_a_reindex_and_drives_the_default_order() {
        let s = sample();
        s.record_use_ref("dokploy", "apps/vetgroom", "POSTGRES_PASSWORD")
            .expect("record use");
        // A sync rewrites the whole table; usage must be carried over by
        // (namespace, path, key) — rowids are reassigned, references are not.
        s.replace_index(&refs(&[
            ("buenaidee", "projects/cb2/infisical", "DATABASE_URL"),
            ("buenaidee", "projects/cb2/infisical", "API_TOKEN"),
            ("dokploy", "apps/vetgroom", "POSTGRES_PASSWORD"),
        ]))
        .expect("reindex");

        let hits = s.search("", 80).expect("search");
        assert_eq!(keys(&hits)[0], "POSTGRES_PASSWORD");
        assert_eq!(hits[0].use_count, 1);
        assert!(hits[0].last_used_at.is_some());
    }

    #[test]
    fn a_secret_that_disappeared_loses_its_usage_rather_than_leaking_it() {
        let s = sample();
        s.record_use_ref("buenaidee", "projects/cb2/infisical", "API_TOKEN")
            .expect("record use");
        s.replace_index(&refs(&[("dokploy", "apps/vetgroom", "POSTGRES_PASSWORD")]))
            .expect("reindex without API_TOKEN");

        assert_eq!(s.count().unwrap(), 1);
        // Same key name, different path: usage must NOT follow the name.
        s.replace_index(&refs(&[("buenaidee", "otro/path", "API_TOKEN")]))
            .expect("reindex elsewhere");
        assert_eq!(s.search("", 80).unwrap()[0].use_count, 0);
    }
}
