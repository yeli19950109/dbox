//! User-level npm packages managed by Pi in its own agent directory.
//!
//! Reading package.json never imports or executes an extension. Git/local sources,
//! project packages, and legacy global npm installations are deliberately excluded.
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use semver::Version;
use serde_json::Value;
use tokio::process::Command;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::domain::{
    ComponentId, Installation, InstallationId, InstallationScope, PackageCoordinate, PackageKind,
    ProviderId,
};
use crate::version::{ComponentStatus, StatusReason, VersionValue};

use super::{
    ProviderCapabilities, ProviderError, ProviderOperation, ProviderProgress,
    ProviderProgressCallback, ProviderStatus, ProviderUpdatePlan, ProviderUpdateRequest,
    Recoverability, ToolProvider, UpdateCheck,
};

pub const PI_EXTENSIONS_PROVIDER_ID: &str = "pi-extensions";

#[derive(Debug, Clone)]
pub struct PiExtensionsProviderOptions {
    pub command_timeout: Duration,
    pub max_concurrency: usize,
}

impl Default for PiExtensionsProviderOptions {
    fn default() -> Self {
        Self {
            command_timeout: Duration::from_secs(15),
            max_concurrency: 4,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PackageSource {
    original: String,
    name: String,
    query: String,
    pinned: bool,
    configuration_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Snapshot {
    source: PackageSource,
    installation: Installation,
    issue: Option<StatusReason>,
}

#[derive(Clone)]
pub struct PiExtensionsProvider {
    id: ProviderId,
    pi_path: PathBuf,
    npm_path: PathBuf,
    agent_dir: PathBuf,
    options: PiExtensionsProviderOptions,
    query_slots: Arc<Semaphore>,
    scanned: Arc<Mutex<BTreeMap<InstallationId, Snapshot>>>,
    checked: Arc<Mutex<BTreeMap<InstallationId, Snapshot>>>,
}

impl PiExtensionsProvider {
    /// `agent_dir` must be the directory used by Pi's inherited environment:
    /// PI_CODING_AGENT_DIR (with ~ expanded), or ~/.pi/agent by default.
    /// An explicit path also permits isolated filesystem/command fixtures in tests.
    pub fn new(
        pi_path: impl Into<PathBuf>,
        npm_path: impl Into<PathBuf>,
        agent_dir: impl Into<PathBuf>,
    ) -> Self {
        Self::with_options(
            pi_path,
            npm_path,
            agent_dir,
            PiExtensionsProviderOptions::default(),
        )
    }

    pub fn with_options(
        pi_path: impl Into<PathBuf>,
        npm_path: impl Into<PathBuf>,
        agent_dir: impl Into<PathBuf>,
        mut options: PiExtensionsProviderOptions,
    ) -> Self {
        options.max_concurrency = options.max_concurrency.max(1);
        Self {
            id: ProviderId::new(PI_EXTENSIONS_PROVIDER_ID).expect("static provider ID is valid"),
            pi_path: pi_path.into(),
            npm_path: npm_path.into(),
            agent_dir: agent_dir.into(),
            query_slots: Arc::new(Semaphore::new(options.max_concurrency)),
            options,
            scanned: Arc::new(Mutex::new(BTreeMap::new())),
            checked: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    fn error(&self, operation: ProviderOperation, summary: &str) -> ProviderError {
        ProviderError::new(
            self.id.clone(),
            operation,
            Recoverability::RequiresConfiguration,
            summary,
        )
    }

    fn validate_paths(&self, operation: ProviderOperation) -> Result<(), ProviderError> {
        if !self.agent_dir.is_absolute() {
            return Err(self.error(operation, "Pi agent directory must be absolute"));
        }
        for (label, path) in [("Pi", &self.pi_path), ("npm", &self.npm_path)] {
            if !path.is_absolute() || !path.is_file() {
                return Err(self.error(
                    operation,
                    &format!("{label} executable must be an existing absolute file"),
                ));
            }
        }
        Ok(())
    }

    async fn sources(
        &self,
        operation: ProviderOperation,
    ) -> Result<Vec<PackageSource>, ProviderError> {
        let bytes = match tokio::fs::read(self.agent_dir.join("settings.json")).await {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(_) => return Err(self.error(operation, "Could not read Pi settings.json")),
        };
        // Never include parser input or unrelated settings (which may contain credentials)
        // in diagnostics. Only the two relevant fields are inspected.
        let settings: Value =
            serde_json::from_slice(bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes))
                .map_err(|_| self.error(operation, "Pi settings.json is not valid JSON"))?;
        let settings = settings
            .as_object()
            .ok_or_else(|| self.error(operation, "Pi settings.json must contain an object"))?;
        if let Some(command) = settings.get("npmCommand") {
            let default_command = command.is_null()
                || command == &serde_json::json!([])
                || command == &serde_json::json!(["npm"]);
            if !default_command {
                return Err(self.error(operation, "Custom Pi npmCommand is unsupported; the configured registry or package manager may differ from npm"));
            }
        }
        let Some(packages) = settings.get("packages") else {
            return Ok(Vec::new());
        };
        let packages = packages
            .as_array()
            .ok_or_else(|| self.error(operation, "Pi settings packages must be an array"))?;
        let mut sources = BTreeMap::<String, PackageSource>::new();
        for entry in packages {
            let raw = entry.as_str().or_else(|| entry.get("source").and_then(Value::as_str))
                .ok_or_else(|| self.error(operation, "Pi packages entries must be source strings or objects with a source string"))?;
            if !raw.starts_with("npm:") {
                continue;
            }
            let source = parse_source(raw, entry).ok_or_else(|| {
                self.error(
                    operation,
                    "Pi packages contains an invalid or unsupported npm source",
                )
            })?;
            if let Some(previous) = sources.get(&source.name) {
                if previous != &source {
                    return Err(self.error(
                        operation,
                        "Pi packages contains conflicting sources for the same npm package",
                    ));
                }
            } else {
                sources.insert(source.name.clone(), source);
            }
        }
        Ok(sources.into_values().collect())
    }

    async fn snapshot(&self, source: PackageSource) -> Snapshot {
        let path = self.agent_dir.join("npm/node_modules").join(&source.name);
        let coordinate = PackageCoordinate::new(PackageKind::Other("pi_npm".into()), &source.name)
            .expect("source names have been validated");
        let mut installation =
            Installation::new(self.id.clone(), coordinate, InstallationScope::User)
                .expect("source names have been validated");
        // A second agent directory must never reuse another installation's saved selection.
        let directory_key = blake3::hash(self.agent_dir.to_string_lossy().as_bytes()).to_hex();
        installation.id = InstallationId::new(format!(
            "{}:{}:{}",
            self.id,
            &directory_key[..16],
            source.name
        ))
        .expect("source-qualified installation ID is valid");
        installation.install_path = Some(path.clone());
        let issue = match tokio::fs::read(path.join("package.json")).await {
            Ok(bytes) => match serde_json::from_slice::<Value>(bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes)) {
                Ok(metadata) => {
                    let name_matches = metadata.get("name").is_none_or(|name| name.as_str() == Some(source.name.as_str()));
                    let version = metadata.get("version").and_then(Value::as_str);
                    if name_matches && version.and_then(strict_version).is_some() {
                        installation.installed_version = version.map(VersionValue::new);
                    }
                    if !name_matches {
                        Some(StatusReason::new("package_name_mismatch", "Managed package metadata does not match the configured npm package", false))
                    } else if version.and_then(strict_version).is_none() {
                        Some(StatusReason::new("installed_version_unknown", "Managed package does not contain a valid npm version", false))
                    } else { None }
                }
                Err(_) => Some(StatusReason::new("invalid_package_metadata", "Managed package.json is not valid JSON", false)),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Some(StatusReason::new(
                "managed_package_missing", "Configured package is missing from Pi's managed npm directory; legacy global npm packages are outside this provider's scope", false,
            )),
            Err(_) => Some(StatusReason::new("package_metadata_unreadable", "Could not read managed package.json", true)),
        };
        Snapshot {
            source,
            installation,
            issue,
        }
    }

    async fn query(&self, snapshot: &Snapshot) -> UpdateCheck {
        let installation = &snapshot.installation;
        if let Some(reason) = &snapshot.issue {
            return check(
                installation,
                None,
                ComponentStatus::Unknown {
                    reason: reason.clone(),
                },
            );
        }
        if snapshot.source.pinned {
            return check(installation, None, ComponentStatus::Unsupported { reason: StatusReason::new(
                "version_pinned", "Pi keeps this npm package at its configured exact version; change its source in Pi settings to follow updates", false,
            ) });
        }
        let result = async {
            let _permit = self.query_slots.acquire().await.map_err(|_| "Pi npm query was cancelled")?;
            let mut command = Command::new(&self.npm_path);
            command.args(["view", snapshot.source.query.as_str(), "version", "--json"])
                .current_dir(&self.agent_dir)
                .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
            let output = tokio::time::timeout(self.options.command_timeout, command.output()).await
                .map_err(|_| "Pi npm registry query timed out")?
                .map_err(|_| "Could not start Pi npm registry query")?;
            if !output.status.success() {
                // npm stderr can contain registry URLs and credentials. Do not surface it.
                return Err("Pi npm registry query failed; check npm registry access and the configured package source");
            }
            latest_version(&output.stdout).ok_or("Pi npm registry query did not return a valid version")
        }.await;
        match result {
            Ok(latest) => {
                let latest = VersionValue::new(latest.to_string());
                let status = ComponentStatus::from_versions(
                    installation.installed_version.as_ref(),
                    Some(&latest),
                );
                check(installation, Some(latest), status)
            }
            Err(summary) => check(
                installation,
                None,
                ComponentStatus::CheckFailed {
                    reason: StatusReason::new("npm_registry_query_failed", summary, true),
                },
            ),
        }
    }
}

#[async_trait]
impl ToolProvider for PiExtensionsProvider {
    fn id(&self) -> ProviderId {
        self.id.clone()
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            full_scan: true,
            check_updates: true,
            update: true,
            ..ProviderCapabilities::default()
        }
    }

    async fn probe(&self) -> Result<ProviderStatus, ProviderError> {
        self.validate_paths(ProviderOperation::Probe)?;
        self.sources(ProviderOperation::Probe).await?;
        Ok(ProviderStatus {
            available: true, version: None,
            detail: Some(format!("User-level npm packages configured in {}/settings.json and installed in its npm/node_modules directory. Git/local sources, project packages, custom npmCommand, and legacy global npm installs are excluded. Pi updates inherit PI_CODING_AGENT_DIR from the application environment.", self.agent_dir.display())),
        })
    }

    async fn scan(&self) -> Result<Vec<Installation>, ProviderError> {
        self.scan_with_progress(&mut |_| {}).await
    }

    async fn scan_with_progress(
        &self,
        on_progress: &mut ProviderProgressCallback<'_>,
    ) -> Result<Vec<Installation>, ProviderError> {
        self.scanned
            .lock()
            .expect("Pi snapshot mutex is not poisoned")
            .clear();
        let sources = match async {
            self.validate_paths(ProviderOperation::Scan)?;
            self.sources(ProviderOperation::Scan).await
        }
        .await
        {
            Ok(sources) => sources,
            Err(error) => {
                // Failed scans invalidate old update plans, while a successful
                // post-check must preserve plans for other unchanged packages.
                self.checked
                    .lock()
                    .expect("Pi checked mutex is not poisoned")
                    .clear();
                return Err(error);
            }
        };
        let total = sources.len();
        on_progress(ProviderProgress::items(None, 0, total));
        let mut snapshots = BTreeMap::new();
        let mut installations = Vec::with_capacity(total);
        for (index, source) in sources.into_iter().enumerate() {
            let name = source.name.clone();
            on_progress(ProviderProgress::items(Some(name.clone()), index, total));
            let snapshot = self.snapshot(source).await;
            installations.push(snapshot.installation.clone());
            snapshots.insert(snapshot.installation.id.clone(), snapshot);
            on_progress(ProviderProgress::items(Some(name), index + 1, total));
        }
        self.checked
            .lock()
            .expect("Pi checked mutex is not poisoned")
            .retain(|id, previous| snapshots.get(id) == Some(previous));
        *self
            .scanned
            .lock()
            .expect("Pi snapshot mutex is not poisoned") = snapshots;
        Ok(installations)
    }

    async fn check_updates(
        &self,
        installations: &[Installation],
    ) -> Result<Vec<UpdateCheck>, ProviderError> {
        self.check_updates_with_progress(installations, &mut |_| {})
            .await
    }

    async fn check_updates_with_progress(
        &self,
        installations: &[Installation],
        on_progress: &mut ProviderProgressCallback<'_>,
    ) -> Result<Vec<UpdateCheck>, ProviderError> {
        self.validate_paths(ProviderOperation::CheckUpdates)?;
        self.checked
            .lock()
            .expect("Pi checked mutex is not poisoned")
            .clear();
        let snapshots = self
            .scanned
            .lock()
            .expect("Pi snapshot mutex is not poisoned")
            .clone();
        let mut checks = vec![None; installations.len()];
        let mut completed = 0;
        on_progress(ProviderProgress::items(
            None,
            completed,
            installations.len(),
        ));
        let mut tasks = JoinSet::new();
        for (index, installation) in installations.iter().enumerate() {
            match snapshots
                .get(&installation.id)
                .filter(|snapshot| &snapshot.installation == installation)
            {
                Some(snapshot) => {
                    let provider = self.clone();
                    let snapshot = snapshot.clone();
                    tasks.spawn(async move {
                        let result = provider.query(&snapshot).await;
                        (index, snapshot, result)
                    });
                }
                None => {
                    checks[index] = Some(check(
                        installation,
                        None,
                        ComponentStatus::Unknown {
                            reason: StatusReason::new(
                                "scan_required",
                                "Refresh Pi packages before checking this installation",
                                false,
                            ),
                        },
                    ));
                    completed += 1;
                    on_progress(ProviderProgress::items(
                        Some(installation.package.name.clone()),
                        completed,
                        installations.len(),
                    ));
                }
            }
        }
        while let Some(result) = tasks.join_next().await {
            let (index, snapshot, result) = result.map_err(|_| {
                self.error(
                    ProviderOperation::CheckUpdates,
                    "Pi package query task failed",
                )
            })?;
            if matches!(
                result.status,
                ComponentStatus::UpToDate | ComponentStatus::UpdateAvailable
            ) {
                self.checked
                    .lock()
                    .expect("Pi checked mutex is not poisoned")
                    .insert(snapshot.installation.id.clone(), snapshot);
            }
            checks[index] = Some(result);
            completed += 1;
            on_progress(ProviderProgress::items(
                Some(installations[index].package.name.clone()),
                completed,
                installations.len(),
            ));
        }
        Ok(checks
            .into_iter()
            .map(|check| check.expect("every query returns a result"))
            .collect())
    }

    async fn validate_update_plan(&self, plan: &ProviderUpdatePlan) -> Result<(), ProviderError> {
        let current = self
            .plan_update(ProviderUpdateRequest {
                installation_id: plan.installation_id.clone(),
                component_id: plan.component_id.clone(),
                strategy_id: plan.strategy_id.clone(),
                target_version: None,
            })
            .await?;
        if &current != plan {
            return Err(self.error(
                ProviderOperation::PlanUpdate,
                "Pi update plan no longer matches its configured source; refresh and preview again",
            ));
        }
        Ok(())
    }

    async fn plan_update(
        &self,
        request: ProviderUpdateRequest,
    ) -> Result<ProviderUpdatePlan, ProviderError> {
        let operation = ProviderOperation::PlanUpdate;
        self.validate_paths(operation)?;
        if request.component_id.as_str() != "core"
            || request.strategy_id.as_str() != "provider-default"
            || request.target_version.is_some()
        {
            return Err(self.error(operation, "Pi extensions only support core/provider-default and the version configured in Pi settings"));
        }
        let checked = self.checked.lock().expect("Pi checked mutex is not poisoned").get(&request.installation_id).cloned()
            .ok_or_else(|| self.error(operation, "Refresh and successfully check this Pi package before updating; pinned or missing packages cannot be updated"))?;
        let source = self
            .sources(operation)
            .await?
            .into_iter()
            .find(|source| source.name == checked.source.name)
            .ok_or_else(|| {
                self.error(
                    operation,
                    "Pi package source changed or was removed; refresh before updating",
                )
            })?;
        let current = self.snapshot(source).await;
        if current != checked || current.source.pinned || current.issue.is_some() {
            return Err(self.error(
                operation,
                "Pi package configuration or installed version changed; refresh before updating",
            ));
        }
        Ok(ProviderUpdatePlan {
            provider_id: self.id.clone(),
            installation_id: request.installation_id,
            component_id: request.component_id,
            strategy_id: request.strategy_id,
            program: self.pi_path.to_string_lossy().into_owned(),
            args: vec![
                "update".into(),
                "--extension".into(),
                current.source.original,
                "--no-approve".into(),
            ],
        })
    }
}

fn check(
    installation: &Installation,
    latest_version: Option<VersionValue>,
    status: ComponentStatus,
) -> UpdateCheck {
    UpdateCheck {
        installation_id: installation.id.clone(),
        component_id: ComponentId::new("core").expect("static component ID is valid"),
        installed_version: installation.installed_version.clone(),
        latest_version,
        status,
    }
}

fn parse_source(original: &str, entry: &Value) -> Option<PackageSource> {
    if original.chars().any(char::is_control) || original.len() > 2048 {
        return None;
    }
    let spec = original.strip_prefix("npm:")?.trim();
    let split = if let Some(scoped) = spec.strip_prefix('@') {
        scoped.find('@').map(|index| index + 1)
    } else {
        spec.find('@')
    };
    let (name, constraint) = match split {
        Some(index) => (&spec[..index], Some(spec[index + 1..].trim())),
        None => (spec, None),
    };
    if !valid_package_name(name) {
        return None;
    }
    if let Some(constraint) = constraint {
        if constraint.is_empty()
            || !constraint
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._-^~*<>=| +".contains(&byte))
        {
            return None;
        }
    }
    Some(PackageSource {
        original: original.into(),
        name: name.into(),
        query: if constraint.is_some() {
            spec.into()
        } else {
            format!("{name}@latest")
        },
        pinned: constraint.and_then(strict_version).is_some(),
        configuration_key: blake3::hash(entry.to_string().as_bytes())
            .to_hex()
            .to_string(),
    })
}

fn valid_package_name(name: &str) -> bool {
    fn segment(value: &str) -> bool {
        !value.is_empty()
            && !value.starts_with(['.', '_', '-'])
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
    }
    if name.len() > 214 {
        return false;
    }
    match name.strip_prefix('@') {
        Some(scoped) => scoped
            .split_once('/')
            .is_some_and(|(scope, package)| segment(scope) && segment(package)),
        None => segment(name),
    }
}

fn strict_version(value: &str) -> Option<Version> {
    // npm's semver.valid accepts a leading v; do not use VersionValue's permissive
    // extraction, which could mistake arbitrary error text for package metadata.
    Version::parse(value.trim().strip_prefix('v').unwrap_or(value.trim())).ok()
}

fn latest_version(bytes: &[u8]) -> Option<Version> {
    match serde_json::from_slice::<Value>(bytes).ok()? {
        Value::String(value) => strict_version(&value),
        Value::Array(values) => {
            // npm has already applied the original range/tag. Reject malformed
            // entries rather than reporting a partially parsed result as latest.
            values
                .into_iter()
                .map(|value| strict_version(value.as_str()?))
                .collect::<Option<Vec<_>>>()?
                .into_iter()
                .max_by(|a, b| a.cmp_precedence(b))
        }
        _ => None,
    }
}
