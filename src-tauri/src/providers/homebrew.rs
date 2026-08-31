use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tokio::process::Command;

use crate::domain::{
    ComponentId, Executable, Installation, InstallationId, InstallationScope, PackageCoordinate,
    PackageKind, ProviderId,
};
use crate::version::{ComponentStatus, StatusReason, VersionValue};

use super::{
    ProviderCapabilities, ProviderError, ProviderOperation, ProviderStatus, ProviderUpdatePlan,
    ProviderUpdateRequest, Recoverability, ToolProvider, UpdateCheck,
};

pub const HOMEBREW_PROVIDER_ID: &str = "homebrew";

#[derive(Debug, Clone)]
pub struct HomebrewProviderOptions {
    pub command_timeout: Duration,
    pub update_cache_ttl: Duration,
}

impl Default for HomebrewProviderOptions {
    fn default() -> Self {
        Self {
            command_timeout: Duration::from_secs(60),
            update_cache_ttl: Duration::from_secs(15 * 60),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrewEnvironment {
    pub brew_path: PathBuf,
    pub brew_version: VersionValue,
    pub prefix: PathBuf,
    pub architecture: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrewDiagnostic {
    pub installation_id: Option<InstallationId>,
    pub package: Option<String>,
    pub code: String,
    pub summary: String,
    pub recoverability: Recoverability,
}

#[derive(Debug, Clone, Default)]
struct BrewPackageState {
    pinned: bool,
    disabled: bool,
}

#[derive(Debug, Clone)]
struct CachedUpdate {
    checked_at: Instant,
    installed_version: Option<String>,
    check: UpdateCheck,
}

#[derive(Clone)]
pub struct HomebrewProvider {
    id: ProviderId,
    brew_path: PathBuf,
    options: HomebrewProviderOptions,
    environment: Arc<Mutex<Option<BrewEnvironment>>>,
    diagnostics: Arc<Mutex<Vec<BrewDiagnostic>>>,
    packages: Arc<Mutex<BTreeMap<InstallationId, BrewPackageState>>>,
    update_cache: Arc<Mutex<BTreeMap<InstallationId, CachedUpdate>>>,
}

impl HomebrewProvider {
    pub fn new(brew_path: impl Into<PathBuf>) -> Self {
        Self::with_options(brew_path, HomebrewProviderOptions::default())
    }

    pub fn with_options(brew_path: impl Into<PathBuf>, options: HomebrewProviderOptions) -> Self {
        Self {
            id: ProviderId::new(HOMEBREW_PROVIDER_ID)
                .expect("the embedded Homebrew provider ID is valid"),
            brew_path: brew_path.into(),
            options,
            environment: Arc::new(Mutex::new(None)),
            diagnostics: Arc::new(Mutex::new(Vec::new())),
            packages: Arc::new(Mutex::new(BTreeMap::new())),
            update_cache: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    pub fn last_environment(&self) -> Option<BrewEnvironment> {
        self.environment
            .lock()
            .expect("Homebrew environment mutex is not poisoned")
            .clone()
    }

    pub fn diagnostics(&self) -> Vec<BrewDiagnostic> {
        self.diagnostics
            .lock()
            .expect("Homebrew diagnostics mutex is not poisoned")
            .clone()
    }

    pub fn clear_update_cache(&self) {
        self.update_cache
            .lock()
            .expect("Homebrew update cache mutex is not poisoned")
            .clear();
    }

    fn validate_program(&self, operation: ProviderOperation) -> Result<(), ProviderError> {
        if !self.brew_path.is_absolute() || !self.brew_path.is_file() {
            return Err(ProviderError::new(
                self.id.clone(),
                operation,
                Recoverability::RequiresConfiguration,
                format!(
                    "brew executable must be an existing absolute file: {}",
                    self.brew_path.display()
                ),
            ));
        }
        Ok(())
    }

    async fn run_brew<I, S>(
        &self,
        operation: ProviderOperation,
        args: I,
    ) -> Result<BrewCommandOutput, ProviderError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        self.validate_program(operation)?;
        let mut command = Command::new(&self.brew_path);
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let output = tokio::time::timeout(self.options.command_timeout, command.output())
            .await
            .map_err(|_| {
                ProviderError::new(
                    self.id.clone(),
                    operation,
                    Recoverability::Retryable,
                    format!(
                        "brew command timed out after {} seconds",
                        self.options.command_timeout.as_secs()
                    ),
                )
            })?
            .map_err(|error| {
                ProviderError::new(
                    self.id.clone(),
                    operation,
                    Recoverability::Retryable,
                    format!("could not run brew: {error}"),
                )
            })?;
        Ok(BrewCommandOutput {
            code: output.status.code(),
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }

    async fn run_required(
        &self,
        operation: ProviderOperation,
        args: &[&str],
    ) -> Result<BrewCommandOutput, ProviderError> {
        let output = self.run_brew(operation, args).await?;
        if output.code != Some(0) {
            return Err(command_failure(&self.id, operation, &output));
        }
        Ok(output)
    }

    async fn load_environment(
        &self,
        operation: ProviderOperation,
        refresh: bool,
    ) -> Result<BrewEnvironment, ProviderError> {
        if !refresh {
            if let Some(environment) = self.last_environment() {
                return Ok(environment);
            }
        }
        let version_output = self.run_required(operation, &["--version"]).await?;
        let version_line = single_line(&version_output.stdout).ok_or_else(|| {
            parse_error(
                &self.id,
                operation,
                "brew returned an invalid version",
                &version_output,
            )
        })?;
        let version = version_line
            .strip_prefix("Homebrew ")
            .unwrap_or(&version_line)
            .to_owned();
        let prefix_output = self.run_required(operation, &["--prefix"]).await?;
        let prefix = PathBuf::from(single_line(&prefix_output.stdout).ok_or_else(|| {
            parse_error(
                &self.id,
                operation,
                "brew returned an invalid prefix",
                &prefix_output,
            )
        })?);
        if !prefix.is_absolute() {
            return Err(ProviderError::new(
                self.id.clone(),
                operation,
                Recoverability::RequiresConfiguration,
                "brew returned a non-absolute prefix",
            ));
        }
        let config_output = self.run_required(operation, &["config"]).await?;
        let config = String::from_utf8_lossy(&config_output.stdout);
        let architecture = config
            .lines()
            .find_map(|line| {
                line.strip_prefix("CPU:")
                    .or_else(|| line.strip_prefix("Architecture:"))
                    .map(str::trim)
            })
            .filter(|value| !value.is_empty())
            .unwrap_or("unknown")
            .to_owned();
        let environment = BrewEnvironment {
            brew_path: self.brew_path.clone(),
            brew_version: VersionValue::new(version),
            prefix,
            architecture,
        };
        *self
            .environment
            .lock()
            .expect("Homebrew environment mutex is not poisoned") = Some(environment.clone());
        Ok(environment)
    }

    fn reset_scan_state(&self) {
        self.diagnostics
            .lock()
            .expect("Homebrew diagnostics mutex is not poisoned")
            .clear();
        self.packages
            .lock()
            .expect("Homebrew package mutex is not poisoned")
            .clear();
    }

    fn record_diagnostic(&self, diagnostic: BrewDiagnostic) {
        self.diagnostics
            .lock()
            .expect("Homebrew diagnostics mutex is not poisoned")
            .push(diagnostic);
    }

    fn formula_installation(
        &self,
        environment: &BrewEnvironment,
        value: &Value,
    ) -> Option<Installation> {
        let object = value.as_object()?;
        let name = package_name(object, "full_name", "name")?;
        let versions = formula_installed_versions(object);
        let linked = object.get("linked_keg").and_then(Value::as_str);
        let installed_version = linked
            .and_then(|linked| versions.iter().find(|version| version.as_str() == linked))
            .or_else(|| versions.last())
            .cloned()
            .map(VersionValue::new);
        let pinned = truthy(object.get("pinned"));
        let disabled = truthy(object.get("disabled"));
        let deprecated = truthy(object.get("deprecated"));
        let keg_only = truthy(object.get("keg_only"));
        let mut installation = make_installation(&self.id, PackageKind::HomebrewFormula, &name)?;
        installation.installed_version = installed_version;
        installation.install_path = linked.map(|version| {
            environment
                .prefix
                .join("Cellar")
                .join(formula_basename(&name))
                .join(version)
        });
        self.packages
            .lock()
            .expect("Homebrew package mutex is not poisoned")
            .insert(
                installation.id.clone(),
                BrewPackageState { pinned, disabled },
            );
        for (active, code, summary) in [
            (
                pinned,
                "pinned",
                "formula is pinned and will not be upgraded",
            ),
            (keg_only, "keg_only", "formula is keg-only"),
            (deprecated, "deprecated", "formula is deprecated"),
            (disabled, "disabled", "formula is disabled"),
            (
                versions.len() > 1,
                "multiple_versions",
                "formula has multiple installed versions",
            ),
        ] {
            if active {
                self.record_diagnostic(BrewDiagnostic {
                    installation_id: Some(installation.id.clone()),
                    package: Some(name.clone()),
                    code: code.into(),
                    summary: summary.into(),
                    recoverability: if matches!(code, "pinned" | "disabled") {
                        Recoverability::RequiresConfiguration
                    } else {
                        Recoverability::Permanent
                    },
                });
            }
        }
        Some(installation)
    }

    async fn formula_executables(
        &self,
        environment: &BrewEnvironment,
        installations: &[Installation],
    ) -> BTreeMap<InstallationId, Vec<Executable>> {
        if installations.is_empty() {
            return BTreeMap::new();
        }
        let mut args = vec!["list".to_owned(), "--formula".to_owned(), "--".to_owned()];
        args.extend(
            installations
                .iter()
                .map(|installation| installation.package.name.clone()),
        );
        let output = match self.run_brew(ProviderOperation::Scan, args).await {
            Ok(output) if output.code == Some(0) => output,
            Ok(output) => {
                self.record_diagnostic(BrewDiagnostic {
                    installation_id: None,
                    package: None,
                    code: "executables_unavailable".into(),
                    summary: format!(
                        "brew list could not enumerate executables: {}",
                        sanitized_output(&output.stderr)
                    ),
                    recoverability: classify_recoverability(&output.stderr),
                });
                return BTreeMap::new();
            }
            Err(error) => {
                self.record_diagnostic(BrewDiagnostic {
                    installation_id: None,
                    package: None,
                    code: "executables_unavailable".into(),
                    summary: error.summary,
                    recoverability: error.recoverability,
                });
                return BTreeMap::new();
            }
        };
        let installation_ids = installations.iter().fold(
            BTreeMap::<String, Option<InstallationId>>::new(),
            |mut result, installation| {
                result
                    .entry(formula_basename(&installation.package.name).to_owned())
                    .and_modify(|id| *id = None)
                    .or_insert_with(|| Some(installation.id.clone()));
                result
            },
        );
        let cellar = environment.prefix.join("Cellar");
        let mut executables = BTreeMap::<InstallationId, Vec<Executable>>::new();
        for path in String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::trim)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
        {
            let Some(parent) = path
                .parent()
                .and_then(Path::file_name)
                .and_then(|name| name.to_str())
            else {
                continue;
            };
            if !matches!(parent, "bin" | "sbin") {
                continue;
            }
            let Some(cellar_name) = path
                .strip_prefix(&cellar)
                .ok()
                .and_then(|relative| relative.components().next())
                .and_then(|component| component.as_os_str().to_str())
            else {
                continue;
            };
            let Some(Some(installation_id)) = installation_ids.get(cellar_name) else {
                continue;
            };
            let Some(name) = path
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::to_owned)
            else {
                continue;
            };
            executables
                .entry(installation_id.clone())
                .or_default()
                .push(Executable { name, path });
        }
        executables
    }

    fn cask_installation(
        &self,
        environment: &BrewEnvironment,
        value: &Value,
    ) -> Option<Installation> {
        let object = value.as_object()?;
        let name = package_name(object, "full_token", "token")?;
        let versions = cask_installed_versions(object);
        let disabled = truthy(object.get("disabled"));
        let deprecated = truthy(object.get("deprecated"));
        let mut installation = make_installation(&self.id, PackageKind::HomebrewCask, &name)?;
        installation.installed_version = versions.last().cloned().map(VersionValue::new);
        installation.install_path = Some(
            environment
                .prefix
                .join("Caskroom")
                .join(cask_basename(&name)),
        );
        installation.set_executables(cask_executables(
            object.get("artifacts"),
            &environment.prefix,
        ));
        self.packages
            .lock()
            .expect("Homebrew package mutex is not poisoned")
            .insert(
                installation.id.clone(),
                BrewPackageState {
                    pinned: false,
                    disabled,
                },
            );
        for (active, code, summary) in [
            (deprecated, "deprecated", "cask is deprecated"),
            (disabled, "disabled", "cask is disabled"),
            (
                versions.len() > 1,
                "multiple_versions",
                "cask has multiple installed versions",
            ),
        ] {
            if active {
                self.record_diagnostic(BrewDiagnostic {
                    installation_id: Some(installation.id.clone()),
                    package: Some(name.clone()),
                    code: code.into(),
                    summary: summary.into(),
                    recoverability: if code == "disabled" {
                        Recoverability::RequiresConfiguration
                    } else {
                        Recoverability::Permanent
                    },
                });
            }
        }
        Some(installation)
    }

    fn cached_check(&self, installation: &Installation, fresh_only: bool) -> Option<UpdateCheck> {
        let installed_version = installation
            .installed_version
            .as_ref()
            .map(|version| version.raw().to_owned());
        let cache = self
            .update_cache
            .lock()
            .expect("Homebrew update cache mutex is not poisoned");
        let cached = cache.get(&installation.id)?;
        if cached.installed_version != installed_version
            || (fresh_only && cached.checked_at.elapsed() >= self.options.update_cache_ttl)
        {
            return None;
        }
        Some(cached.check.clone())
    }

    fn cache_check(&self, installation: &Installation, check: &UpdateCheck) {
        self.update_cache
            .lock()
            .expect("Homebrew update cache mutex is not poisoned")
            .insert(
                installation.id.clone(),
                CachedUpdate {
                    checked_at: Instant::now(),
                    installed_version: installation
                        .installed_version
                        .as_ref()
                        .map(|version| version.raw().to_owned()),
                    check: check.clone(),
                },
            );
    }
}

#[async_trait]
impl ToolProvider for HomebrewProvider {
    fn id(&self) -> ProviderId {
        self.id.clone()
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            full_scan: true,
            check_updates: true,
            update: true,
            version_pinning: false,
            multiple_versions: true,
            executables: true,
            hierarchical_components: false,
        }
    }

    async fn probe(&self) -> Result<ProviderStatus, ProviderError> {
        let environment = self
            .load_environment(ProviderOperation::Probe, true)
            .await?;
        Ok(ProviderStatus {
            available: true,
            version: Some(environment.brew_version.clone()),
            detail: Some(format!(
                "path={}; prefix={}; architecture={}",
                environment.brew_path.display(),
                environment.prefix.display(),
                environment.architecture
            )),
        })
    }

    async fn scan(&self) -> Result<Vec<Installation>, ProviderError> {
        self.reset_scan_state();
        let environment = self
            .load_environment(ProviderOperation::Scan, false)
            .await?;
        let output = self
            .run_brew(
                ProviderOperation::Scan,
                ["info", "--json=v2", "--installed"],
            )
            .await?;
        if output.code != Some(0) {
            return Err(command_failure(&self.id, ProviderOperation::Scan, &output));
        }
        let root: Value = serde_json::from_slice(&output.stdout).map_err(|error| {
            parse_error(
                &self.id,
                ProviderOperation::Scan,
                format!("brew info returned malformed JSON: {error}"),
                &output,
            )
        })?;
        let object = root.as_object().ok_or_else(|| {
            parse_error(
                &self.id,
                ProviderOperation::Scan,
                "brew info JSON was not an object",
                &output,
            )
        })?;
        let formulae = required_array(object, "formulae", &self.id, ProviderOperation::Scan)?;
        let casks = required_array(object, "casks", &self.id, ProviderOperation::Scan)?;
        let mut installations = Vec::with_capacity(formulae.len() + casks.len());
        let mut formula_installations = Vec::with_capacity(formulae.len());
        for formula in formulae {
            if formula_is_dependency_only(formula) {
                continue;
            }
            match self.formula_installation(&environment, formula) {
                Some(installation) => formula_installations.push(installation),
                None => self.record_diagnostic(BrewDiagnostic {
                    installation_id: None,
                    package: None,
                    code: "invalid_formula".into(),
                    summary: "brew returned a formula without a valid name; it was skipped".into(),
                    recoverability: Recoverability::Permanent,
                }),
            }
        }
        let mut formula_executables = self
            .formula_executables(&environment, &formula_installations)
            .await;
        for mut installation in formula_installations {
            if let Some(executables) = formula_executables.remove(&installation.id) {
                installation.set_executables(executables);
            }
            installations.push(installation);
        }
        for cask in casks {
            match self.cask_installation(&environment, cask) {
                Some(installation) => installations.push(installation),
                None => self.record_diagnostic(BrewDiagnostic {
                    installation_id: None,
                    package: None,
                    code: "invalid_cask".into(),
                    summary: "brew returned a cask without a valid token; it was skipped".into(),
                    recoverability: Recoverability::Permanent,
                }),
            }
        }
        installations.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(installations)
    }

    async fn check_updates(
        &self,
        installations: &[Installation],
    ) -> Result<Vec<UpdateCheck>, ProviderError> {
        let mut checks = vec![None; installations.len()];
        let mut pending = Vec::new();
        for (index, installation) in installations.iter().enumerate() {
            if installation.provider_id != self.id
                || !matches!(
                    installation.package.kind,
                    PackageKind::HomebrewFormula | PackageKind::HomebrewCask
                )
            {
                checks[index] = Some(failed_check(
                    installation,
                    "invalid_installation",
                    "installation does not belong to this Homebrew provider",
                    false,
                ));
            } else if let Some(cached) = self.cached_check(installation, true) {
                checks[index] = Some(cached);
            } else {
                pending.push((index, installation));
            }
        }
        if pending.is_empty() {
            return Ok(checks.into_iter().flatten().collect());
        }

        let output = self
            .run_brew(ProviderOperation::CheckUpdates, ["outdated", "--json=v2"])
            .await;
        let outdated = output.and_then(|output| parse_outdated(&self.id, output));
        match outdated {
            Ok(outdated) => {
                for (index, installation) in pending {
                    let entries = match installation.package.kind {
                        PackageKind::HomebrewFormula => outdated.formulae.as_ref(),
                        PackageKind::HomebrewCask => outdated.casks.as_ref(),
                        _ => None,
                    };
                    let check = match entries {
                        None => failed_check(
                            installation,
                            "outdated_kind_missing",
                            "brew outdated omitted this package kind",
                            true,
                        ),
                        Some(entries) => {
                            let state = self
                                .packages
                                .lock()
                                .expect("Homebrew package mutex is not poisoned")
                                .get(&installation.id)
                                .cloned()
                                .unwrap_or_default();
                            if state.pinned {
                                unsupported_check(
                                    installation,
                                    "pinned",
                                    "formula is pinned and cannot be upgraded automatically",
                                )
                            } else if state.disabled {
                                unsupported_check(
                                    installation,
                                    "disabled",
                                    "package is disabled and cannot be upgraded",
                                )
                            } else if let Some(entry) = entries.get(&installation.package.name) {
                                match entry
                                    .as_object()
                                    .and_then(|entry| entry.get("current_version"))
                                    .and_then(Value::as_str)
                                    .filter(|version| !version.trim().is_empty())
                                {
                                    Some(version) => successful_check(
                                        installation,
                                        Some(VersionValue::new(version)),
                                    ),
                                    None => failed_check(
                                        installation,
                                        "outdated_version_missing",
                                        "brew outdated omitted current_version",
                                        true,
                                    ),
                                }
                            } else {
                                successful_check(
                                    installation,
                                    installation.installed_version.clone(),
                                )
                            }
                        }
                    };
                    if !matches!(check.status, ComponentStatus::CheckFailed { .. }) {
                        self.cache_check(installation, &check);
                    }
                    checks[index] = Some(check);
                }
            }
            Err(error) => {
                for (index, installation) in pending {
                    checks[index] =
                        Some(self.cached_check(installation, false).unwrap_or_else(|| {
                            failed_check(
                                installation,
                                "remote_check_failed",
                                &error.summary,
                                error.recoverability == Recoverability::Retryable,
                            )
                        }));
                }
            }
        }
        Ok(checks.into_iter().flatten().collect())
    }

    async fn plan_update(
        &self,
        request: ProviderUpdateRequest,
    ) -> Result<ProviderUpdatePlan, ProviderError> {
        let operation = ProviderOperation::PlanUpdate;
        self.validate_program(operation)?;
        if request.component_id.as_str() != "core"
            || request.strategy_id.as_str() != "provider-default"
        {
            return Err(ProviderError::new(
                self.id.clone(),
                operation,
                Recoverability::Permanent,
                "Homebrew provider only supports the core/provider-default strategy",
            ));
        }
        if request.target_version.is_some() {
            return Err(ProviderError::new(
                self.id.clone(),
                operation,
                Recoverability::Permanent,
                "Homebrew provider does not support target-version pinning",
            ));
        }
        let state = self
            .packages
            .lock()
            .expect("Homebrew package mutex is not poisoned")
            .get(&request.installation_id)
            .cloned()
            .ok_or_else(|| {
                ProviderError::new(
                    self.id.clone(),
                    operation,
                    Recoverability::RequiresConfiguration,
                    "installation must be scanned before planning an upgrade",
                )
            })?;
        if state.pinned || state.disabled {
            return Err(ProviderError::new(
                self.id.clone(),
                operation,
                Recoverability::RequiresConfiguration,
                if state.pinned {
                    "pinned formula must be unpinned explicitly before upgrade"
                } else {
                    "disabled package cannot be upgraded"
                },
            ));
        }
        let (kind, name) = parse_installation_id(&request.installation_id).ok_or_else(|| {
            ProviderError::new(
                self.id.clone(),
                operation,
                Recoverability::Permanent,
                "installation ID is not a Homebrew formula or cask",
            )
        })?;
        let kind_flag = match kind {
            PackageKind::HomebrewFormula => "--formula",
            PackageKind::HomebrewCask => "--cask",
            _ => unreachable!("Homebrew installation parser only returns formula or cask"),
        };
        Ok(ProviderUpdatePlan {
            provider_id: self.id.clone(),
            installation_id: request.installation_id,
            component_id: request.component_id,
            strategy_id: request.strategy_id,
            program: self.brew_path.display().to_string(),
            args: vec!["upgrade".into(), kind_flag.into(), "--".into(), name],
        })
    }
}

#[derive(Debug)]
struct BrewCommandOutput {
    code: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

#[derive(Debug)]
struct BrewOutdated {
    formulae: Option<BTreeMap<String, Value>>,
    casks: Option<BTreeMap<String, Value>>,
}

fn parse_outdated(
    provider_id: &ProviderId,
    output: BrewCommandOutput,
) -> Result<BrewOutdated, ProviderError> {
    if output.code != Some(0) {
        return Err(command_failure(
            provider_id,
            ProviderOperation::CheckUpdates,
            &output,
        ));
    }
    let value: Value = serde_json::from_slice(&output.stdout).map_err(|error| {
        parse_error(
            provider_id,
            ProviderOperation::CheckUpdates,
            format!("brew outdated returned malformed JSON: {error}"),
            &output,
        )
    })?;
    let object = value.as_object().ok_or_else(|| {
        parse_error(
            provider_id,
            ProviderOperation::CheckUpdates,
            "brew outdated JSON was not an object",
            &output,
        )
    })?;
    Ok(BrewOutdated {
        formulae: optional_outdated_entries(object.get("formulae"), "name"),
        casks: optional_outdated_entries(object.get("casks"), "token"),
    })
}

fn optional_outdated_entries(
    value: Option<&Value>,
    primary_name: &str,
) -> Option<BTreeMap<String, Value>> {
    let entries = value?.as_array()?;
    Some(
        entries
            .iter()
            .filter_map(|entry| {
                let object = entry.as_object()?;
                let name = if primary_name == "name" {
                    package_name(object, "full_name", "name")
                } else {
                    package_name(object, "full_token", "token").or_else(|| {
                        object
                            .get("name")
                            .and_then(Value::as_str)
                            .filter(|name| !name.trim().is_empty())
                            .map(str::to_owned)
                    })
                }?;
                Some((name, entry.clone()))
            })
            .collect(),
    )
}

fn required_array<'a>(
    object: &'a Map<String, Value>,
    field: &str,
    provider_id: &ProviderId,
    operation: ProviderOperation,
) -> Result<&'a Vec<Value>, ProviderError> {
    object.get(field).and_then(Value::as_array).ok_or_else(|| {
        ProviderError::new(
            provider_id.clone(),
            operation,
            Recoverability::Permanent,
            format!("brew JSON field {field} was missing or not an array"),
        )
    })
}

fn package_name(object: &Map<String, Value>, preferred: &str, fallback: &str) -> Option<String> {
    object
        .get(preferred)
        .and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty())
        .or_else(|| {
            object
                .get(fallback)
                .and_then(Value::as_str)
                .filter(|name| !name.trim().is_empty())
        })
        .map(str::to_owned)
}

fn formula_installed_versions(object: &Map<String, Value>) -> Vec<String> {
    object
        .get("installed")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|installed| {
            installed
                .as_object()
                .and_then(|installed| installed.get("version"))
                .and_then(Value::as_str)
        })
        .map(str::to_owned)
        .collect()
}

fn formula_is_dependency_only(value: &Value) -> bool {
    let Some(installed) = value
        .as_object()
        .and_then(|object| object.get("installed"))
        .and_then(Value::as_array)
    else {
        return false;
    };
    !installed.is_empty()
        && installed.iter().all(|receipt| {
            receipt
                .as_object()
                .and_then(|receipt| receipt.get("installed_on_request"))
                .and_then(Value::as_bool)
                == Some(false)
        })
}

fn cask_installed_versions(object: &Map<String, Value>) -> Vec<String> {
    match object.get("installed") {
        Some(Value::String(version)) if !version.trim().is_empty() => vec![version.clone()],
        Some(Value::Array(versions)) => versions
            .iter()
            .filter_map(Value::as_str)
            .filter(|version| !version.trim().is_empty())
            .map(str::to_owned)
            .collect(),
        _ => Vec::new(),
    }
}

fn truthy(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Bool(value)) => *value,
        Some(Value::String(value)) => !value.is_empty(),
        Some(Value::Number(value)) => value.as_u64().is_some_and(|value| value != 0),
        _ => false,
    }
}

