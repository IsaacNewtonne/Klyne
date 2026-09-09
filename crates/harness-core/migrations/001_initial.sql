CREATE TABLE events (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    version INTEGER NOT NULL CHECK(version = 1),
    kind TEXT NOT NULL,
    detail TEXT NOT NULL
);
CREATE TRIGGER events_no_update BEFORE UPDATE ON events BEGIN
    SELECT RAISE(ABORT, 'events are append only');
END;
CREATE TRIGGER events_no_delete BEFORE DELETE ON events BEGIN
    SELECT RAISE(ABORT, 'events are append only');
END;
CREATE TABLE checkpoints (
    id INTEGER PRIMARY KEY CHECK(id = 1),
    event_seq INTEGER NOT NULL REFERENCES events(seq),
    payload TEXT NOT NULL
);
PRAGMA user_version = 1;
