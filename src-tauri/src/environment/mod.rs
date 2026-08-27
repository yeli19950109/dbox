//! GUI PATH repair and deterministic executable resolution.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::version::VersionValue;

pub trait PathFixer: Send + Sync {
    fn fix(&self) -> Result<(), PathFixError>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemPathFixer;

static PATH_FIX_RESULT: OnceLock<Result<(), PathFixError>> = OnceLock::new();

impl PathFixer for SystemPathFixer {
    fn fix(&self) -> Result<(), PathFixError> {
        fix_path_env::fix().map_err(|error| PathFixError {
            summary: error.to_string().into_boxed_str(),
        })
    }
}

pub fn fix_gui_path() -> Result<(), PathFixError> {
    PATH_FIX_RESULT
        .get_or_init(|| SystemPathFixer.fix())
        .clone()
}

pub fn startup_path_fix_error() -> Option<PathFixError> {
    PATH_FIX_RESULT
        .get()
        .and_then(|result| result.clone().err())
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("GUI PATH repair failed: {summary}")]
pub struct PathFixError {
    pub summary: Box<str>,
}

pub trait ExecutableLocator: Send + Sync {
    fn find_all(
        &self,
        name: &OsStr,
        path: &OsStr,
        cwd: &Path,
    ) -> Result<Vec<PathBuf>, LocatorError>;

    fn validate(&self, candidate: &Path, path: &OsStr, cwd: &Path)
        -> Result<PathBuf, LocatorError>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct WhichLocator;

impl ExecutableLocator for WhichLocator {
    fn find_all(
        &self,
        name: &OsStr,
        path: &OsStr,
        cwd: &Path,
    ) -> Result<Vec<PathBuf>, LocatorError> {
        let candidates = which::which_in_all(name, Some(path), cwd)
            .map_err(LocatorError::from_which)?
            .collect();
        Ok(deduplicate_paths(candidates))
    }

