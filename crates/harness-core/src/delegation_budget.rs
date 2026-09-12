//! Durable parent-owned reservations for concurrent child runs. Failed or crashed
//! children retain their reservation until their actual usage is reconciled.
use rusqlite::{Connection, params};
use std::{
    io,
    path::{Path, PathBuf},
    time::Duration,
};
fn error(e: impl std::fmt::Display) -> io::Error {
    io::Error::other(e.to_string())
}
pub struct DelegationBudget {
    path: PathBuf,
}
pub struct Reservation {
    path: PathBuf,
    id: String,
    amount: u64,
}
impl DelegationBudget {
    pub fn open(path: impl AsRef<Path>, limit: u64) -> io::Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let budget = Self { path: path.into() };
        let mut db = budget.connection()?;
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(error)?;
        tx.execute_batch("CREATE TABLE IF NOT EXISTS capacity(id INTEGER PRIMARY KEY, amount INTEGER NOT NULL);CREATE TABLE IF NOT EXISTS reservations(id TEXT PRIMARY KEY, amount INTEGER NOT NULL, settled INTEGER NOT NULL DEFAULT 0);").map_err(error)?;
        let limit = i64::try_from(limit).map_err(error)?;
        tx.execute("INSERT OR IGNORE INTO capacity VALUES(1,?1)", [limit])
            .map_err(error)?;
        let existing: i64 = tx
            .query_row("SELECT amount FROM capacity WHERE id=1", [], |r| r.get(0))
            .map_err(error)?;
        if existing != limit {
            return Err(error(
                "Persisted delegation capacity cannot be silently replaced",
            ));
        }
        tx.commit().map_err(error)?;
        Ok(budget)
    }
    fn connection(&self) -> io::Result<Connection> {
        let db = Connection::open(&self.path).map_err(error)?;
        db.busy_timeout(Duration::from_secs(5)).map_err(error)?;
        Ok(db)
    }
    pub fn reserve(&self, id: &str, amount: u64) -> io::Result<Reservation> {
        if id.is_empty()
            || id.len() > 128
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(error("Invalid child reservation id"));
        }
        let mut db = self.connection()?;
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(error)?;
        let available:i64=tx.query_row("SELECT amount-(SELECT COALESCE(SUM(amount),0) FROM reservations) FROM capacity WHERE id=1",[],|r|r.get(0)).map_err(error)?;
        let requested = i64::try_from(amount).map_err(error)?;
        if requested > available {
            return Err(error("Parent delegation budget exhausted"));
        }
        tx.execute(
            "INSERT INTO reservations(id,amount) VALUES(?1,?2)",
            params![id, requested],
        )
        .map_err(error)?;
        tx.commit().map_err(error)?;
        Ok(Reservation {
            path: self.path.clone(),
            id: id.into(),
            amount,
        })
    }
    pub fn remaining(&self) -> io::Result<u64> {
        let db = self.connection()?;
        let n:i64=db.query_row("SELECT amount-(SELECT COALESCE(SUM(amount),0) FROM reservations) FROM capacity WHERE id=1",[],|r|r.get(0)).map_err(error)?;
        u64::try_from(n).map_err(error)
    }
}
impl Reservation {
    pub fn settle(self, used: u64) -> io::Result<()> {
        if used > self.amount {
            return Err(error("Child usage exceeded its reservation"));
        }
        let db = Connection::open(&self.path).map_err(error)?;
        db.busy_timeout(Duration::from_secs(5)).map_err(error)?;
        db.execute(
            "UPDATE reservations SET amount=?2,settled=1 WHERE id=?1 AND settled=0",
            params![self.id, i64::try_from(used).map_err(error)?],
        )
        .map_err(error)?;
        if db.changes() != 1 {
            return Err(error("Reservation already settled or missing"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parallel_children_cannot_reuse_the_same_capacity() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("budget.sqlite3");
        let pool = DelegationBudget::open(&path, 10).unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let handles = (0..2)
            .map(|n| {
                let path = path.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    let pool = DelegationBudget::open(path, 10).unwrap();
                    barrier.wait();
                    pool.reserve(&format!("child-{n}"), 6)
                })
            })
            .collect::<Vec<_>>();
        let reservations = handles
            .into_iter()
            .filter_map(|h| h.join().unwrap().ok())
            .collect::<Vec<_>>();
        assert_eq!(reservations.len(), 1);
        assert_eq!(pool.remaining().unwrap(), 4);
        reservations.into_iter().next().unwrap().settle(2).unwrap();
        assert_eq!(
            DelegationBudget::open(&path, 10)
                .unwrap()
                .remaining()
                .unwrap(),
            8
        );
        assert!(DelegationBudget::open(&path, 20).is_err());
    }
}