fn make_installation(
    provider_id: &ProviderId,
    kind: PackageKind,
    name: &str,
) -> Option<Installation> {
    let package = PackageCoordinate::new(kind, name).ok()?;
    Installation::new(provider_id.clone(), package, InstallationScope::Global).ok()
}

fn formula_basename(name: &str) -> &str {
    name.rsplit('/').next().unwrap_or(name)
}

fn cask_basename(name: &str) -> &str {
    name.rsplit('/').next().unwrap_or(name)
}

fn cask_executables(artifacts: Option<&Value>, prefix: &Path) -> Vec<Executable> {
    let mut names = BTreeSet::new();
    for artifact in artifacts.and_then(Value::as_array).into_iter().flatten() {
        let Some(binary) = artifact
            .as_object()
            .and_then(|artifact| artifact.get("binary"))
        else {
            continue;
        };
        let Some(values) = binary.as_array() else {
            continue;
        };
        let target = values
            .iter()
            .find_map(|value| {
                value
                    .as_object()
                    .and_then(|value| value.get("target"))
                    .and_then(Value::as_str)
            })
            .and_then(|target| Path::new(target).file_name())
            .and_then(|target| target.to_str())
            .or_else(|| {
                values
                    .first()
                    .and_then(Value::as_str)
                    .and_then(|source| Path::new(source).file_name())
                    .and_then(|source| source.to_str())
            });
        if let Some(target) = target.filter(|target| !target.is_empty()) {
            names.insert(target.to_owned());
        }
    }
    names
        .into_iter()
        .map(|name| Executable {
            path: prefix.join("bin").join(&name),
            name,
        })
        .collect()
}

