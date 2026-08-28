//! Atomic settings/state storage and per-run JSONL logs.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::SystemTime;

use atomic_write_file::AtomicWriteFile;
use chrono::{DateTime, Utc};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use thiserror::Error;

use crate::domain::{ComponentId, ProviderId, Run, RunId, StrategyId, Tool, ToolId};

pub const SETTINGS_SCHEMA_VERSION: u32 = 1;
pub const STATE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPaths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub log_dir: PathBuf,
}

impl AppPaths {
    pub fn new(
        config_dir: impl Into<PathBuf>,
        data_dir: impl Into<PathBuf>,
        log_dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            config_dir: config_dir.into(),
            data_dir: data_dir.into(),
            log_dir: log_dir.into(),
        }
    }

    pub fn from_tauri<R: tauri::Runtime>(
        resolver: &tauri::path::PathResolver<R>,
    ) -> Result<Self, StoreError> {
        let config_dir = resolver
            .app_config_dir()
            .map_err(|error| StoreError::path_resolution("app_config_dir", error.to_string()))?;
        let data_dir = resolver
            .app_data_dir()
            .map_err(|error| StoreError::path_resolution("app_data_dir", error.to_string()))?;
        let log_dir = resolver
            .app_log_dir()
            .map_err(|error| StoreError::path_resolution("app_log_dir", error.to_string()))?;
        Ok(Self::new(config_dir, data_dir, log_dir))
    }

    #[cfg(all(debug_assertions, feature = "dev-http"))]
    pub fn from_desktop_identifier(identifier: &str) -> Result<Self, StoreError> {
        let config_dir = dirs::config_dir()
            .ok_or_else(|| {
                StoreError::path_resolution("config_dir", "directory is unavailable".into())
            })?
            .join(identifier);
        let data_dir = dirs::data_dir()
            .ok_or_else(|| {
                StoreError::path_resolution("data_dir", "directory is unavailable".into())
            })?
            .join(identifier);

        #[cfg(target_os = "macos")]
        let log_dir = dirs::home_dir()
            .ok_or_else(|| {
                StoreError::path_resolution("home_dir", "directory is unavailable".into())
            })?
            .join("Library/Logs")
            .join(identifier);
        #[cfg(not(target_os = "macos"))]
        let log_dir = dirs::data_local_dir()
            .ok_or_else(|| {
                StoreError::path_resolution("local_data_dir", "directory is unavailable".into())
            })?
            .join(identifier)
            .join("logs");

        Ok(Self::new(config_dir, data_dir, log_dir))
    }

    pub fn settings_file(&self) -> PathBuf {
        self.config_dir.join("settings.toml")
    }

    pub fn state_file(&self) -> PathBuf {
        self.data_dir.join("state.json")
    }

    pub fn runs_dir(&self) -> PathBuf {
        self.log_dir.join("runs")
    }

    pub fn manifests_dir(&self) -> PathBuf {
        self.config_dir.join("tools.d")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Revision(String);

impl Revision {
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self(blake3::hash(bytes).to_hex().to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for Revision {
    type Err = InvalidRevision;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            Ok(Self(value.to_owned()))
        } else {
            Err(InvalidRevision)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("revision must be a 64-character lowercase hexadecimal hash")]
pub struct InvalidRevision;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Versioned<T> {
    pub revision: Revision,
    pub value: T,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogRetentionPolicy {
    pub max_files: usize,
    pub max_total_bytes: u64,
}

impl Default for LogRetentionPolicy {
    fn default() -> Self {
        Self {
            max_files: 100,
            max_total_bytes: 50 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    #[serde(default)]
    pub provider_enabled: BTreeMap<ProviderId, bool>,
    #[serde(default = "default_timeout_seconds")]
    pub default_timeout_seconds: u64,
    #[serde(default)]
    pub log_retention: LogRetentionPolicy,
    #[serde(default)]
    pub executable_overrides: BTreeMap<String, PathBuf>,
    #[serde(default)]
    pub component_strategies: BTreeMap<ToolId, BTreeMap<ComponentId, StrategyId>>,
}

const fn default_timeout_seconds() -> u64 {
    600
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            provider_enabled: BTreeMap::new(),
            default_timeout_seconds: default_timeout_seconds(),
            log_retention: LogRetentionPolicy::default(),
            executable_overrides: BTreeMap::new(),
            component_strategies: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CachedState {
    #[serde(default)]
    pub tools: Vec<Tool>,
    #[serde(default)]
    pub runs: Vec<Run>,
    pub last_refresh_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SettingsDocument {
    schema_version: u32,
    settings: Settings,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StateDocument {
    schema_version: u32,
    state: CachedState,
}

#[derive(Debug, Clone)]
pub struct PersistenceStore {
    paths: AppPaths,
}

impl PersistenceStore {
    pub fn new(paths: AppPaths) -> Self {
        Self { paths }
    }

    pub fn paths(&self) -> &AppPaths {
        &self.paths
    }

    pub fn ensure_directories(&self) -> Result<(), StoreError> {
        for path in [
            &self.paths.config_dir,
            &self.paths.data_dir,
            &self.paths.log_dir,
            &self.paths.runs_dir(),
            &self.paths.manifests_dir(),
        ] {
            fs::create_dir_all(path).map_err(|error| StoreError::io(path, error))?;
        }
        Ok(())
    }

    pub fn user_manifest_path(&self, file_name: &str) -> Result<PathBuf, StoreError> {
        let path = Path::new(file_name);
        let safe = path.components().count() == 1
            && path
                .extension()
                .is_some_and(|extension| extension == "toml")
            && path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .is_some_and(|stem| {
                    !stem.is_empty()
                        && !stem.starts_with('.')
                        && stem.chars().all(|character| {
                            character.is_ascii_alphanumeric() || matches!(character, '-' | '_')
                        })
                });
        if !safe {
            return Err(StoreError {
                path: self.paths.manifests_dir(),
                kind: StoreErrorKind::InvalidFileName,
                message: "manifest filename must be a simple name ending in .toml".into(),
            });
        }
        Ok(self.paths.manifests_dir().join(path))
    }

    pub fn load_user_manifest(
        &self,
        file_name: &str,
    ) -> Result<Option<Versioned<String>>, StoreError> {
        self.ensure_directories()?;
        let path = self.user_manifest_path(file_name)?;
        if !path.exists() {
            return Ok(None);
        }
        let bytes = read_all(&path)?;
        let contents = String::from_utf8(bytes.clone()).map_err(|error| StoreError {
            path: path.clone(),
            kind: StoreErrorKind::Corrupt,
            message: error.to_string().into_boxed_str(),
        })?;
        Ok(Some(Versioned {
            revision: Revision::from_bytes(&bytes),
            value: contents,
        }))
    }

    pub fn save_user_manifest(
        &self,
        file_name: &str,
        expected_revision: Option<&Revision>,
        contents: &str,
    ) -> Result<Versioned<String>, StoreError> {
        self.ensure_directories()?;
        let path = self.user_manifest_path(file_name)?;
        match (self.load_user_manifest(file_name)?, expected_revision) {
            (Some(current), Some(expected)) => {
                require_revision(&path, expected, &current.revision)?;
            }
            (Some(current), None) => {
                return Err(StoreError {
                    path,
                    kind: StoreErrorKind::RevisionConflict,
                    message: format!(
                        "revision conflict: existing manifest has revision {}",
                        current.revision.as_str()
                    )
                    .into_boxed_str(),
                });
            }
            (None, Some(expected)) => {
                return Err(StoreError {
                    path,
                    kind: StoreErrorKind::RevisionConflict,
                    message: format!(
                        "revision conflict: expected {}, but the manifest does not exist",
                        expected.as_str()
                    )
                    .into_boxed_str(),
                });
            }
            (None, None) => {}
        }
        let bytes = contents.as_bytes();
        atomic_write(&path, bytes)?;
        Ok(Versioned {
            revision: Revision::from_bytes(bytes),
            value: contents.to_owned(),
        })
    }

    pub fn load_settings(&self) -> Result<Versioned<Settings>, StoreError> {
        self.ensure_directories()?;
        let path = self.paths.settings_file();
        if !path.exists() {
            return self.initialize_settings(&path);
        }
        let bytes = read_all(&path)?;
        let document = decode_settings_document(&path, &bytes)?;
        Ok(Versioned {
            revision: Revision::from_bytes(&bytes),
            value: document.settings,
        })
    }

    pub fn save_settings(
        &self,
        expected_revision: &Revision,
        settings: &Settings,
    ) -> Result<Versioned<Settings>, StoreError> {
        let path = self.paths.settings_file();
        let current = self.load_settings()?;
        require_revision(&path, expected_revision, &current.revision)?;
        let bytes = toml::to_string_pretty(&SettingsDocument {
            schema_version: SETTINGS_SCHEMA_VERSION,
            settings: settings.clone(),
        })
        .map_err(|error| StoreError::serialize(&path, error.to_string()))?
        .into_bytes();
        atomic_write(&path, &bytes)?;
        Ok(Versioned {
            revision: Revision::from_bytes(&bytes),
            value: settings.clone(),
        })
    }

    pub fn load_state(&self) -> Result<Versioned<CachedState>, StoreError> {
        self.ensure_directories()?;
        let path = self.paths.state_file();
        if !path.exists() {
            return self.initialize_state(&path);
        }
        let bytes = read_all(&path)?;
        let document = decode_state_document(&path, &bytes)?;
        Ok(Versioned {
            revision: Revision::from_bytes(&bytes),
            value: document.state,
        })
    }

    pub fn save_state(
        &self,
        expected_revision: &Revision,
        state: &CachedState,
    ) -> Result<Versioned<CachedState>, StoreError> {
        let path = self.paths.state_file();
        let current = self.load_state()?;
        require_revision(&path, expected_revision, &current.revision)?;
        let bytes = serde_json::to_vec_pretty(&StateDocument {
            schema_version: STATE_SCHEMA_VERSION,
            state: state.clone(),
        })
        .map_err(|error| StoreError::serialize(&path, error.to_string()))?;
        atomic_write(&path, &bytes)?;
        Ok(Versioned {
            revision: Revision::from_bytes(&bytes),
            value: state.clone(),
        })
    }

    pub fn append_run_log(
        &self,
        run_id: &RunId,
        entry: &RunLogEntry,
    ) -> Result<PathBuf, StoreError> {
        self.ensure_directories()?;
        let path = self.run_log_path(run_id)?;
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|error| StoreError::io(&path, error))?;
        let mut writer = BufWriter::new(file);
        serde_json::to_writer(&mut writer, entry)
            .map_err(|error| StoreError::serialize(&path, error.to_string()))?;
        writer
            .write_all(b"\n")
            .and_then(|()| writer.flush())
            .map_err(|error| StoreError::io(&path, error))?;
        writer
            .get_ref()
            .sync_data()
            .map_err(|error| StoreError::io(&path, error))?;
        Ok(path)
    }

    pub fn read_run_log(&self, run_id: &RunId) -> Result<Vec<RunLogEntry>, StoreError> {
        let path = self.run_log_path(run_id)?;
        let file = File::open(&path).map_err(|error| StoreError::io(&path, error))?;
        BufReader::new(file)
            .lines()
            .enumerate()
            .map(|(index, line)| {
                let line = line.map_err(|error| StoreError::io(&path, error))?;
                serde_json::from_str(&line).map_err(|error| StoreError {
                    path: path.clone(),
                    kind: StoreErrorKind::Corrupt,
                    message: format!("invalid JSONL entry at line {}: {error}", index + 1)
                        .into_boxed_str(),
                })
            })
            .collect()
    }

    pub fn rotate_run_logs(
        &self,
        policy: &LogRetentionPolicy,
        active_runs: &BTreeSet<RunId>,
    ) -> Result<RotationReport, StoreError> {
        self.ensure_directories()?;
        let runs_dir = self.paths.runs_dir();
        let mut files = Vec::new();
        for entry in fs::read_dir(&runs_dir).map_err(|error| StoreError::io(&runs_dir, error))? {
            let entry = entry.map_err(|error| StoreError::io(&runs_dir, error))?;
            let path = entry.path();
            if path
                .extension()
                .is_none_or(|extension| extension != "jsonl")
            {
                continue;
            }
            let metadata = entry
                .metadata()
                .map_err(|error| StoreError::io(&path, error))?;
            let run_id = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .and_then(|stem| RunId::new(stem).ok());
            files.push(LogFile {
                path,
                len: metadata.len(),
                modified: metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                active: run_id.is_some_and(|run_id| active_runs.contains(&run_id)),
            });
        }
        files.sort_by(|left, right| {
            left.modified
                .cmp(&right.modified)
                .then_with(|| left.path.cmp(&right.path))
        });
        let mut file_count = files.len();
        let mut total_bytes = files.iter().map(|file| file.len).sum::<u64>();
        let mut removed = Vec::new();
        for file in files {
            if file_count <= policy.max_files && total_bytes <= policy.max_total_bytes {
                break;
            }
            if file.active {
                continue;
            }
            fs::remove_file(&file.path).map_err(|error| StoreError::io(&file.path, error))?;
            file_count -= 1;
            total_bytes = total_bytes.saturating_sub(file.len);
            removed.push(file.path);
        }
        Ok(RotationReport {
            removed,
            remaining_files: file_count,
            remaining_bytes: total_bytes,
            limits_satisfied: file_count <= policy.max_files
                && total_bytes <= policy.max_total_bytes,
        })
    }

    fn initialize_settings(&self, path: &Path) -> Result<Versioned<Settings>, StoreError> {
        let settings = Settings::default();
        let bytes = toml::to_string_pretty(&SettingsDocument {
            schema_version: SETTINGS_SCHEMA_VERSION,
            settings: settings.clone(),
        })
        .map_err(|error| StoreError::serialize(path, error.to_string()))?
        .into_bytes();
        atomic_write(path, &bytes)?;
        Ok(Versioned {
            revision: Revision::from_bytes(&bytes),
            value: settings,
        })
    }

    fn initialize_state(&self, path: &Path) -> Result<Versioned<CachedState>, StoreError> {
        let state = CachedState::default();
        let bytes = serde_json::to_vec_pretty(&StateDocument {
            schema_version: STATE_SCHEMA_VERSION,
            state: state.clone(),
        })
        .map_err(|error| StoreError::serialize(path, error.to_string()))?;
        atomic_write(path, &bytes)?;
        Ok(Versioned {
            revision: Revision::from_bytes(&bytes),
            value: state,
        })
    }

    fn run_log_path(&self, run_id: &RunId) -> Result<PathBuf, StoreError> {
        if !run_id
            .as_str()
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
        {
            return Err(StoreError {
                path: self.paths.runs_dir(),
                kind: StoreErrorKind::InvalidRunId,
                message: "run ID is not safe for use as a log filename".into(),
            });
        }
        Ok(self.paths.runs_dir().join(format!("{run_id}.jsonl")))
    }
}

fn decode_toml_document<T: DeserializeOwned>(path: &Path, bytes: &[u8]) -> Result<T, StoreError> {
    let text = std::str::from_utf8(bytes).map_err(|error| StoreError::corrupt(path, error))?;
    toml::from_str(text).map_err(|error| StoreError::corrupt(path, error))
}

fn decode_json_document<T: DeserializeOwned>(path: &Path, bytes: &[u8]) -> Result<T, StoreError> {
    serde_json::from_slice(bytes).map_err(|error| StoreError::corrupt(path, error))
}

fn decode_settings_document(path: &Path, bytes: &[u8]) -> Result<SettingsDocument, StoreError> {
    let value: toml::Value = decode_toml_document(path, bytes)?;
    let version = value
        .get("schema_version")
        .and_then(toml::Value::as_integer)
        .and_then(|version| u32::try_from(version).ok())
        .ok_or_else(|| StoreError::corrupt(path, "schema_version must be a positive integer"))?;
    if version == SETTINGS_SCHEMA_VERSION {
        return value
            .try_into()
            .map_err(|error| StoreError::corrupt(path, error));
    }
    migrate_settings_document(path, version, value)
}

fn decode_state_document(path: &Path, bytes: &[u8]) -> Result<StateDocument, StoreError> {
    let value: serde_json::Value = decode_json_document(path, bytes)?;
    let version = value
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
        .and_then(|version| u32::try_from(version).ok())
        .ok_or_else(|| StoreError::corrupt(path, "schema_version must be a positive integer"))?;
    if version == STATE_SCHEMA_VERSION {
        return serde_json::from_value(value).map_err(|error| StoreError::corrupt(path, error));
    }
    migrate_state_document(path, version, value)
}

// New migrations are added to these two explicit entries rather than being
// scattered through the read/write paths.
fn migrate_settings_document(
    path: &Path,
    found: u32,
    _document: toml::Value,
) -> Result<SettingsDocument, StoreError> {
    unsupported_schema(path, found, SETTINGS_SCHEMA_VERSION)
}

fn migrate_state_document(
    path: &Path,
    found: u32,
    _document: serde_json::Value,
) -> Result<StateDocument, StoreError> {
    unsupported_schema(path, found, STATE_SCHEMA_VERSION)
}

fn unsupported_schema<T>(path: &Path, found: u32, supported: u32) -> Result<T, StoreError> {
    Err(StoreError {
        path: path.to_path_buf(),
        kind: StoreErrorKind::UnsupportedSchema,
        message: format!(
            "schema version {found} is unsupported; migration entry supports up to {supported}"
        )
        .into_boxed_str(),
    })
}

fn require_revision(path: &Path, expected: &Revision, actual: &Revision) -> Result<(), StoreError> {
    if expected == actual {
        return Ok(());
    }
    Err(StoreError {
        path: path.to_path_buf(),
        kind: StoreErrorKind::RevisionConflict,
        message: format!(
            "revision conflict: expected {}, found {}",
            expected.as_str(),
            actual.as_str()
        )
        .into_boxed_str(),
    })
}

fn read_all(path: &Path) -> Result<Vec<u8>, StoreError> {
    let mut file = File::open(path).map_err(|error| StoreError::io(path, error))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|error| StoreError::io(path, error))?;
    Ok(bytes)
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), StoreError> {
    let parent = path.parent().ok_or_else(|| StoreError {
        path: path.to_path_buf(),
        kind: StoreErrorKind::Io,
        message: "file has no parent directory".into(),
    })?;
    fs::create_dir_all(parent).map_err(|error| StoreError::io(parent, error))?;
    let mut file = AtomicWriteFile::open(path).map_err(|error| StoreError::io(path, error))?;
    file.write_all(bytes)
        .map_err(|error| StoreError::io(path, error))?;
    file.commit().map_err(|error| StoreError::io(path, error))
}

#[derive(Debug)]
struct LogFile {
    path: PathBuf,
    len: u64,
    modified: SystemTime,
    active: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunLogStream {
    Stdout,
    Stderr,
    System,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunLogEntry {
    pub sequence: u64,
    pub timestamp: DateTime<Utc>,
    pub stream: RunLogStream,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RotationReport {
    pub removed: Vec<PathBuf>,
    pub remaining_files: usize,
    pub remaining_bytes: u64,
    pub limits_satisfied: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoreErrorKind {
    PathResolution,
    Io,
    Corrupt,
    Serialize,
    UnsupportedSchema,
    RevisionConflict,
    InvalidRunId,
    InvalidFileName,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{kind:?} at {}: {message}", path.display())]
pub struct StoreError {
    pub path: PathBuf,
    pub kind: StoreErrorKind,
    pub message: Box<str>,
}

impl StoreError {
    fn io(path: &Path, error: std::io::Error) -> Self {
        Self {
            path: path.to_path_buf(),
            kind: StoreErrorKind::Io,
            message: error.to_string().into_boxed_str(),
        }
    }

    fn corrupt(path: &Path, error: impl std::fmt::Display) -> Self {
        Self {
            path: path.to_path_buf(),
            kind: StoreErrorKind::Corrupt,
            message: error.to_string().into_boxed_str(),
        }
    }

    fn serialize(path: &Path, message: String) -> Self {
        Self {
            path: path.to_path_buf(),
            kind: StoreErrorKind::Serialize,
            message: message.into_boxed_str(),
        }
    }

    fn path_resolution(name: &str, message: String) -> Self {
        Self {
            path: PathBuf::from(name),
            kind: StoreErrorKind::PathResolution,
            message: message.into_boxed_str(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;
    use tempfile::TempDir;

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    fn store() -> (TempDir, PersistenceStore) {
        let temporary = TempDir::new().unwrap();
        let paths = AppPaths::new(
            temporary.path().join("config"),
            temporary.path().join("data"),
            temporary.path().join("logs"),
        );
        (temporary, PersistenceStore::new(paths))
    }

    fn run_id(value: &str) -> RunId {
        RunId::new(value).unwrap()
    }

    fn log_entry(sequence: u64, message: &str) -> RunLogEntry {
        RunLogEntry {
            sequence,
            timestamp: Utc::now(),
            stream: RunLogStream::Stdout,
            message: message.into(),
        }
    }

    #[test]
    fn first_load_creates_directories_and_default_documents() {
        let (_temporary, store) = store();
        let settings = store.load_settings().unwrap();
        let state = store.load_state().unwrap();

        assert_eq!(settings.value, Settings::default());
        assert_eq!(state.value, CachedState::default());
        assert!(store.paths().settings_file().is_file());
        assert!(store.paths().state_file().is_file());
        assert!(store.paths().runs_dir().is_dir());
        assert!(store.paths().manifests_dir().is_dir());
    }

    #[test]
    fn revisions_require_canonical_lowercase_hashes() {
        let hash = "a".repeat(64);
        assert_eq!(Revision::from_str(&hash).unwrap().as_str(), hash);
        assert!(Revision::from_str(&"A".repeat(64)).is_err());
        assert!(Revision::from_str(&"a".repeat(63)).is_err());
        assert!(Revision::from_str(&"g".repeat(64)).is_err());
    }

    #[test]
    fn user_manifests_are_revision_guarded_and_cannot_escape_their_directory() {
        let (_temporary, store) = store();
        let contents = "schema_version = 1\nid = \"fixture\"\n";
        let created = store
            .save_user_manifest("fixture.toml", None, contents)
            .unwrap();
        assert_eq!(
            store.load_user_manifest("fixture.toml").unwrap().unwrap(),
            created
        );

        let conflict = store
            .save_user_manifest("fixture.toml", None, contents)
            .unwrap_err();
        assert_eq!(conflict.kind, StoreErrorKind::RevisionConflict);

        let updated = store
            .save_user_manifest(
                "fixture.toml",
                Some(&created.revision),
                &format!("{contents}display_name = \"Fixture\"\n"),
            )
            .unwrap();
        assert_ne!(updated.revision, created.revision);

        for unsafe_name in [
            "../outside.toml",
            "nested/tool.toml",
            ".hidden.toml",
            "tool.txt",
        ] {
            let error = store.user_manifest_path(unsafe_name).unwrap_err();
            assert_eq!(error.kind, StoreErrorKind::InvalidFileName);
        }
    }

    #[test]
    fn settings_and_state_round_trip_with_new_revisions() {
        let (_temporary, store) = store();
        let settings = store.load_settings().unwrap();
        let mut updated = settings.value;
        updated.default_timeout_seconds = 42;
        let saved = store.save_settings(&settings.revision, &updated).unwrap();
        assert_ne!(saved.revision, settings.revision);
        assert_eq!(store.load_settings().unwrap(), saved);

        let state = store.load_state().unwrap();
        let mut updated_state = state.value;
        updated_state.last_refresh_at = Some(Utc::now());
        let saved_state = store.save_state(&state.revision, &updated_state).unwrap();
        assert_eq!(store.load_state().unwrap(), saved_state);
    }

    #[test]
    fn abandoned_temporary_file_does_not_replace_previous_value() {
        let (_temporary, store) = store();
        let original = store.load_settings().unwrap();
        let stray = store
            .paths()
            .config_dir
            .join(".settings.toml.interrupted.tmp");
        fs::write(&stray, b"incomplete = [").unwrap();

        assert_eq!(store.load_settings().unwrap(), original);
        assert!(stray.exists());
    }

    #[test]
    fn discarded_atomic_write_preserves_existing_target() {
        let temporary = TempDir::new().unwrap();
        let path = temporary.path().join("settings.toml");
        fs::write(&path, b"old").unwrap();

        let mut pending = AtomicWriteFile::open(&path).unwrap();
        pending.write_all(b"new").unwrap();
        pending.discard().unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"old");
    }

    #[test]
    fn commit_failure_does_not_replace_non_file_target() {
        let temporary = TempDir::new().unwrap();
        let target = temporary.path().join("target");
        fs::create_dir(&target).unwrap();

        let error = atomic_write(&target, b"new").unwrap_err();

        assert_eq!(error.kind, StoreErrorKind::Io);
        assert!(target.is_dir());
    }

    #[cfg(unix)]
    #[test]
    fn atomic_write_preserves_existing_file_mode() {
        let temporary = TempDir::new().unwrap();
        let path = temporary.path().join("settings.toml");
        fs::write(&path, b"old").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();

        atomic_write(&path, b"new").unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"new");
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o640
        );
    }

    #[cfg(unix)]
    #[test]
    fn read_only_directory_rejects_atomic_write() {
        let temporary = TempDir::new().unwrap();
        let directory = temporary.path().join("read-only");
        fs::create_dir(&directory).unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o555)).unwrap();
        let path = directory.join("settings.toml");

        let result = atomic_write(&path, b"new");
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o755)).unwrap();

        assert_eq!(result.unwrap_err().kind, StoreErrorKind::Io);
        assert!(!path.exists());
    }

    #[test]
    fn revision_conflict_never_overwrites_external_change() {
        let (_temporary, store) = store();
        let first = store.load_settings().unwrap();
        let mut external = first.value.clone();
        external.default_timeout_seconds = 11;
        let external = store.save_settings(&first.revision, &external).unwrap();

        let mut stale = first.value;
        stale.default_timeout_seconds = 99;
        let error = store.save_settings(&first.revision, &stale).unwrap_err();
        assert_eq!(error.kind, StoreErrorKind::RevisionConflict);
        assert_eq!(store.load_settings().unwrap(), external);
    }

    #[test]
    fn corrupt_file_reports_path_and_remains_untouched() {
        let (_temporary, store) = store();
        store.ensure_directories().unwrap();
        let path = store.paths().settings_file();
        let corrupt = b"schema_version = 1\n[settings\n";
        fs::write(&path, corrupt).unwrap();

        let error = store.load_settings().unwrap_err();
        assert_eq!(error.kind, StoreErrorKind::Corrupt);
        assert_eq!(error.path, path);
        assert_eq!(fs::read(error.path).unwrap(), corrupt);
    }

    #[test]
    fn jsonl_round_trip_and_rotation_preserve_active_runs() {
        let (_temporary, store) = store();
        let active = run_id("run-active");
        let old = run_id("run-old");
        let newer = run_id("run-newer");
        store.append_run_log(&old, &log_entry(1, "old")).unwrap();
        thread::sleep(Duration::from_millis(5));
        store
            .append_run_log(&active, &log_entry(1, "active"))
            .unwrap();
        thread::sleep(Duration::from_millis(5));
        store
            .append_run_log(&newer, &log_entry(1, "newer"))
            .unwrap();

        assert_eq!(store.read_run_log(&active).unwrap()[0].message, "active");
        let report = store
            .rotate_run_logs(
                &LogRetentionPolicy {
                    max_files: 1,
                    max_total_bytes: u64::MAX,
                },
                &BTreeSet::from([active.clone()]),
            )
            .unwrap();
        assert!(store.run_log_path(&active).unwrap().exists());
        assert_eq!(report.remaining_files, 1);
        assert!(report.limits_satisfied);
        assert_eq!(report.removed.len(), 2);
    }

    #[test]
    fn rotation_reports_unsatisfied_limits_when_only_active_logs_remain() {
        let (_temporary, store) = store();
        let active = run_id("run-active");
        store
            .append_run_log(&active, &log_entry(1, "active output"))
            .unwrap();
        let report = store
            .rotate_run_logs(
                &LogRetentionPolicy {
                    max_files: 0,
                    max_total_bytes: 0,
                },
                &BTreeSet::from([active]),
            )
            .unwrap();
        assert!(!report.limits_satisfied);
        assert!(report.removed.is_empty());
    }

    #[test]
    fn future_schema_uses_the_migration_entry_and_is_rejected_explicitly() {
        let (_temporary, store) = store();
        store.ensure_directories().unwrap();
        let path = store.paths().settings_file();
        fs::write(
            &path,
            "schema_version = 99\n\n[settings]\ndefault_timeout_seconds = 1\n",
        )
        .unwrap();
        assert_eq!(
            store.load_settings().unwrap_err().kind,
            StoreErrorKind::UnsupportedSchema
        );
    }
}