    fn validate(
        &self,
        candidate: &Path,
        path: &OsStr,
        cwd: &Path,
    ) -> Result<PathBuf, LocatorError> {
        which::which_in(candidate, Some(path), cwd).map_err(LocatorError::from_which)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("executable lookup failed: {summary}")]
pub struct LocatorError {
    pub summary: Box<str>,
}

impl LocatorError {
    fn from_which(error: which::Error) -> Self {
        Self {
            summary: error.to_string().into_boxed_str(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutableSource {
    UserOverride,
    VerifiedCache,
    Path,
    Provider,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedExecutable {
    pub name: String,
    pub path: PathBuf,
    pub source: ExecutableSource,
    pub verified_at: DateTime<Utc>,
    pub fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CachedExecutable {
    pub path: PathBuf,
    pub fingerprint: String,
    pub verified_at: DateTime<Utc>,
}

impl From<&ResolvedExecutable> for CachedExecutable {
    fn from(resolved: &ResolvedExecutable) -> Self {
        Self {
            path: resolved.path.clone(),
            fingerprint: resolved.fingerprint.clone(),
            verified_at: resolved.verified_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveRequest {
    pub name: String,
    pub user_override: Option<PathBuf>,
    pub cached: Option<CachedExecutable>,
    pub provider_paths: Vec<PathBuf>,
}

impl ResolveRequest {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            user_override: None,
            cached: None,
            provider_paths: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolutionOutcome {
    pub resolved: Option<ResolvedExecutable>,
    pub candidates: Vec<PathBuf>,
    pub conflicts: Vec<PathBuf>,
    pub unavailable_reason: Option<String>,
}

pub struct EnvironmentResolver {
    locator: Arc<dyn ExecutableLocator>,
    path: OsString,
    cwd: PathBuf,
}

impl EnvironmentResolver {
    pub fn new(locator: Arc<dyn ExecutableLocator>, path: OsString, cwd: PathBuf) -> Self {
        Self { locator, path, cwd }
    }

    pub fn from_current_process() -> Result<Self, EnvironmentError> {
        let path = std::env::var_os("PATH").ok_or(EnvironmentError::MissingPath)?;
        let cwd = std::env::current_dir().map_err(|error| EnvironmentError::CurrentDirectory {
            summary: error.to_string().into_boxed_str(),
        })?;
        Ok(Self::new(Arc::new(WhichLocator), path, cwd))
    }

    pub fn path(&self) -> &OsStr {
        &self.path
    }

    pub fn resolve(&self, request: &ResolveRequest, now: DateTime<Utc>) -> ResolutionOutcome {
        let mut all_candidates = Vec::new();
        let mut reasons = Vec::new();
        let path_candidates =
            match self
                .locator
                .find_all(OsStr::new(&request.name), &self.path, &self.cwd)
            {
                Ok(candidates) => candidates,
                Err(error) => {
                    reasons.push(format!("PATH lookup failed: {}", error.summary));
                    Vec::new()
                }
            };

        let user = request.user_override.as_ref().and_then(|candidate| {
            match self.validate_candidate(candidate) {
                Ok(path) => {
                    all_candidates.push(path.clone());
                    Some(path)
                }
                Err(error) => {
                    reasons.push(format!("user override unavailable: {}", error.summary));
                    None
                }
            }
        });

        let cached = request.cached.as_ref().and_then(|cached| {
            match self.validate_candidate(&cached.path) {
                Ok(path)
                    if executable_fingerprint(&path).ok().as_ref() == Some(&cached.fingerprint) =>
                {
                    all_candidates.push(path.clone());
                    Some(path)
                }
                Ok(_) => {
                    reasons.push("cached executable changed since verification".into());
                    None
                }
                Err(error) => {
                    reasons.push(format!("cached executable unavailable: {}", error.summary));
                    None
                }
            }
        });

        all_candidates.extend(path_candidates.iter().cloned());
        let mut provider_candidates = Vec::new();
        for candidate in &request.provider_paths {
            match self.validate_candidate(candidate) {
                Ok(path) => {
                    all_candidates.push(path.clone());
                    provider_candidates.push(path);
                }
                Err(error) => {
                    reasons.push(format!("provider path unavailable: {}", error.summary));
                }
            }
        }
        let all_candidates = deduplicate_paths(all_candidates);

        let selected = user
            .map(|path| (path, ExecutableSource::UserOverride))
            .or_else(|| cached.map(|path| (path, ExecutableSource::VerifiedCache)))
            .or_else(|| {
                path_candidates
                    .first()
                    .cloned()
                    .map(|path| (path, ExecutableSource::Path))
            })
            .or_else(|| {
                provider_candidates
                    .first()
                    .cloned()
                    .map(|path| (path, ExecutableSource::Provider))
            });

        let resolved = selected.and_then(|(path, source)| match executable_fingerprint(&path) {
            Ok(fingerprint) => Some(ResolvedExecutable {
                name: request.name.clone(),
                path,
                source,
                verified_at: now,
                fingerprint,
            }),
            Err(error) => {
                reasons.push(error.to_string());
                None
            }
        });
        let conflicts = resolved
            .as_ref()
            .map(|resolved| {
                all_candidates
                    .iter()
                    .filter(|candidate| *candidate != &resolved.path)
                    .cloned()
                    .collect()
            })
            .unwrap_or_else(|| all_candidates.clone());
        ResolutionOutcome {
            resolved,
            candidates: all_candidates,
            conflicts,
            unavailable_reason: if reasons.is_empty() {
                None
            } else {
                Some(reasons.join("; "))
            },
        }
    }

    pub fn diagnostics(
        &self,
        requests: &[ResolveRequest],
        versions: &BTreeMap<String, VersionValue>,
        path_fix_error: Option<&PathFixError>,
        now: DateTime<Utc>,
    ) -> EnvironmentDiagnostics {
        let recorded_fix_error = path_fix_error.cloned().or_else(startup_path_fix_error);
        let commands: BTreeMap<_, _> = requests
            .iter()
            .map(|request| {
                let outcome = self.resolve(request, now);
                (
                    request.name.clone(),
                    CommandDiagnostic {
                        name: request.name.clone(),
                        resolved: outcome.resolved,
                        version: versions.get(&request.name).cloned(),
                        candidates: outcome.candidates,
                        conflicts: outcome.conflicts,
                        unavailable_reason: outcome.unavailable_reason,
                    },
                )
            })
            .collect();
        let path_entries = std::env::split_paths(&self.path).collect();
        let revision =
            environment_revision(commands.values().filter_map(|item| item.resolved.as_ref()));
        EnvironmentDiagnostics {
            path_entries,
            commands,
            path_fix_error: recorded_fix_error.map(|error| error.summary.to_string()),
            revision,
            generated_at: now,
        }
    }

    fn validate_candidate(&self, candidate: &Path) -> Result<PathBuf, LocatorError> {
        if !candidate.is_absolute() {
            return Err(LocatorError {
                summary: "candidate is not an absolute path".into(),
            });
        }
        self.locator.validate(candidate, &self.path, &self.cwd)
    }
}

fn deduplicate_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = BTreeSet::new();
    paths
        .into_iter()
        .filter(|path| seen.insert(path.clone()))
        .collect()
}

pub fn executable_fingerprint(path: &Path) -> Result<String, EnvironmentError> {
    let metadata = std::fs::metadata(path).map_err(|error| EnvironmentError::Fingerprint {
        path: path.to_path_buf(),
        summary: error.to_string().into_boxed_str(),
    })?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let material = format!("{}\0{}\0{modified}", path.display(), metadata.len());
    Ok(blake3::hash(material.as_bytes()).to_hex().to_string())
}

pub fn environment_revision<'a>(
    executables: impl IntoIterator<Item = &'a ResolvedExecutable>,
) -> String {
    let mut material: Vec<_> = executables
        .into_iter()
        .map(|executable| {
            (
                executable.name.as_str(),
                executable.path.as_path(),
                executable.fingerprint.as_str(),
            )
        })
        .collect();
    material.sort();
    let encoded = serde_json::to_vec(&material).expect("resolved executable material serializes");
    blake3::hash(&encoded).to_hex().to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandDiagnostic {
    pub name: String,
    pub resolved: Option<ResolvedExecutable>,
    pub version: Option<VersionValue>,
    pub candidates: Vec<PathBuf>,
    pub conflicts: Vec<PathBuf>,
    pub unavailable_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentDiagnostics {
    pub path_entries: Vec<PathBuf>,
    pub commands: BTreeMap<String, CommandDiagnostic>,
    pub path_fix_error: Option<String>,
    pub revision: String,
    pub generated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EnvironmentError {
    #[error("PATH is unavailable")]
    MissingPath,
    #[error("current directory is unavailable: {summary}")]
    CurrentDirectory { summary: Box<str> },
    #[error("cannot fingerprint {}: {summary}", path.display())]
    Fingerprint { path: PathBuf, summary: Box<str> },
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tempfile::TempDir;

    #[derive(Default)]
    struct FakeLocator {
        candidates: BTreeMap<String, Vec<PathBuf>>,
        valid: BTreeSet<PathBuf>,
    }

    impl ExecutableLocator for FakeLocator {
        fn find_all(
            &self,
            name: &OsStr,
            _path: &OsStr,
            _cwd: &Path,
        ) -> Result<Vec<PathBuf>, LocatorError> {
            Ok(self
                .candidates
                .get(&name.to_string_lossy().into_owned())
                .cloned()
                .unwrap_or_default())
        }

        fn validate(
            &self,
            candidate: &Path,
            _path: &OsStr,
            _cwd: &Path,
        ) -> Result<PathBuf, LocatorError> {
            if self.valid.contains(candidate) {
                Ok(candidate.to_path_buf())
            } else {
                Err(LocatorError {
                    summary: "fake executable is unavailable".into(),
                })
            }
        }
    }

    struct FakeFixer {
        result: Result<(), PathFixError>,
        calls: Mutex<usize>,
    }

    impl PathFixer for FakeFixer {
        fn fix(&self) -> Result<(), PathFixError> {
            *self.calls.lock().unwrap() += 1;
            self.result.clone()
        }
    }

    fn create_binary(directory: &Path, name: &str) -> PathBuf {
        let path = directory.join(name);
        std::fs::write(&path, b"#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        path
    }

    fn now() -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000, 0).unwrap()
    }

    #[test]
    fn injected_fake_binaries_follow_documented_priority() {
        let temporary = TempDir::new().unwrap();
        let user = create_binary(temporary.path(), "user-npm");
        let cached = create_binary(temporary.path(), "cached-npm");
        let from_path = create_binary(temporary.path(), "path-npm");
        let provider = create_binary(temporary.path(), "provider-npm");
        let locator = FakeLocator {
            candidates: BTreeMap::from([("npm".into(), vec![from_path.clone()])]),
            valid: BTreeSet::from([
                user.clone(),
                cached.clone(),
                from_path.clone(),
                provider.clone(),
            ]),
        };
        let resolver = EnvironmentResolver::new(
            Arc::new(locator),
            OsString::from("/injected/path"),
            temporary.path().to_path_buf(),
        );
        let request = ResolveRequest {
            name: "npm".into(),
            user_override: Some(user.clone()),
            cached: Some(CachedExecutable {
                fingerprint: executable_fingerprint(&cached).unwrap(),
                path: cached,
                verified_at: now(),
            }),
            provider_paths: vec![provider],
        };
        let result = resolver.resolve(&request, now());
        assert_eq!(result.resolved.unwrap().path, user);
    }

    #[test]
    fn multiple_path_candidates_keep_order_and_report_conflicts() {
        let temporary = TempDir::new().unwrap();
        let first = create_binary(temporary.path(), "npm-first");
        let second = create_binary(temporary.path(), "npm-second");
        let resolver = EnvironmentResolver::new(
            Arc::new(FakeLocator {
                candidates: BTreeMap::from([(
                    "npm".into(),
                    vec![first.clone(), second.clone(), first.clone()],
                )]),
                valid: BTreeSet::from([first.clone(), second.clone()]),
            }),
            OsString::from("/injected/path"),
            temporary.path().to_path_buf(),
        );
        let result = resolver.resolve(&ResolveRequest::new("npm"), now());
        assert_eq!(result.resolved.unwrap().path, first);
        assert_eq!(result.candidates, vec![first, second.clone()]);
        assert_eq!(result.conflicts, vec![second]);
    }

    #[test]
    fn invalid_cache_falls_back_to_path_lookup() {
        let temporary = TempDir::new().unwrap();
        let from_path = create_binary(temporary.path(), "brew");
        let resolver = EnvironmentResolver::new(
            Arc::new(FakeLocator {
                candidates: BTreeMap::from([("brew".into(), vec![from_path.clone()])]),
                valid: BTreeSet::from([from_path.clone()]),
            }),
            OsString::from("/injected/path"),
            temporary.path().to_path_buf(),
        );
        let request = ResolveRequest {
            name: "brew".into(),
            user_override: None,
            cached: Some(CachedExecutable {
                path: temporary.path().join("removed-brew"),
                fingerprint: "old".into(),
                verified_at: now(),
            }),
            provider_paths: Vec::new(),
        };
        let result = resolver.resolve(&request, now());
        assert_eq!(result.resolved.unwrap().path, from_path);
        assert!(result.unavailable_reason.unwrap().contains("cached"));
    }

    #[test]
    fn path_fix_failure_does_not_block_user_override_or_inherited_path() {
        let temporary = TempDir::new().unwrap();
        let user = create_binary(temporary.path(), "npx");
        let fixer = FakeFixer {
            result: Err(PathFixError {
                summary: "shell unavailable".into(),
            }),
            calls: Mutex::new(0),
        };
        let fix_error = fixer.fix().unwrap_err();
        let resolver = EnvironmentResolver::new(
            Arc::new(FakeLocator {
                candidates: BTreeMap::new(),
                valid: BTreeSet::from([user.clone()]),
            }),
            OsString::from("/existing/path"),
            temporary.path().to_path_buf(),
        );
        let mut request = ResolveRequest::new("npx");
        request.user_override = Some(user.clone());
        assert_eq!(
            resolver.resolve(&request, now()).resolved.unwrap().path,
            user
        );
        let diagnostics =
            resolver.diagnostics(&[request], &BTreeMap::new(), Some(&fix_error), now());
        assert_eq!(
            diagnostics.path_fix_error.as_deref(),
            Some("shell unavailable")
        );
        assert_eq!(*fixer.calls.lock().unwrap(), 1);
    }

    #[test]
    fn system_which_adapter_uses_only_injected_path() {
        let first = TempDir::new().unwrap();
        let second = TempDir::new().unwrap();
        let first_npm = create_binary(first.path(), "npm");
        let second_npm = create_binary(second.path(), "npm");
        let path = std::env::join_paths([first.path(), second.path()]).unwrap();
        let resolver = EnvironmentResolver::new(Arc::new(WhichLocator), path, first.path().into());
        let outcome = resolver.resolve(&ResolveRequest::new("npm"), now());
        assert_eq!(outcome.candidates, vec![first_npm, second_npm]);
    }

    #[test]
    fn changed_executable_fingerprint_changes_environment_revision() {
        let temporary = TempDir::new().unwrap();
        let path = create_binary(temporary.path(), "npm");
        let first = ResolvedExecutable {
            name: "npm".into(),
            path: path.clone(),
            source: ExecutableSource::Path,
            verified_at: now(),
            fingerprint: executable_fingerprint(&path).unwrap(),
        };
        std::fs::write(&path, b"#!/bin/sh\necho changed\n").unwrap();
        let second = ResolvedExecutable {
            fingerprint: executable_fingerprint(&path).unwrap(),
            ..first.clone()
        };
        assert_ne!(
            environment_revision([&first]),
            environment_revision([&second])
        );
    }
}