fn parse_installation_id(id: &InstallationId) -> Option<(PackageKind, String)> {
    if let Some(name) = id.as_str().strip_prefix("homebrew-formula:") {
        return Some((PackageKind::HomebrewFormula, unescape_coordinate(name)));
    }
    id.as_str()
        .strip_prefix("homebrew-cask:")
        .map(|name| (PackageKind::HomebrewCask, unescape_coordinate(name)))
}

fn unescape_coordinate(value: &str) -> String {
    value.replace("%3A", ":").replace("%25", "%")
}

fn successful_check(
    installation: &Installation,
    latest_version: Option<VersionValue>,
) -> UpdateCheck {
    let status = if installation.installed_version.is_none() {
        ComponentStatus::unknown(
            "installed_version_unavailable",
            "brew did not report an installed version",
        )
    } else {
        ComponentStatus::from_versions(
            installation.installed_version.as_ref(),
            latest_version.as_ref(),
        )
    };
    UpdateCheck {
        installation_id: installation.id.clone(),
        component_id: ComponentId::new("core").expect("the embedded component ID is valid"),
        installed_version: installation.installed_version.clone(),
        latest_version,
        status,
    }
}

fn failed_check(
    installation: &Installation,
    code: &str,
    summary: &str,
    retryable: bool,
) -> UpdateCheck {
    UpdateCheck {
        installation_id: installation.id.clone(),
        component_id: ComponentId::new("core").expect("the embedded component ID is valid"),
        installed_version: installation.installed_version.clone(),
        latest_version: None,
        status: ComponentStatus::CheckFailed {
            reason: StatusReason::new(code, super::redact_summary(summary), retryable),
        },
    }
}

