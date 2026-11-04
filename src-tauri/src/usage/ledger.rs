//! Persistent record of counted Codex requests.
//!
//! - `codex-requests.jsonl`: one counted request per line, append-only. Readers
//!   dedupe by request key, so a request appended twice (for example after a
//!   crash between the two writes below) is still counted once, and usage stays
//!   available after Codex session files are deleted.
//! - `codex-files.json`: size + mtime of each rollout already recorded, written
//!   atomically. Unchanged rollouts are not parsed again; changed or deferred
//!   ones are, and only their new request keys are appended.
//!
//! Both files are a cache of data derived from Codex's own logs: a damaged
//! line or document is ignored and rebuilt from the logs still on disk.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use atomic_write_file::AtomicWriteFile;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::codex::{CodexScan, CodexUsageEvent, DeltaTokens, FileStamp};

const FILES_DOCUMENT_VERSION: u32 = 1;
pub const REQUESTS_FILE: &str = "codex-requests.jsonl";
pub const FILES_FILE: &str = "codex-files.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RequestRecord {
    key: String,
    thread_id: String,
    model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    timestamp: Option<DateTime<Utc>>,
    input: u64,
    cached_input: u64,
    #[serde(default)]
    cache_write_input: u64,
    output: u64,
}

impl From<&CodexUsageEvent> for RequestRecord {
    fn from(event: &CodexUsageEvent) -> Self {
        Self {
            key: event.request_key.clone(),
            thread_id: event.thread_id.clone(),
            model: event.model.clone(),
            timestamp: event.timestamp,
            input: event.tokens.input,
            cached_input: event.tokens.cached_input,
            cache_write_input: event.tokens.cache_write_input,
            output: event.tokens.output,
        }
    }
}

