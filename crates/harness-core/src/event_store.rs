use crate::types::Event;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub trait EventStore: Send {
    fn append(&mut self, kind: &str, detail: &str) -> io::Result<Event>;
}

pub struct FileEventStore {
    path: PathBuf,
    next_seq: u64,
}

impl FileEventStore {
    pub fn open(path: impl Into<PathBuf>) -> io::Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() { fs::create_dir_all(parent)?; }
        let next_seq = match fs::read_to_string(&path) {
            Ok(s) => s.lines().count() as u64 + 1,
            Err(e) if e.kind() == io::ErrorKind::NotFound => 1,
            Err(e) => return Err(e),
        };
        Ok(Self { path, next_seq })
    }

    pub fn path(&self) -> &Path { &self.path }

    fn escape(input: &str) -> String {
        input.replace('\\', "\\\\").replace('\t', "\\t").replace('\n', "\\n")
    }
}

impl EventStore for FileEventStore {
    fn append(&mut self, kind: &str, detail: &str) -> io::Result<Event> {
        let event = Event { seq: self.next_seq, kind: kind.to_string(), detail: detail.to_string() };
        self.next_seq += 1;
        let mut file = OpenOptions::new().create(true).append(true).open(&self.path)?;
        writeln!(file, "{}\t{}\t{}", event.seq, Self::escape(&event.kind), Self::escape(&event.detail))?;
        file.flush()?;
        Ok(event)
    }
}
