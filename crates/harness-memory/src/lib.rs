//! SQLite-backed memory with explained retrieval.
//!
//! Four record classes: project facts, episodic outcomes, reusable
//! procedures, and failure lessons. Every record carries confidence,
//! provenance, timestamps, and salience; retrieval scores transparently and
//! explains each inclusion, so callers can audit why context was selected.
//! Retention and consolidation keep the store bounded without silent loss:
//! merged records become auditable tombstones, never vanished rows.

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::io;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    Project,
    Episodic,
    Procedural,
    Failure,
}

impl MemoryKind {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::Episodic => "episodic",
            Self::Procedural => "procedural",
            Self::Failure => "failure",
        }
    }

    fn from_str(text: &str) -> Option<Self> {
        match text {
            "project" => Some(Self::Project),
            "episodic" => Some(Self::Episodic),
            "procedural" => Some(Self::Procedural),
            "failure" => Some(Self::Failure),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MemoryRecord {
    pub id: i64,
    pub kind: MemoryKind,
    pub content: String,
    /// 0.0..=1.0 estimated reliability.
    pub confidence: f64,
    /// Where it came from: run id, task name, operator note.
    pub provenance: String,
    pub created_at_ms: u64,
    /// 0.0..=1.0 retention weight; low-salience old rows are forgettable.
    pub salience: f64,
    /// Merge counter: how many duplicate observations consolidated here.
    pub reinforced: u64,
    /// Tombstone pointer after consolidation; superseded rows never retrieve.
    pub superseded_by: Option<i64>,
}

#[derive(Clone, Debug)]
pub struct NewMemory {
    pub kind: MemoryKind,
    pub content: String,
    pub confidence: f64,
    pub provenance: String,
    pub salience: f64,
}

impl NewMemory {
    pub fn new(
        kind: MemoryKind,
        content: impl Into<String>,
        confidence: f64,
        provenance: impl Into<String>,
        salience: f64,
    ) -> Self {
        Self {
            kind,
            content: content.into(),
            confidence: confidence.clamp(0.0, 1.0),
            provenance: provenance.into(),
            salience: salience.clamp(0.0, 1.0),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct MemoryQuery {
    pub kinds: Vec<MemoryKind>,
    /// Free-text terms; empty matches everything (filtered by kind/recency).
    pub terms: Vec<String>,
    pub limit: usize,
    /// Soft boost for memories from this provenance (e.g. same project).
    pub provenance_boost: Option<String>,
}

impl MemoryQuery {
    pub fn new(kinds: Vec<MemoryKind>, terms: Vec<String>, limit: usize) -> Self {
        Self {
            kinds,
            terms,
            limit: limit.max(1),
            provenance_boost: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ScoredMemory {
    pub record: MemoryRecord,
    pub score: f64,
    /// Human- and machine-readable account of the score, one line per
    /// contributing dimension, so selection stays auditable.
    pub reasons: Vec<String>,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn error(display: impl std::fmt::Display) -> io::Error {
    io::Error::other(display.to_string())
}

fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|token| token.len() > 1)
        .map(|token| token.to_ascii_lowercase())
        .collect()
}

pub struct MemoryStore {
    connection: Connection,
}

impl MemoryStore {
    pub fn open(path: impl AsRef<std::path::Path>) -> io::Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(path).map_err(error)?;
        connection
            .execute_batch(
                "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
                 CREATE TABLE IF NOT EXISTS memories(
                     id INTEGER PRIMARY KEY,
                     kind TEXT NOT NULL,
                     content TEXT NOT NULL,
                     confidence REAL NOT NULL,
                     provenance TEXT NOT NULL,
                     created_at_ms INTEGER NOT NULL,
                     salience REAL NOT NULL,
                     reinforced INTEGER NOT NULL DEFAULT 1,
                     superseded_by INTEGER NULL
                 );
                 CREATE INDEX IF NOT EXISTS memories_kind ON memories(kind);",
            )
            .map_err(error)?;
        Ok(Self { connection })
    }

    pub fn write(&mut self, memory: NewMemory) -> io::Result<i64> {
        self.connection
            .execute(
                "INSERT INTO memories(kind, content, confidence, provenance, created_at_ms, salience)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    memory.kind.as_str(),
                    memory.content,
                    memory.confidence,
                    memory.provenance,
                    now_ms() as i64,
                    memory.salience,
                ],
            )
            .map_err(error)?;
        Ok(self.connection.last_insert_rowid())
    }

    fn row_to_record(row: &rusqlite::Row) -> rusqlite::Result<MemoryRecord> {
        let kind: String = row.get(1)?;
        Ok(MemoryRecord {
            id: row.get(0)?,
            kind: MemoryKind::from_str(&kind).ok_or(rusqlite::Error::InvalidColumnType(
                1,
                "kind".into(),
                rusqlite::types::Type::Text,
            ))?,
            content: row.get(2)?,
            confidence: row.get(3)?,
            provenance: row.get(4)?,
            created_at_ms: row.get::<_, i64>(5)? as u64,
            salience: row.get(6)?,
            reinforced: row.get::<_, i64>(7)? as u64,
            superseded_by: row.get(8)?,
        })
    }

    pub fn get(&self, id: i64) -> io::Result<Option<MemoryRecord>> {
        self.connection
            .query_row(
                "SELECT * FROM memories WHERE id=?1",
                [id],
                Self::row_to_record,
            )
            .optional()
            .map_err(error)
    }

    pub fn count(&self) -> io::Result<u64> {
        self.connection
            .query_row("SELECT COUNT(*) FROM memories", [], |row| {
                row.get::<_, i64>(0)
            })
            .map(|count| count as u64)
            .map_err(error)
    }

    /// Retrieve live (non-superseded) memories of the requested kinds,
    /// scored transparently. Score dimensions: term overlap, confidence,
    /// recency (30-day linear decay floor 0.25), reinforcement, and an
    /// optional provenance boost. Nothing is silently dropped: every
    /// returned item carries its reasons.
    pub fn retrieve(&self, query: &MemoryQuery) -> io::Result<Vec<ScoredMemory>> {
        let kinds: Vec<String> = query
            .kinds
            .iter()
            .map(|kind| kind.as_str().to_string())
            .collect();
        let placeholders = kinds.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!(
            "SELECT * FROM memories WHERE kind IN ({placeholders}) AND superseded_by IS NULL"
        );
        let mut statement = self.connection.prepare(&sql).map_err(error)?;
        let terms: Vec<String> = query
            .terms
            .iter()
            .map(|term| term.to_ascii_lowercase())
            .collect();
        let now = now_ms();
        let rows = statement
            .query_map(
                rusqlite::params_from_iter(kinds.iter()),
                Self::row_to_record,
            )
            .map_err(error)?;
        let mut scored = Vec::new();
        for record in rows {
            let record = record.map_err(error)?;
            let tokens = tokenize(&record.content);
            let hits = terms
                .iter()
                .filter(|term| tokens.iter().any(|token| token == *term))
                .count();
            if !terms.is_empty() && hits == 0 {
                continue;
            }
            let mut score = 0.0;
            let mut reasons = Vec::new();
            if !terms.is_empty() {
                let overlap = hits as f64 / terms.len() as f64;
                score += 4.0 * overlap;
                reasons.push(format!(
                    "term overlap {hits}/{} (+{:.2})",
                    terms.len(),
                    4.0 * overlap
                ));
            }
            score += 2.0 * record.confidence;
            reasons.push(format!(
                "confidence {:.2} (+{:.2})",
                record.confidence,
                2.0 * record.confidence
            ));
            let age_days = now.saturating_sub(record.created_at_ms) as f64 / 86_400_000.0;
            let recency = (1.0 - age_days / 30.0).clamp(0.25, 1.0);
            score += recency;
            reasons.push(format!("recency {age_days:.1}d (+{recency:.2})"));
            let reinforcement = (record.reinforced as f64).ln_1p().min(1.0);
            score += reinforcement;
            reasons.push(format!(
                "reinforced x{} (+{reinforcement:.2})",
                record.reinforced
            ));
            if let Some(boost) = &query.provenance_boost
                && record.provenance == *boost
            {
                score += 1.0;
                reasons.push("provenance match (+1.00)".into());
            }
            scored.push(ScoredMemory {
                record,
                score,
                reasons,
            });
        }
        scored.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        scored.truncate(query.limit);
        Ok(scored)
    }

    /// Point an outdated record at its replacement. Superseded rows stay in
    /// the database as tombstones but never retrieve.
    pub fn supersede(&mut self, old_id: i64, new_id: i64) -> io::Result<()> {
        if old_id == new_id {
            return Err(io::Error::other("a memory cannot supersede itself"));
        }
        let changed = self
            .connection
            .execute(
                "UPDATE memories SET superseded_by=?1 WHERE id=?2 AND superseded_by IS NULL",
                params![new_id, old_id],
            )
            .map_err(error)?;
        if changed == 0 {
            return Err(io::Error::other(
                "nothing superseded: unknown or already-superseded id",
            ));
        }
        Ok(())
    }

    /// Merge exact-duplicate contents within each kind: the earliest row
    /// keeps the maximum confidence and absorbs reinforcement counts, the
    /// rest become tombstones. Returns the number of rows superseded.
    pub fn consolidate(&mut self) -> io::Result<u64> {
        let mut statement = self
            .connection
            .prepare("SELECT id, kind, content, confidence, reinforced FROM memories WHERE superseded_by IS NULL ORDER BY id")
            .map_err(error)?;
        let rows: Vec<(i64, String, String, f64, i64)> = statement
            .query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            })
            .map_err(error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(error)?;
        drop(statement);
        use std::collections::BTreeMap;
        type DuplicateGroup = Vec<(i64, f64, i64)>;
        let mut groups: BTreeMap<(String, String), DuplicateGroup> = BTreeMap::new();
        for (id, kind, content, confidence, reinforced) in rows {
            groups
                .entry((kind, content))
                .or_default()
                .push((id, confidence, reinforced));
        }
        let mut superseded = 0u64;
        for (_, members) in groups.iter().filter(|(_, members)| members.len() > 1) {
            let keeper = members[0].0;
            let best: f64 = members.iter().map(|member| member.1).fold(0.0, f64::max);
            let total: i64 = members.iter().map(|member| member.2).sum();
            self.connection
                .execute(
                    "UPDATE memories SET confidence=?1, reinforced=?2 WHERE id=?3",
                    params![best, total, keeper],
                )
                .map_err(error)?;
            for (id, _, _) in &members[1..] {
                self.connection
                    .execute(
                        "UPDATE memories SET superseded_by=?1 WHERE id=?2",
                        params![keeper, id],
                    )
                    .map_err(error)?;
                superseded += 1;
            }
        }
        Ok(superseded)
    }

    /// Forget rows older than `older_than_ms` whose salience is below
    /// `keep_salience_at_least`. High-salience rows survive regardless of
    /// age; tombstones older than the cutoff are removed. Returns deletions.
    pub fn forget(&mut self, older_than_ms: u64, keep_salience_at_least: f64) -> io::Result<u64> {
        self.forget_as_of(now_ms(), older_than_ms, keep_salience_at_least)
    }

    /// Deterministic variant of [`MemoryStore::forget`] with an explicit
    /// clock, so retention policies are testable without sleeping.
    pub fn forget_as_of(
        &mut self,
        now_ms: u64,
        older_than_ms: u64,
        keep_salience_at_least: f64,
    ) -> io::Result<u64> {
        let cutoff = (now_ms as i64).saturating_sub(older_than_ms as i64);
        let deleted = self
            .connection
            .execute(
                "DELETE FROM memories WHERE created_at_ms < ?1 AND (salience < ?2 OR superseded_by IS NOT NULL)",
                params![cutoff, keep_salience_at_least],
            )
            .map_err(error)? as u64;
        Ok(deleted)
    }
}
