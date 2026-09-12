//! Durable external-effect intent. A dispatched effect is never assumed safe to replay.
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    io,
    path::Path,
    time::Duration,
};
fn error(e: impl std::fmt::Display) -> io::Error {
    io::Error::other(e.to_string())
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageIntent {
    pub recipient: String,
    pub body: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EffectState {
    Prepared,
    Dispatched,
    Verified,
}
pub struct ActionJournal {
    db: Connection,
    _lease: File,
}
impl ActionJournal {
    pub fn open(path: &Path) -> io::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut lock = path.as_os_str().to_owned();
        lock.push(".lock");
        let lease = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(lock)?;
        lease.try_lock().map_err(error)?;
        let db = Connection::open(path).map_err(error)?;
        db.busy_timeout(Duration::from_secs(2)).map_err(error)?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
            CREATE TABLE IF NOT EXISTS effects(id TEXT PRIMARY KEY,intent TEXT NOT NULL,state TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS effect_events(seq INTEGER PRIMARY KEY,id TEXT NOT NULL,state TEXT NOT NULL,evidence TEXT NOT NULL);").map_err(error)?;
        Ok(Self { db, _lease: lease })
    }
    pub fn prepare(&mut self, id: &str, intent: &MessageIntent) -> io::Result<EffectState> {
        if id.is_empty()
            || id.len() > 128
            || intent.recipient.trim().is_empty()
            || intent.recipient.len() > 512
            || intent.body.trim().is_empty()
            || intent.body.len() > 8192
        {
            return Err(error(
                "Action ID, recipient, and message body are required and must be bounded",
            ));
        }
        let encoded = serde_json::to_string(intent).map_err(error)?;
        let tx = self.db.transaction().map_err(error)?;
        let existing: Option<(String, String)> = tx
            .query_row("SELECT intent,state FROM effects WHERE id=?1", [id], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()
            .map_err(error)?;
        let state = if let Some((saved, state)) = existing {
            if saved != encoded {
                return Err(error("Action ID already belongs to a different intent"));
            }
            parse_state(&state)?
        } else {
            tx.execute(
                "INSERT INTO effects VALUES(?1,?2,'prepared')",
                params![id, encoded],
            )
            .map_err(error)?;
            tx.execute(
                "INSERT INTO effect_events(id,state,evidence) VALUES(?1,'prepared','')",
                [id],
            )
            .map_err(error)?;
            EffectState::Prepared
        };
        tx.commit().map_err(error)?;
        Ok(state)
    }
    pub fn dispatched(&mut self, id: &str) -> io::Result<()> {
        self.transition(id, "prepared", "dispatched", "")
    }
    fn verified(&mut self, id: &str, receipt: &Receipt) -> io::Result<()> {
        self.transition(
            id,
            "dispatched",
            "verified",
            &serde_json::to_string(receipt).map_err(error)?,
        )
    }
    fn transition(&mut self, id: &str, from: &str, to: &str, evidence: &str) -> io::Result<()> {
        let tx = self.db.transaction().map_err(error)?;
        if tx
            .execute(
                "UPDATE effects SET state=?1 WHERE id=?2 AND state=?3",
                params![to, id, from],
            )
            .map_err(error)?
            != 1
        {
            return Err(error("Invalid effect transition"));
        }
        tx.execute(
            "INSERT INTO effect_events(id,state,evidence) VALUES(?1,?2,?3)",
            params![id, to, evidence],
        )
        .map_err(error)?;
        tx.commit().map_err(error)
    }
}
fn parse_state(state: &str) -> io::Result<EffectState> {
    match state {
        "prepared" => Ok(EffectState::Prepared),
        "dispatched" => Ok(EffectState::Dispatched),
        "verified" => Ok(EffectState::Verified),
        _ => Err(error("Unsupported action state")),
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Receipt {
    pub action_id: String,
    pub recipient: String,
    pub body: String,
    pub destination_id: String,
}
pub trait MessageDestination {
    /// Restore/focus/unblock only within the already authorized task scope.
    fn ready(&mut self) -> io::Result<()>;
    fn send(&mut self, id: &str, intent: &MessageIntent) -> io::Result<()>;
    /// A destination-state lookup, not a model's assertion of success.
    fn lookup(&mut self, id: &str) -> io::Result<Vec<Receipt>>;
}
#[derive(Debug, PartialEq, Eq)]
pub enum DeliveryOutcome {
    Verified,
    AlreadyVerified,
    Unknown,
    NeedsInput,
}
pub fn deliver(
    journal: &mut ActionJournal,
    destination: &mut impl MessageDestination,
    id: &str,
    intent: &MessageIntent,
) -> io::Result<DeliveryOutcome> {
    if intent.recipient.trim().is_empty() || intent.body.trim().is_empty() {
        return Ok(DeliveryOutcome::NeedsInput);
    }
    let state = journal.prepare(id, intent)?;
    if state == EffectState::Verified {
        return Ok(DeliveryOutcome::AlreadyVerified);
    }
    if state == EffectState::Prepared {
        destination.ready()?;
        journal.dispatched(id)?;
        // No automatic repeat even if the call reports failure after applying its effect.
        let _ = destination.send(id, intent);
    }
    let receipts = match destination.lookup(id) {
        Ok(receipts) => receipts,
        Err(_) => return Ok(DeliveryOutcome::Unknown),
    };
    if receipts.len() == 1 {
        let receipt = &receipts[0];
        if receipt.action_id == id
            && receipt.recipient == intent.recipient
            && receipt.body == intent.body
            && !receipt.destination_id.is_empty()
        {
            journal.verified(id, receipt)?;
            return Ok(DeliveryOutcome::Verified);
        }
    }
    Ok(DeliveryOutcome::Unknown)
}