fn unsupported_check(installation: &Installation, code: &str, summary: &str) -> UpdateCheck {
    UpdateCheck {
        installation_id: installation.id.clone(),
        component_id: ComponentId::new("core").expect("the embedded component ID is valid"),
        installed_version: installation.installed_version.clone(),
        latest_version: None,
        status: ComponentStatus::Unsupported {
            reason: StatusReason::new(code, summary, false),
        },
    }
}

fn single_line(bytes: &[u8]) -> Option<String> {
    let value = String::from_utf8_lossy(bytes);
    let mut lines = value.lines().filter(|line| !line.trim().is_empty());
    let line = lines.next()?.trim().to_owned();
    lines.next().is_none().then_some(line)
}

fn command_failure(
    provider_id: &ProviderId,
    operation: ProviderOperation,
    output: &BrewCommandOutput,
) -> ProviderError {
    ProviderError::new(
        provider_id.clone(),
        operation,
        classify_recoverability(&output.stderr),
        format!(
            "brew exited with {}; {}",
            output
                .code
                .map(|code| code.to_string())
                .unwrap_or_else(|| "no status".into()),
            sanitized_output(&output.stderr)
        ),
    )
}

fn parse_error(
    provider_id: &ProviderId,
    operation: ProviderOperation,
    summary: impl AsRef<str>,
    output: &BrewCommandOutput,
) -> ProviderError {
    ProviderError::new(
        provider_id.clone(),
        operation,
        Recoverability::Retryable,
        format!("{}; {}", summary.as_ref(), sanitized_output(&output.stderr)),
    )
}

