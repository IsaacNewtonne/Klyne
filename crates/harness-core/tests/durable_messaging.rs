//! Synthetic destination: no personal apps, accounts, or outbound messages.
use harness_core::action_journal::{
    ActionJournal, DeliveryOutcome, MessageDestination, MessageIntent, Receipt, deliver,
};
use rusqlite::{Connection, params};
use std::{io, path::Path, process::Command};
fn err(e: impl std::fmt::Display) -> io::Error {
    io::Error::other(e.to_string())
}
struct Fixture {
    db: Connection,
    lose_ack: bool,
    wrong_recipient: bool,
}
impl Fixture {
    fn open(root: &Path) -> Self {
        let db = Connection::open(root.join("destination.sqlite3")).unwrap();
        db.execute_batch("PRAGMA synchronous=FULL;CREATE TABLE IF NOT EXISTS messages(n INTEGER PRIMARY KEY,action TEXT,recipient TEXT,body TEXT);CREATE TABLE IF NOT EXISTS app(state TEXT);INSERT INTO app SELECT 'ready' WHERE NOT EXISTS(SELECT 1 FROM app);").unwrap();
        Self {
            db,
            lose_ack: false,
            wrong_recipient: false,
        }
    }
    fn count(&self) -> i64 {
        self.db
            .query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))
            .unwrap()
    }
    fn state(&mut self, state: &str) {
        self.db.execute("UPDATE app SET state=?1", [state]).unwrap();
    }
}
impl MessageDestination for Fixture {
    fn ready(&mut self) -> io::Result<()> {
        let state: String = self
            .db
            .query_row("SELECT state FROM app", [], |r| r.get(0))
            .map_err(err)?;
        match state.as_str() {
            "ready" => Ok(()),
            "minimized" | "background" | "notice" => {
                self.state("ready");
                Ok(())
            }
            _ => Err(err("Destination requires authentication or user input")),
        }
    }
    fn send(&mut self, id: &str, intent: &MessageIntent) -> io::Result<()> {
        self.db
            .execute(
                "INSERT INTO messages(action,recipient,body) VALUES(?1,?2,?3)",
                params![
                    id,
                    if self.wrong_recipient {
                        "wrong recipient"
                    } else {
                        &intent.recipient
                    },
                    intent.body
                ],
            )
            .map_err(err)?;
        if self.lose_ack {
            Err(err("Connection lost after send"))
        } else {
            Ok(())
        }
    }
    fn lookup(&mut self, id: &str) -> io::Result<Vec<Receipt>> {
        let mut statement = self
            .db
            .prepare("SELECT n,recipient,body FROM messages WHERE action=?1")
            .map_err(err)?;
        statement
            .query_map([id], |r| {
                Ok(Receipt {
                    action_id: id.into(),
                    destination_id: r.get::<_, i64>(0)?.to_string(),
                    recipient: r.get(1)?,
                    body: r.get(2)?,
                })
            })
            .map_err(err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(err)
    }
}
fn intent() -> MessageIntent {
    MessageIntent {
        recipient: "fixture-recipient".into(),
        body: "Synthetic message only".into(),
    }
}

#[test]
fn missing_body_does_not_dispatch() {
    let root = tempfile::tempdir().unwrap();
    let mut journal = ActionJournal::open(&root.path().join("journal.db")).unwrap();
    let mut app = Fixture::open(root.path());
    assert_eq!(
        deliver(
            &mut journal,
            &mut app,
            "one",
            &MessageIntent {
                recipient: "Jodi".into(),
                body: "".into()
            }
        )
        .unwrap(),
        DeliveryOutcome::NeedsInput
    );
    assert_eq!(app.count(), 0);
}
#[test]
fn destination_state_and_receipts_determine_success() {
    for state in ["ready", "minimized", "background", "notice"] {
        let root = tempfile::tempdir().unwrap();
        let mut journal = ActionJournal::open(&root.path().join("journal.db")).unwrap();
        let mut app = Fixture::open(root.path());
        app.state(state);
        app.lose_ack = true;
        assert_eq!(
            deliver(&mut journal, &mut app, "one", &intent()).unwrap(),
            DeliveryOutcome::Verified
        );
        assert_eq!(
            deliver(&mut journal, &mut app, "one", &intent()).unwrap(),
            DeliveryOutcome::AlreadyVerified
        );
        assert_eq!(app.count(), 1);
    }
}
#[test]
fn wrong_recipient_and_duplicate_receipts_never_verify() {
    let root = tempfile::tempdir().unwrap();
    let mut journal = ActionJournal::open(&root.path().join("journal.db")).unwrap();
    let mut app = Fixture::open(root.path());
    app.wrong_recipient = true;
    assert_eq!(
        deliver(&mut journal, &mut app, "one", &intent()).unwrap(),
        DeliveryOutcome::Unknown
    );
    assert_eq!(
        deliver(&mut journal, &mut app, "one", &intent()).unwrap(),
        DeliveryOutcome::Unknown
    );
    assert_eq!(app.count(), 1);
    app.wrong_recipient = false;
    app.send("one", &intent()).unwrap();
    assert_eq!(
        deliver(&mut journal, &mut app, "one", &intent()).unwrap(),
        DeliveryOutcome::Unknown
    );
    assert_eq!(app.count(), 2);
}
#[test]
fn contract_cannot_change_and_auth_block_does_not_send() {
    let root = tempfile::tempdir().unwrap();
    let mut journal = ActionJournal::open(&root.path().join("journal.db")).unwrap();
    let mut app = Fixture::open(root.path());
    app.state("login");
    assert!(deliver(&mut journal, &mut app, "one", &intent()).is_err());
    assert_eq!(app.count(), 0);
    let changed = MessageIntent {
        body: "Different message".into(),
        ..intent()
    };
    assert!(deliver(&mut journal, &mut app, "one", &changed).is_err());
    assert_eq!(app.count(), 0);
    app.state("ready");
    assert_eq!(
        deliver(&mut journal, &mut app, "one", &intent()).unwrap(),
        DeliveryOutcome::Verified
    );
}
#[test]
fn actual_process_exit_at_every_effect_boundary_does_not_duplicate() {
    for phase in ["prepared", "dispatched", "sent", "verified"] {
        let root = tempfile::tempdir().unwrap();
        let status = Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", "crash_child"])
            .env("KLYNE_SYNTHETIC_ROOT", root.path())
            .env("KLYNE_SYNTHETIC_PHASE", phase)
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(23));
        let mut journal = ActionJournal::open(&root.path().join("journal.db")).unwrap();
        let mut app = Fixture::open(root.path());
        let result = deliver(&mut journal, &mut app, "one", &intent()).unwrap();
        match phase {
            "dispatched" => {
                assert_eq!(result, DeliveryOutcome::Unknown);
                assert_eq!(app.count(), 0);
            }
            "verified" => {
                assert_eq!(result, DeliveryOutcome::AlreadyVerified);
                assert_eq!(app.count(), 1);
            }
            _ => {
                assert_eq!(result, DeliveryOutcome::Verified);
                assert_eq!(app.count(), 1);
            }
        }
        let _ = deliver(&mut journal, &mut app, "one", &intent()).unwrap();
        assert!(app.count() <= 1);
    }
}
#[test]
#[ignore = "Only launched as an isolated crash fixture"]
fn crash_child() {
    let Ok(root) = std::env::var("KLYNE_SYNTHETIC_ROOT") else {
        return;
    };
    let phase = std::env::var("KLYNE_SYNTHETIC_PHASE").unwrap();
    let root = Path::new(&root);
    let mut journal = ActionJournal::open(&root.join("journal.db")).unwrap();
    let mut app = Fixture::open(root);
    journal.prepare("one", &intent()).unwrap();
    if phase == "prepared" {
        std::process::exit(23);
    }
    if phase == "verified" {
        deliver(&mut journal, &mut app, "one", &intent()).unwrap();
        std::process::exit(23);
    }
    journal.dispatched("one").unwrap();
    if phase == "dispatched" {
        std::process::exit(23);
    }
    app.send("one", &intent()).unwrap();
    std::process::exit(23);
}
