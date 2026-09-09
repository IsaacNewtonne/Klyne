//! Single-run SQLite store. A held OS file lock excludes concurrent executors.
use crate::{agent::RunState, event_store::EventStore, types::Event};
use rusqlite::{Connection, OptionalExtension, params};
use std::{
    fs::{self, File, OpenOptions},
    io,
    path::Path,
};

pub struct SqliteEventStore {
    connection: Connection,
    _lease: File,
}

fn error(e: impl std::fmt::Display) -> io::Error {
    io::Error::other(e.to_string())
}

impl SqliteEventStore {
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent)?;
        }
        let mut lease_path = path.as_os_str().to_owned();
        lease_path.push(".lock");
        let lease = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(lease_path)?;
        lease.try_lock().map_err(error)?;
        let mut connection = Connection::open(path).map_err(error)?;
        connection
            .execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;")
            .map_err(error)?;
        let version: u32 = connection
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(error)?;
        if version > 1 {
            return Err(error("database schema is newer than this runtime"));
        }
        if version == 0 {
            let tx = connection.transaction().map_err(error)?;
            tx.execute_batch(include_str!("../migrations/001_initial.sql"))
                .map_err(error)?;
            tx.commit().map_err(error)?;
        }
        Ok(Self {
            connection,
            _lease: lease,
        })
    }

    pub fn events(&self) -> io::Result<Vec<Event>> {
        let mut query = self
            .connection
            .prepare("SELECT seq, kind, detail FROM events ORDER BY seq")
            .map_err(error)?;
        query
            .query_map([], |r| {
                Ok(Event {
                    seq: r
                        .get::<_, i64>(0)?
                        .try_into()
                        .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(0, -1))?,
                    kind: r.get(1)?,
                    detail: r.get(2)?,
                })
            })
            .map_err(error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(error)
    }
}

impl EventStore for SqliteEventStore {
    fn append(&mut self, kind: &str, detail: &str) -> io::Result<Event> {
        self.connection
            .execute(
                "INSERT INTO events(version, kind, detail) VALUES(1, ?1, ?2)",
                params![kind, detail],
            )
            .map_err(error)?;
        Ok(Event {
            seq: self.connection.last_insert_rowid() as u64,
            kind: kind.into(),
            detail: detail.into(),
        })
    }

    fn checkpoint(&mut self, state: &RunState) -> io::Result<()> {
        let payload = serde_json::to_string(state).map_err(error)?;
        let tx = self.connection.transaction().map_err(error)?;
        tx.execute(
            "INSERT INTO events(version, kind, detail) VALUES(1, 'CheckpointCreated', ?1)",
            [&state.objective.id],
        )
        .map_err(error)?;
        tx.execute("INSERT INTO checkpoints(id, event_seq, payload) VALUES(1, ?1, ?2) ON CONFLICT(id) DO UPDATE SET event_seq=excluded.event_seq, payload=excluded.payload", params![tx.last_insert_rowid(), payload]).map_err(error)?;
        tx.commit().map_err(error)
    }

    fn load(&mut self) -> io::Result<Option<RunState>> {
        let payload: Option<String> = self
            .connection
            .query_row("SELECT payload FROM checkpoints WHERE id=1", [], |r| {
                r.get(0)
            })
            .optional()
            .map_err(error)?;
        payload
            .map(|s| serde_json::from_str(&s).map_err(error))
            .transpose()
    }
}