fn classify_recoverability(stderr: &[u8]) -> Recoverability {
    let stderr = String::from_utf8_lossy(stderr).to_ascii_lowercase();
    if stderr.contains("permission denied")
        || stderr.contains("not writable")
        || stderr.contains("cannot write")
    {
        Recoverability::RequiresConfiguration
    } else {
        Recoverability::Retryable
    }
}

fn sanitized_output(bytes: &[u8]) -> String {
    const MAX_LENGTH: usize = 512;
    let value = String::from_utf8_lossy(bytes);
    super::redact_summary(value.trim())
        .chars()
        .take(MAX_LENGTH)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cask_binary_artifacts_support_default_and_explicit_targets() {
        let artifacts = serde_json::json!([
            { "binary": ["App.app/Contents/MacOS/tool"] },
            { "binary": ["App.app/Contents/MacOS/helper", { "target": "renamed" }] },
            { "app": ["App.app"] }
        ]);
        let executables = cask_executables(Some(&artifacts), Path::new("/opt/homebrew"));
        assert_eq!(
            executables
                .iter()
                .map(|executable| executable.name.as_str())
                .collect::<Vec<_>>(),
            ["renamed", "tool"]
        );
    }

    #[test]
    fn formula_and_cask_ids_parse_to_distinct_kinds() {
        let formula = InstallationId::new("homebrew-formula:shared").unwrap();
        let cask = InstallationId::new("homebrew-cask:shared").unwrap();
        assert_eq!(
            parse_installation_id(&formula),
            Some((PackageKind::HomebrewFormula, "shared".into()))
        );
        assert_eq!(
            parse_installation_id(&cask),
            Some((PackageKind::HomebrewCask, "shared".into()))
        );
    }
}