impl From<RequestRecord> for CodexUsageEvent {
    fn from(record: RequestRecord) -> Self {
        Self {
            request_key: record.key,
            thread_id: record.thread_id,
            model: record.model,
            timestamp: record.timestamp,
            tokens: DeltaTokens {
                input: record.input,
                cached_input: record.cached_input,
                cache_write_input: record.cache_write_input,
                output: record.output,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileEntry {
    stamp: FileStamp,
    #[serde(default)]
    replayed_events: u64,
    #[serde(default)]
    duplicate_events: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FilesDocument {
    version: u32,
    files: BTreeMap<String, FileEntry>,
}

#[derive(Debug)]
pub struct CodexLedger {
    dir: PathBuf,
    files: BTreeMap<String, FileEntry>,
    events: Vec<CodexUsageEvent>,
    keys: HashSet<String>,
}

fn path_key(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

impl CodexLedger {
    /// Load the ledger from `dir`; missing files start an empty ledger.
    pub fn open(dir: impl Into<PathBuf>) -> io::Result<Self> {
        let dir = dir.into();
        let files = match fs::read(dir.join(FILES_FILE)) {
            Ok(bytes) => serde_json::from_slice::<FilesDocument>(&bytes)
                .ok()
                .filter(|document| document.version == FILES_DOCUMENT_VERSION)
                .map(|document| document.files)
                .unwrap_or_default(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => BTreeMap::new(),
            Err(error) => return Err(error),
        };
        let mut ledger = Self {
            dir,
            files,
            events: Vec::new(),
            keys: HashSet::new(),
        };
        match fs::read(ledger.requests_path()) {
            Ok(bytes) => {
                for line in bytes.split(|byte| *byte == b'\n') {
                    // Skip blank and damaged lines (e.g. a write cut short).
                    let Ok(record) = serde_json::from_slice::<RequestRecord>(line) else {
                        continue;
                    };
                    if ledger.keys.insert(record.key.clone()) {
                        ledger.events.push(record.into());
                    }
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        Ok(ledger)
    }

    fn requests_path(&self) -> PathBuf {
        self.dir.join(REQUESTS_FILE)
    }

    /// Whether `path` was recorded with exactly this stamp.
    pub fn is_current(&self, path: &Path, stamp: &FileStamp) -> bool {
        self.files
            .get(&path_key(path))
            .is_some_and(|entry| &entry.stamp == stamp)
    }

    pub fn events(&self) -> &[CodexUsageEvent] {
        &self.events
    }

    pub fn replayed_events(&self) -> u64 {
        self.files.values().map(|entry| entry.replayed_events).sum()
    }

    pub fn duplicate_events(&self) -> u64 {
        self.files
            .values()
            .map(|entry| entry.duplicate_events)
            .sum()
    }

    /// Append the new requests of `scan` and remember the rollouts it parsed.
    pub fn record(&mut self, scan: &CodexScan) -> io::Result<()> {
        let new_events = scan
            .events
            .iter()
            .filter(|event| !self.keys.contains(&event.request_key))
            .collect::<Vec<_>>();
        if !new_events.is_empty() {
            let mut lines = Vec::new();
            for event in &new_events {
                serde_json::to_writer(&mut lines, &RequestRecord::from(*event))
                    .map_err(io::Error::other)?;
                lines.push(b'\n');
            }
            self.append_requests(&lines)?;
            for event in new_events {
                self.keys.insert(event.request_key.clone());
                self.events.push(event.clone());
            }
        }

        let mut changed = false;
        for file in &scan.files {
            let key = path_key(&file.path);
            match (file.deferred, file.stamp) {
                (false, Some(stamp)) => {
                    let entry = FileEntry {
                        stamp,
                        replayed_events: file.replayed_events as u64,
                        duplicate_events: file.duplicate_events as u64,
                    };
                    changed |= self.files.insert(key, entry.clone()).as_ref() != Some(&entry);
                }
                // Deferred or unreadable: parse it again next time.
                _ => changed |= self.files.remove(&key).is_some(),
            }
        }
        if changed {
            self.write_files()?;
        }
        Ok(())
    }

    fn append_requests(&self, lines: &[u8]) -> io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        let mut file = fs::OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .open(self.requests_path())?;
        // Start on a fresh line if an earlier write was cut short.
        let len = file.metadata()?.len();
        let mut prefix = Vec::new();
        if len > 0 {
            let mut last = [0u8; 1];
            file.seek(SeekFrom::Start(len - 1))?;
            file.read_exact(&mut last)?;
            if last[0] != b'\n' {
                prefix.push(b'\n');
            }
        }
        prefix.extend_from_slice(lines);
        file.write_all(&prefix)?;
        file.sync_data()
    }

    fn write_files(&self) -> io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        let bytes = serde_json::to_vec_pretty(&FilesDocument {
            version: FILES_DOCUMENT_VERSION,
            files: self.files.clone(),
        })
        .map_err(io::Error::other)?;
        let mut file = AtomicWriteFile::open(self.dir.join(FILES_FILE))?;
        file.write_all(&bytes)?;
        file.commit()
    }
}

#[cfg(test)]
mod tests {
    use super::super::codex::ScannedFile;
    use super::*;
    use tempfile::tempdir;

    fn event(key: &str, input: u64) -> CodexUsageEvent {
        CodexUsageEvent {
            request_key: key.into(),
            thread_id: "t".into(),
            model: "gpt-5.4".into(),
            timestamp: Some(Utc::now()),
            tokens: DeltaTokens {
                input,
                cached_input: 0,
                cache_write_input: 0,
                output: 1,
            },
        }
    }

    fn scanned(path: &str, size: u64, deferred: bool) -> ScannedFile {
        ScannedFile {
            path: PathBuf::from(path),
            stamp: Some(FileStamp {
                modified_ns: Some(1),
                size,
            }),
            deferred,
            replayed_events: 2,
            duplicate_events: 0,
        }
    }

    #[test]
    fn records_survive_reload_and_dedupe_by_key() {
        let dir = tempdir().unwrap();
        let mut ledger = CodexLedger::open(dir.path()).unwrap();
        let scan = CodexScan {
            files: vec![scanned("/a.jsonl", 10, false), scanned("/b.jsonl", 5, true)],
            events: vec![event("a:1", 100), event("a:2", 200)],
            ..CodexScan::default()
        };
        ledger.record(&scan).unwrap();
        ledger.record(&scan).unwrap();
        assert_eq!(ledger.events().len(), 2);

        let reloaded = CodexLedger::open(dir.path()).unwrap();
        assert_eq!(reloaded.events(), ledger.events());
        let stamp = FileStamp {
            modified_ns: Some(1),
            size: 10,
        };
        assert!(reloaded.is_current(Path::new("/a.jsonl"), &stamp));
        assert!(!reloaded.is_current(Path::new("/a.jsonl"), &FileStamp { size: 11, ..stamp }));
        assert!(!reloaded.is_current(Path::new("/b.jsonl"), &FileStamp { size: 5, ..stamp }));
        assert_eq!(reloaded.replayed_events(), 2);
    }

    #[test]
    fn damaged_lines_and_documents_are_ignored() {
        let dir = tempdir().unwrap();
        let mut ledger = CodexLedger::open(dir.path()).unwrap();
        ledger
            .record(&CodexScan {
                files: vec![scanned("/a.jsonl", 10, false)],
                events: vec![event("a:1", 100)],
                ..CodexScan::default()
            })
            .unwrap();
        // A write cut short leaves a partial line; the files document is corrupt.
        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(dir.path().join(REQUESTS_FILE))
            .unwrap();
        file.write_all(b"{\"key\":\"a:2\",\"thr").unwrap();
        fs::write(dir.path().join(FILES_FILE), b"{not json").unwrap();

        let mut ledger = CodexLedger::open(dir.path()).unwrap();
        assert_eq!(ledger.events().len(), 1);
        assert!(!ledger.is_current(
            Path::new("/a.jsonl"),
            &FileStamp {
                modified_ns: Some(1),
                size: 10
            }
        ));
        ledger
            .record(&CodexScan {
                events: vec![event("a:1", 100), event("a:2", 300)],
                ..CodexScan::default()
            })
            .unwrap();
        let reloaded = CodexLedger::open(dir.path()).unwrap();
        assert_eq!(
            reloaded
                .events()
                .iter()
                .map(|event| event.tokens.input)
                .collect::<Vec<_>>(),
            vec![100, 300]
        );
    }
}
