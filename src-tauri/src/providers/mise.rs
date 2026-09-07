//! Global mise requests and coexisting installations, discovered through the public CLI.
//!
//! A global request is a stable installation slot: its ID survives a version upgrade.
//! Other installed versions are user-scoped, read-only inventory. This preserves activation
//! and multiple versions without extending the shared domain or API.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::Deserialize;
use tokio::process::Command;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::domain::{
    ComponentId, Installation, InstallationId, InstallationScope, PackageCoordinate, PackageKind,
    ProviderId,
};
use crate::version::{ComponentStatus, StatusReason, VersionValue};

use super::{
    ProviderCapabilities, ProviderError, ProviderOperation, ProviderStatus, ProviderUpdatePlan,
    ProviderUpdateRequest, Recoverability, ToolProvider, UpdateCheck,
};

pub const MISE_PROVIDER_ID: &str = "mise";

#[derive(Debug, Clone)]
pub struct MiseProviderOptions {
    /// Run outside the app's project directory, including when executing an approved plan.
    pub home_dir: PathBuf,
    pub command_timeout: Duration,
    pub update_cache_ttl: Duration,
    pub max_concurrency: usize,
}

impl Default for MiseProviderOptions {
    fn default() -> Self {
        Self {
            home_dir: std::env::home_dir().unwrap_or_default(),
            command_timeout: Duration::from_secs(30),
            update_cache_ttl: Duration::from_secs(15 * 60),
            max_concurrency: 4,
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
struct Source {
    // Environment/argument sources have no path. Only global file sources are writable.
    path: Option<PathBuf>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
struct VersionRecord {
    version: String,
    install_path: PathBuf,
    installed: bool,
    active: bool,
    requested_version: Option<String>,
    source: Option<Source>,
    symlinked_to: Option<PathBuf>,
}

type Listing = BTreeMap<String, Vec<VersionRecord>>;

#[derive(Deserialize)]
struct ToolInfo {
    backend: String,
}

#[derive(Clone, PartialEq, Eq)]
struct Entry {
    installation: Installation,
    tool_name: String,
    requested_version: Option<String>,
    source: Option<Source>,
    linked: bool,
    metadata_error: Option<ProviderError>,
}

#[derive(Clone)]
struct CachedCheck {
    checked_at: Instant,
    entry: Entry,
    check: UpdateCheck,
}

#[derive(Clone)]
pub struct MiseProvider {
    id: ProviderId,
    mise_path: PathBuf,
    options: MiseProviderOptions,
    status: Arc<Mutex<Option<ProviderStatus>>>,
    can_upgrade: Arc<Mutex<bool>>,
    entries: Arc<Mutex<BTreeMap<InstallationId, Entry>>>,
    cache: Arc<Mutex<BTreeMap<InstallationId, CachedCheck>>>,
    query_slots: Arc<Semaphore>,
}

impl MiseProvider {
    pub fn new(mise_path: impl Into<PathBuf>) -> Self {
        Self::with_options(mise_path, MiseProviderOptions::default())
    }

    pub fn with_options(mise_path: impl Into<PathBuf>, options: MiseProviderOptions) -> Self {
        Self {
            id: ProviderId::new(MISE_PROVIDER_ID).expect("valid mise provider ID"),
            mise_path: mise_path.into(),
            query_slots: Arc::new(Semaphore::new(options.max_concurrency.max(1))),
            options,
            status: Arc::new(Mutex::new(None)),
            can_upgrade: Arc::new(Mutex::new(false)),
            entries: Arc::new(Mutex::new(BTreeMap::new())),
            cache: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    pub fn clear_update_cache(&self) {
        self.cache.lock().expect("mise cache mutex").clear();
    }

    fn error(
        &self,
        operation: ProviderOperation,
        recoverability: Recoverability,
        summary: impl AsRef<str>,
    ) -> ProviderError {
        ProviderError::new(self.id.clone(), operation, recoverability, summary)
    }

    fn args(&self, args: &[&str]) -> Vec<String> {
        let mut result = vec!["--cd".into(), self.options.home_dir.display().to_string()];
        result.extend(args.iter().map(|arg| (*arg).to_owned()));
        result
    }

    async fn run(
        &self,
        operation: ProviderOperation,
        args: &[&str],
    ) -> Result<Vec<u8>, ProviderError> {
        if !self.mise_path.is_absolute()
            || !self.mise_path.is_file()
            || !self.options.home_dir.is_absolute()
            || !self.options.home_dir.is_dir()
        {
            return Err(self.error(
                operation,
                Recoverability::RequiresConfiguration,
                "mise requires an existing absolute executable and home directory",
            ));
        }
        let output = tokio::time::timeout(
            self.options.command_timeout,
            Command::new(&self.mise_path)
                .args(self.args(args))
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true)
                .output(),
        )
        .await
        .map_err(|_| {
            self.error(
                operation,
                Recoverability::Retryable,
                "mise command timed out",
            )
        })?
        .map_err(|error| {
            self.error(
                operation,
                Recoverability::Retryable,
                format!("could not run mise: {error}"),
            )
        })?;
        if !output.status.success() {
            // Bound and redact stderr before returning it to the UI or persisted state.
            let stderr = super::redact_summary(&String::from_utf8_lossy(&output.stderr));
            return Err(self.error(
                operation,
                Recoverability::Retryable,
                format!(
                    "mise exited with {}; {}",
                    output.status,
                    stderr.chars().take(512).collect::<String>()
                ),
            ));
        }
        Ok(output.stdout)
    }

    async fn json<T: serde::de::DeserializeOwned>(
        &self,
        operation: ProviderOperation,
        args: &[&str],
    ) -> Result<T, ProviderError> {
        let output = self.run(operation, args).await?;
        serde_json::from_slice(&output).map_err(|_| {
            self.error(
                operation,
                Recoverability::Retryable,
                "mise returned invalid JSON or an unsupported output schema",
            )
        })
    }

    async fn ensure_probed(&self) -> Result<(), ProviderError> {
        let probed = self.status.lock().expect("mise status mutex").is_some();
        if !probed {
            self.probe().await?;
        }
        Ok(())
    }

    fn entry(&self, installation: &Installation) -> Option<Entry> {
        self.entries
            .lock()
            .expect("mise entries mutex")
            .get(&installation.id)
            .filter(|entry| entry.installation == *installation)
            .cloned()
    }

    fn blocked_status(&self, entry: &Entry) -> Option<ComponentStatus> {
        if let Some(error) = &entry.metadata_error {
            return Some(ComponentStatus::CheckFailed {
                reason: StatusReason::new("mise_backend_unavailable", &error.summary, true),
            });
        }
        let reason = if entry.installation.scope != InstallationScope::Global {
            Some((
                "mise_inactive",
                "此版本未在 mise 全局配置中启用，仅展示安装信息",
            ))
        } else if entry.linked {
            Some(("mise_linked", "此版本是链接安装，请在原安装来源更新"))
        } else if !*self.can_upgrade.lock().expect("mise capability mutex") {
            Some((
                "mise_upgrade_unsupported",
                "当前 mise 不支持保留旧版本的更新，请先升级 mise",
            ))
        } else if !entry
            .requested_version
            .as_deref()
            .is_some_and(valid_request)
        {
            Some((
                "mise_request_unsupported",
                "此 mise 版本请求暂不支持自动更新",
            ))
        } else {
            None
        };
        reason.map(|(code, summary)| ComponentStatus::Unsupported {
            reason: StatusReason::new(code, summary, false),
        })
    }

    async fn check_entry(&self, entry: Entry) -> UpdateCheck {
        if let Some(status) = self.blocked_status(&entry) {
            return make_check(&entry.installation, None, status);
        }
        if let Some(cached) = self
            .cache
            .lock()
            .expect("mise cache mutex")
            .get(&entry.installation.id)
            .filter(|cached| {
                cached.entry == entry && cached.checked_at.elapsed() < self.options.update_cache_ttl
            })
        {
            return cached.check.clone();
        }
        let _permit = self
            .query_slots
            .acquire()
            .await
            .expect("mise query semaphore");
        let spec = format!(
            "{}@{}",
            entry.installation.package.name,
            entry
                .requested_version
                .as_deref()
                .expect("validated request")
        );
        let result = self
            .run(ProviderOperation::CheckUpdates, &["latest", "--", &spec])
            .await;
        let result = result.and_then(|bytes| {
            let version = String::from_utf8(bytes)
                .unwrap_or_default()
                .trim()
                .to_owned();
            if valid_version(&version) {
                Ok(VersionValue::new(version))
            } else {
                Err(self.error(
                    ProviderOperation::CheckUpdates,
                    Recoverability::Retryable,
                    "mise latest did not return a single valid version",
                ))
            }
        });
        match result {
            Ok(latest) => {
                let status = if entry.installation.installed_version.as_ref() == Some(&latest) {
                    ComponentStatus::UpToDate
                } else {
                    ComponentStatus::from_versions(
                        entry.installation.installed_version.as_ref(),
                        Some(&latest),
                    )
                };
                let check = make_check(&entry.installation, Some(latest), status);
                self.cache.lock().expect("mise cache mutex").insert(
                    entry.installation.id.clone(),
                    CachedCheck {
                        checked_at: Instant::now(),
                        entry,
                        check: check.clone(),
                    },
                );
                check
            }
            Err(error) => make_check(
                &entry.installation,
                None,
                ComponentStatus::CheckFailed {
                    reason: StatusReason::new("mise_latest_failed", error.summary, true),
                },
            ),
        }
    }
}

#[async_trait]
impl ToolProvider for MiseProvider {
    fn id(&self) -> ProviderId {
        self.id.clone()
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            full_scan: true,
            check_updates: true,
            update: true,
            multiple_versions: true,
            ..ProviderCapabilities::default()
        }
    }

    async fn probe(&self) -> Result<ProviderStatus, ProviderError> {
        *self.status.lock().expect("mise status mutex") = None;
        *self.can_upgrade.lock().expect("mise capability mutex") = false;
        let output = self.run(ProviderOperation::Probe, &["version"]).await?;
        let version = String::from_utf8(output)
            .unwrap_or_default()
            .trim()
            .to_owned();
        if version.is_empty() || version.lines().count() != 1 {
            return Err(self.error(
                ProviderOperation::Probe,
                Recoverability::Retryable,
                "mise returned an invalid version",
            ));
        }
        let help = self
            .run(ProviderOperation::Probe, &["upgrade", "--help"])
            .await?;
        let can_upgrade = String::from_utf8_lossy(&help).contains("--no-prune");
        let status = ProviderStatus {
            available: true,
            version: Some(VersionValue::new(version)),
            detail: Some(format!(
                "path={}; global context={}; multiple versions; {}",
                self.mise_path.display(),
                self.options.home_dir.display(),
                if can_upgrade {
                    "updates preserve old versions"
                } else {
                    "read-only: upgrade --no-prune is unavailable"
                }
            )),
        };
        *self.can_upgrade.lock().expect("mise capability mutex") = can_upgrade;
        *self.status.lock().expect("mise status mutex") = Some(status.clone());
        Ok(status)
    }

    async fn scan(&self) -> Result<Vec<Installation>, ProviderError> {
        self.ensure_probed().await?;
        let operation = ProviderOperation::Scan;
        let installed: Listing = self
            .json(operation, &["ls", "--installed", "--json"])
            .await?;
        let globals: Listing = self
            .json(operation, &["ls", "--global", "--installed", "--json"])
            .await?;
        let mut entries = BTreeMap::new();
        // Include global records even if another source overrides that version in the full list.
        let mut inventory = installed;
        for (tool, versions) in &globals {
            let records = inventory.entry(tool.clone()).or_default();
            for version in versions {
                if !records.iter().any(|record| {
                    record.version == version.version && record.install_path == version.install_path
                }) {
                    records.push(version.clone());
                }
            }
        }
        for (tool, records) in inventory {
            if !valid_tool(&tool) {
                return Err(self.error(
                    operation,
                    Recoverability::Permanent,
                    "mise returned an invalid tool name",
                ));
            }
            let metadata: Result<ToolInfo, _> =
                self.json(operation, &["tool", "--json", "--", &tool]).await;
            let (backend, metadata_error) = match metadata {
                Ok(info) if valid_tool(&info.backend) => (info.backend, None),
                Ok(_) => (
                    tool.clone(),
                    Some(self.error(
                        operation,
                        Recoverability::Permanent,
                        "mise returned an invalid backend",
                    )),
                ),
                Err(error) => (tool.clone(), Some(error)),
            };
            for record in records {
                if !record.installed {
                    continue;
                }
                if !valid_version(&record.version) || !record.install_path.is_absolute() {
                    return Err(self.error(
                        operation,
                        Recoverability::Retryable,
                        "mise returned an invalid installed version or path",
                    ));
                }
                let global = globals.get(&tool).and_then(|versions| {
                    versions.iter().find(|global| {
                        global.installed
                            && global.active
                            && global.version == record.version
                            && global.install_path == record.install_path
                    })
                });
                let scope = if global.is_some() {
                    InstallationScope::Global
                } else {
                    InstallationScope::User
                };
                let source = global.and_then(|global| global.source.clone());
                let requested_version = global.and_then(|global| global.requested_version.clone());
                if global.is_some()
                    && !source.as_ref().is_some_and(|source| {
                        source.path.as_ref().is_some_and(|path| path.is_absolute())
                    })
                {
                    return Err(self.error(
                        operation,
                        Recoverability::Retryable,
                        "mise global request has no absolute config source",
                    ));
                }
                let mut installation = Installation::new(
                    self.id.clone(),
                    PackageCoordinate::new(PackageKind::Mise, &backend)
                        .expect("validated coordinate"),
                    scope,
                )
                .expect("valid mise installation");
                // Include the data root, backend and config source, but not the resolved version
                // of an active request, so post-check can find the same slot after an upgrade.
                let identity = serde_json::to_vec(&(
                    &tool,
                    &backend,
                    record.install_path.parent(),
                    source.as_ref().and_then(|source| source.path.as_ref()),
                    if global.is_some() {
                        requested_version.as_deref()
                    } else {
                        Some(record.version.as_str())
                    },
                    global.is_some(),
                ))
                .expect("serializable mise identity");
                installation.id =
                    InstallationId::new(format!("mise:{}", blake3::hash(&identity).to_hex()))
                        .expect("valid mise installation ID");
                installation.install_path = Some(record.install_path);
                installation.installed_version = Some(VersionValue::new(record.version));
                let entry = Entry {
                    installation,
                    tool_name: tool.clone(),
                    requested_version,
                    source,
                    linked: record.symlinked_to.is_some(),
                    metadata_error: metadata_error.clone(),
                };
                if entries
                    .insert(entry.installation.id.clone(), entry)
                    .is_some()
                {
                    return Err(self.error(
                        operation,
                        Recoverability::Retryable,
                        "mise returned ambiguous duplicate installation slots",
                    ));
                }
            }
        }
        let installations = entries
            .values()
            .map(|entry| entry.installation.clone())
            .collect();
        self.cache
            .lock()
            .expect("mise cache mutex")
            .retain(|id, cached| entries.get(id) == Some(&cached.entry));
        *self.entries.lock().expect("mise entries mutex") = entries;
        Ok(installations)
    }

    async fn check_updates(
        &self,
        installations: &[Installation],
    ) -> Result<Vec<UpdateCheck>, ProviderError> {
        let mut tasks = JoinSet::new();
        for (index, installation) in installations.iter().enumerate() {
            let provider = self.clone();
            let installation = installation.clone();
            tasks.spawn(async move {
                let check = match provider.entry(&installation) {
                    Some(entry) => provider.check_entry(entry).await,
                    None => make_check(
                        &installation,
                        None,
                        ComponentStatus::CheckFailed {
                            reason: StatusReason::new(
                                "mise_installation_changed",
                                "Refresh mise before checking this installation",
                                true,
                            ),
                        },
                    ),
                };
                (index, check)
            });
        }
        let mut checks = BTreeMap::new();
        while let Some(result) = tasks.join_next().await {
            let (index, check) = result.map_err(|_| {
                self.error(
                    ProviderOperation::CheckUpdates,
                    Recoverability::Retryable,
                    "mise update query task failed",
                )
            })?;
            checks.insert(index, check);
        }
        Ok(checks.into_values().collect())
    }

    async fn plan_update(
        &self,
        request: ProviderUpdateRequest,
    ) -> Result<ProviderUpdatePlan, ProviderError> {
        let operation = ProviderOperation::PlanUpdate;
        if request.component_id.as_str() != "core"
            || request.strategy_id.as_str() != "provider-default"
            || request.target_version.is_some()
        {
            return Err(self.error(operation, Recoverability::Permanent, "mise supports core/provider-default updates within the configured range; target-version pinning is unsupported"));
        }
        let entry = self
            .entries
            .lock()
            .expect("mise entries mutex")
            .get(&request.installation_id)
            .cloned()
            .ok_or_else(|| {
                self.error(
                    operation,
                    Recoverability::RequiresConfiguration,
                    "refresh mise before planning an update",
                )
            })?;
        if let Some(status) = self.blocked_status(&entry) {
            return Err(self.error(
                operation,
                Recoverability::RequiresConfiguration,
                format!("mise installation cannot be updated: {status:?}"),
            ));
        }
        // Re-read both scopes: a home-directory override or an environment-selected tool
        // must never redirect an update away from the global request the user selected.
        let globals: Listing = self
            .json(operation, &["ls", "--global", "--installed", "--json"])
            .await?;
        let current: Listing = self.json(operation, &["ls", "--current", "--json"]).await?;
        let selected = globals.get(&entry.tool_name).and_then(|versions| {
            versions.iter().find(|record| {
                record.installed
                    && record.active
                    && record.source == entry.source
                    && record.requested_version == entry.requested_version
                    && Some(&record.install_path) == entry.installation.install_path.as_ref()
                    && Some(record.version.as_str())
                        == entry
                            .installation
                            .installed_version
                            .as_ref()
                            .map(VersionValue::raw)
            })
        });
        if selected.is_none() || current.get(&entry.tool_name) != globals.get(&entry.tool_name) {
            return Err(self.error(operation, Recoverability::RequiresConfiguration, "mise global selection changed or is overridden in the home context; refresh and resolve the override before updating"));
        }
        let info: ToolInfo = self
            .json(operation, &["tool", "--json", "--", &entry.tool_name])
            .await?;
        if info.backend != entry.installation.package.name {
            return Err(self.error(
                operation,
                Recoverability::RequiresConfiguration,
                "mise backend changed; refresh before updating",
            ));
        }
        let spec = format!(
            "{}@{}",
            entry.tool_name,
            entry
                .requested_version
                .as_deref()
                .expect("validated request")
        );
        Ok(ProviderUpdatePlan {
            provider_id: self.id.clone(),
            installation_id: request.installation_id,
            component_id: request.component_id,
            strategy_id: request.strategy_id,
            program: self.mise_path.display().to_string(),
            args: self.args(&["upgrade", "--no-prune", "--yes", "--", &spec]),
        })
    }
}

fn make_check(
    installation: &Installation,
    latest_version: Option<VersionValue>,
    status: ComponentStatus,
) -> UpdateCheck {
    UpdateCheck {
        installation_id: installation.id.clone(),
        component_id: ComponentId::new("core").expect("valid component ID"),
        installed_version: installation.installed_version.clone(),
        latest_version,
        status,
    }
}

fn valid_tool(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('-')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.:/@+".contains(&byte))
}

fn valid_version(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('-')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.+".contains(&byte))
}

fn valid_request(value: &str) -> bool {
    valid_version(value) && !matches!(value, "system")
}
