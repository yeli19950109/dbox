use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::process::Command;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::domain::{
    ComponentId, Executable, Installation, InstallationId, InstallationScope, PackageCoordinate,
    PackageKind, ProviderId,
};
use crate::version::{ComponentStatus, StatusReason, VersionValue};

use super::{
    ProviderCapabilities, ProviderError, ProviderOperation, ProviderStatus, ProviderUpdatePlan,
    ProviderUpdateRequest, Recoverability, ToolProvider, UpdateCheck,
};

pub const NPM_GLOBAL_PROVIDER_ID: &str = "npm-global";

#[derive(Debug, Clone)]
pub struct NpmProviderOptions {
    pub command_timeout: Duration,
    pub update_cache_ttl: Duration,
    pub max_concurrency: usize,
    pub update_batch_size: usize,
}

impl Default for NpmProviderOptions {
    fn default() -> Self {
        Self {
            command_timeout: Duration::from_secs(30),
            update_cache_ttl: Duration::from_secs(15 * 60),
            max_concurrency: 4,
            update_batch_size: 64,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NpmEnvironment {
    pub npm_path: PathBuf,
    pub npm_version: VersionValue,
    pub global_prefix: PathBuf,
    pub global_root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NpmDiagnostic {
    pub package: Option<String>,
    pub code: String,
    pub summary: String,
    pub recoverability: Recoverability,
}

#[derive(Debug, Clone)]
struct CachedUpdate {
    checked_at: Instant,
    installed_version: Option<String>,
    check: UpdateCheck,
}

#[derive(Clone)]
pub struct NpmGlobalProvider {
    id: ProviderId,
    npm_path: PathBuf,
    options: NpmProviderOptions,
    environment: Arc<Mutex<Option<NpmEnvironment>>>,
    diagnostics: Arc<Mutex<Vec<NpmDiagnostic>>>,
    update_cache: Arc<Mutex<BTreeMap<InstallationId, CachedUpdate>>>,
    query_slots: Arc<Semaphore>,
}

impl NpmGlobalProvider {
    pub fn new(npm_path: impl Into<PathBuf>) -> Self {
        Self::with_options(npm_path, NpmProviderOptions::default())
    }

    pub fn with_options(npm_path: impl Into<PathBuf>, mut options: NpmProviderOptions) -> Self {
        options.max_concurrency = options.max_concurrency.max(1);
        options.update_batch_size = options.update_batch_size.max(1);
        Self {
            id: ProviderId::new(NPM_GLOBAL_PROVIDER_ID)
                .expect("the embedded npm provider ID is valid"),
            npm_path: npm_path.into(),
            query_slots: Arc::new(Semaphore::new(options.max_concurrency)),
            options,
            environment: Arc::new(Mutex::new(None)),
            diagnostics: Arc::new(Mutex::new(Vec::new())),
            update_cache: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    pub fn last_environment(&self) -> Option<NpmEnvironment> {
        self.environment
            .lock()
            .expect("npm environment mutex is not poisoned")
            .clone()
    }

    pub fn diagnostics(&self) -> Vec<NpmDiagnostic> {
        self.diagnostics
            .lock()
            .expect("npm diagnostics mutex is not poisoned")
            .clone()
    }

    pub fn clear_update_cache(&self) {
        self.update_cache
            .lock()
            .expect("npm update cache mutex is not poisoned")
            .clear();
    }

    async fn load_environment(
        &self,
        operation: ProviderOperation,
        refresh: bool,
    ) -> Result<NpmEnvironment, ProviderError> {
        if !refresh {
            if let Some(environment) = self.last_environment() {
                return Ok(environment);
            }
        }

        self.validate_program(operation)?;
        let version = self
            .run_required(operation, &["--version"])
            .await
            .and_then(|output| {
                parse_single_line(&output.stdout, "npm version", &self.id, operation)
            })?;
        let prefix = self
            .run_required(operation, &["prefix", "--global"])
            .await
            .and_then(|output| {
                parse_absolute_path(&output.stdout, "global prefix", &self.id, operation)
            })?;
        let root = self
            .run_required(operation, &["root", "--global"])
            .await
            .and_then(|output| {
                parse_absolute_path(&output.stdout, "global root", &self.id, operation)
            })?;

        let environment = NpmEnvironment {
            npm_path: self.npm_path.clone(),
            npm_version: VersionValue::new(version),
            global_prefix: prefix,
            global_root: root,
        };
        *self
            .environment
            .lock()
            .expect("npm environment mutex is not poisoned") = Some(environment.clone());
        Ok(environment)
    }

    fn validate_program(&self, operation: ProviderOperation) -> Result<(), ProviderError> {
        if !self.npm_path.is_absolute() || !self.npm_path.is_file() {
            return Err(ProviderError::new(
                self.id.clone(),
                operation,
                Recoverability::RequiresConfiguration,
                format!(
                    "npm executable must be an existing absolute file: {}",
                    self.npm_path.display()
                ),
            ));
        }
        Ok(())
    }

    async fn run_required(
        &self,
        operation: ProviderOperation,
        args: &[&str],
    ) -> Result<NpmCommandOutput, ProviderError> {
        let output = self.run_npm(operation, args.iter().copied()).await?;
        if !output.success() {
            return Err(command_failure(
                &self.id,
                operation,
                &output,
                Recoverability::Retryable,
            ));
        }
        Ok(output)
    }

    async fn run_npm<I, S>(
        &self,
        operation: ProviderOperation,
        args: I,
    ) -> Result<NpmCommandOutput, ProviderError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        self.validate_program(operation)?;
        let mut command = Command::new(&self.npm_path);
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
                        "npm command timed out after {} seconds",
                        self.options.command_timeout.as_secs()
                    ),
                )
            })?
            .map_err(|error| {
                ProviderError::new(
                    self.id.clone(),
                    operation,
                    Recoverability::Retryable,
                    format!("could not run npm: {error}"),
                )
            })?;
        Ok(NpmCommandOutput {
            code: output.status.code(),
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }

    fn reset_diagnostics(&self) {
        self.diagnostics
            .lock()
            .expect("npm diagnostics mutex is not poisoned")
            .clear();
    }

    fn record_diagnostic(&self, diagnostic: NpmDiagnostic) {
        self.diagnostics
            .lock()
            .expect("npm diagnostics mutex is not poisoned")
            .push(diagnostic);
    }

    async fn installation_from_entry(
        &self,
        environment: &NpmEnvironment,
        package_name: String,
        entry: Value,
    ) -> Result<Installation, ProviderError> {
        if !valid_package_name(&package_name) {
            return Err(ProviderError::new(
                self.id.clone(),
                ProviderOperation::Scan,
                Recoverability::Permanent,
                format!("npm returned an invalid package name: {package_name:?}"),
            ));
        }

        let entry_object = entry.as_object();
        if entry_object.is_none() {
            self.record_diagnostic(NpmDiagnostic {
                package: Some(package_name.clone()),
                code: "invalid_list_entry".into(),
                summary: "npm list entry was not an object; the package was retained".into(),
                recoverability: Recoverability::Retryable,
            });
        }
        let listed_version = entry_object
            .and_then(|object| object.get("version"))
            .and_then(Value::as_str)
            .filter(|version| !version.trim().is_empty())
            .map(VersionValue::new);
        let package_path = entry_object
            .and_then(|object| object.get("path"))
            .and_then(Value::as_str)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .unwrap_or_else(|| environment.global_root.join(&package_name));

        if let Some(object) = entry_object {
            for (field, code) in [
                ("extraneous", "extraneous_package"),
                ("missing", "missing_package"),
                ("invalid", "invalid_package"),
            ] {
                if object.get(field).and_then(Value::as_bool) == Some(true) {
                    self.record_diagnostic(NpmDiagnostic {
                        package: Some(package_name.clone()),
                        code: code.into(),
                        summary: format!(
                            "npm marked {package_name} as {field}; the package was retained"
                        ),
                        recoverability: Recoverability::RequiresConfiguration,
                    });
                }
            }
        }

        let metadata = self.read_metadata(&package_name, &package_path).await;
        let metadata_version = metadata
            .as_ref()
            .and_then(|metadata| metadata.version.as_deref())
            .filter(|version| !version.trim().is_empty())
            .map(VersionValue::new);
        let bin_names = metadata
            .as_ref()
            .map(|metadata| metadata.bin_names(&package_name))
            .unwrap_or_default();

        let package = PackageCoordinate::new(PackageKind::NpmGlobal, package_name.clone())
            .map_err(|error| {
                ProviderError::new(
                    self.id.clone(),
                    ProviderOperation::Scan,
                    Recoverability::Permanent,
                    error.to_string(),
                )
            })?;
        let mut installation =
            Installation::new(self.id.clone(), package, InstallationScope::Global).map_err(
                |error| {
                    ProviderError::new(
                        self.id.clone(),
                        ProviderOperation::Scan,
                        Recoverability::Permanent,
                        error.to_string(),
                    )
                },
            )?;
        installation.id = installation_id(&self.id, &environment.global_root, &package_name)?;
        installation.install_path = Some(package_path);
        installation.installed_version = listed_version.or(metadata_version);
        installation.set_executables(bin_names.into_iter().map(|name| Executable {
            path: global_bin_path(&environment.global_prefix, &name),
            name,
        }));
        Ok(installation)
    }

    async fn read_metadata(&self, package_name: &str, package_path: &Path) -> Option<NpmMetadata> {
        let metadata_path = package_path.join("package.json");
        let contents = match tokio::fs::read(&metadata_path).await {
            Ok(contents) => contents,
            Err(error) => {
                self.record_diagnostic(NpmDiagnostic {
                    package: Some(package_name.into()),
                    code: "metadata_unavailable".into(),
                    summary: format!(
                        "could not read {}: {}; the package was retained without bin metadata",
                        metadata_path.display(),
                        error
                    ),
                    recoverability: Recoverability::RequiresConfiguration,
                });
                return None;
            }
        };
        match serde_json::from_slice(&contents) {
            Ok(metadata) => Some(metadata),
            Err(error) => {
                self.record_diagnostic(NpmDiagnostic {
                    package: Some(package_name.into()),
                    code: "metadata_invalid".into(),
                    summary: format!(
                        "could not parse {}: {}; the package was retained without bin metadata",
                        metadata_path.display(),
                        error
                    ),
                    recoverability: Recoverability::Permanent,
                });
                None
            }
        }
    }

    fn cached_check(&self, installation: &Installation, fresh_only: bool) -> Option<UpdateCheck> {
        let installed_version = installation
            .installed_version
            .as_ref()
            .map(|version| version.raw().to_owned());
        let cache = self
            .update_cache
            .lock()
            .expect("npm update cache mutex is not poisoned");
        let entry = cache.get(&installation.id)?;
        if entry.installed_version != installed_version
            || (fresh_only && entry.checked_at.elapsed() >= self.options.update_cache_ttl)
        {
            return None;
        }
        Some(entry.check.clone())
    }

    fn cache_check(&self, installation: &Installation, check: &UpdateCheck) {
        self.update_cache
            .lock()
            .expect("npm update cache mutex is not poisoned")
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

    async fn query_outdated_chunk(
        &self,
        chunk: Vec<(usize, Installation)>,
    ) -> Vec<(usize, UpdateCheck)> {
        let permit = match self.query_slots.acquire().await {
            Ok(permit) => permit,
            Err(_) => {
                return chunk
                    .into_iter()
                    .map(|(index, installation)| {
                        (
                            index,
                            failed_check(
                                &installation,
                                "query_cancelled",
                                "npm update query was cancelled",
                                true,
                            ),
                        )
                    })
                    .collect();
            }
        };
        let mut args = vec![
            "outdated".to_owned(),
            "--global".to_owned(),
            "--depth=0".to_owned(),
            "--json".to_owned(),
            "--".to_owned(),
        ];
        args.extend(
            chunk
                .iter()
                .map(|(_, installation)| installation.package.name.clone()),
        );
        let output = self.run_npm(ProviderOperation::CheckUpdates, &args).await;
        drop(permit);

        let outdated = match output.and_then(|output| parse_outdated_output(&self.id, output)) {
            Ok(outdated) => outdated,
            Err(error) => {
                return chunk
                    .into_iter()
                    .map(|(index, installation)| {
                        let check = self.cached_check(&installation, false).unwrap_or_else(|| {
                            failed_check(
                                &installation,
                                "registry_unavailable",
                                &error.summary,
                                error.recoverability == Recoverability::Retryable,
                            )
                        });
                        (index, check)
                    })
                    .collect();
            }
        };

        chunk
            .into_iter()
            .map(|(index, installation)| {
                let check = match outdated.get(&installation.package.name) {
                    None => successful_check(&installation, installation.installed_version.clone()),
                    Some(entry) => match entry
                        .as_object()
                        .and_then(|entry| entry.get("latest"))
                        .and_then(Value::as_str)
                        .filter(|version| !version.trim().is_empty())
                    {
                        Some(latest) => {
                            successful_check(&installation, Some(VersionValue::new(latest)))
                        }
                        None => failed_check(
                            &installation,
                            "invalid_outdated_entry",
                            "npm outdated did not return a valid latest version",
                            true,
                        ),
                    },
                };
                if !matches!(check.status, ComponentStatus::CheckFailed { .. }) {
                    self.cache_check(&installation, &check);
                }
                (index, check)
            })
            .collect()
    }
}

#[async_trait]
impl ToolProvider for NpmGlobalProvider {
    fn id(&self) -> ProviderId {
        self.id.clone()
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            full_scan: true,
            check_updates: true,
            update: true,
            version_pinning: true,
            multiple_versions: false,
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
            version: Some(environment.npm_version.clone()),
            detail: Some(format!(
                "path={}; global_prefix={}; global_root={}",
                environment.npm_path.display(),
                environment.global_prefix.display(),
                environment.global_root.display()
            )),
        })
    }

    async fn scan(&self) -> Result<Vec<Installation>, ProviderError> {
        self.reset_diagnostics();
        let environment = self
            .load_environment(ProviderOperation::Scan, false)
            .await?;
        let output = self
            .run_npm(
                ProviderOperation::Scan,
                ["ls", "--global", "--depth=0", "--json", "--long"],
            )
            .await?;
        let dependencies = parse_list_output(&self.id, output)?;
        let mut installations = Vec::with_capacity(dependencies.len());
        for (package_name, entry) in dependencies {
            match self
                .installation_from_entry(&environment, package_name.clone(), entry)
                .await
            {
                Ok(installation) => installations.push(installation),
                Err(error) => {
                    self.record_diagnostic(NpmDiagnostic {
                        package: Some(package_name),
                        code: "invalid_package".into(),
                        summary: error.summary,
                        recoverability: error.recoverability,
                    });
                }
            }
        }
        installations.sort_by(|left, right| left.package.name.cmp(&right.package.name));
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
                || installation.package.kind != PackageKind::NpmGlobal
                || !valid_package_name(&installation.package.name)
            {
                checks[index] = Some(failed_check(
                    installation,
                    "invalid_installation",
                    "installation does not belong to this npm global provider",
                    false,
                ));
            } else if let Some(cached) = self.cached_check(installation, true) {
                checks[index] = Some(cached);
            } else {
                pending.push((index, installation.clone()));
            }
        }

        let mut tasks = JoinSet::new();
        for chunk in pending.chunks(self.options.update_batch_size) {
            let provider = self.clone();
            let chunk = chunk.to_vec();
            tasks.spawn(async move { provider.query_outdated_chunk(chunk).await });
        }
        while let Some(result) = tasks.join_next().await {
            match result {
                Ok(results) => {
                    for (index, check) in results {
                        checks[index] = Some(check);
                    }
                }
                Err(error) => {
                    return Err(ProviderError::new(
                        self.id.clone(),
                        ProviderOperation::CheckUpdates,
                        Recoverability::Retryable,
                        format!("npm update query task failed: {error}"),
                    ));
                }
            }
        }

        Ok(checks
            .into_iter()
            .enumerate()
            .map(|(index, check)| {
                check.unwrap_or_else(|| {
                    failed_check(
                        &installations[index],
                        "query_incomplete",
                        "npm update query did not return a result",
                        true,
                    )
                })
            })
            .collect())
    }

    async fn plan_update(
        &self,
        request: ProviderUpdateRequest,
    ) -> Result<ProviderUpdatePlan, ProviderError> {
        let operation = ProviderOperation::PlanUpdate;
        if request.component_id.as_str() != "core"
            || request.strategy_id.as_str() != "provider-default"
        {
            return Err(ProviderError::new(
                self.id.clone(),
                operation,
                Recoverability::Permanent,
                "npm global provider only supports the core/provider-default strategy",
            ));
        }
        let environment = self.load_environment(operation, false).await?;
        let (source, package_name) =
            parse_installation_id(&request.installation_id).ok_or_else(|| {
                ProviderError::new(
                    self.id.clone(),
                    operation,
                    Recoverability::Permanent,
                    "installation ID is not a source-qualified npm global package",
                )
            })?;
        if source != source_key(&environment.global_root) || !valid_package_name(&package_name) {
            return Err(ProviderError::new(
                self.id.clone(),
                operation,
                Recoverability::RequiresConfiguration,
                "installation ID belongs to a different npm global root",
            ));
        }
        let target = request
            .target_version
            .as_ref()
            .map(VersionValue::raw)
            .unwrap_or("latest");
        if !valid_target(target) {
            return Err(ProviderError::new(
                self.id.clone(),
                operation,
                Recoverability::Permanent,
                "target version is not a valid npm version or dist-tag",
            ));
        }
        let package_spec = format!("{package_name}@{target}");
        Ok(ProviderUpdatePlan {
            provider_id: self.id.clone(),
            installation_id: request.installation_id,
            component_id: request.component_id,
            strategy_id: request.strategy_id,
            program: self.npm_path.display().to_string(),
            args: vec![
                "install".into(),
                "--global".into(),
                "--".into(),
                package_spec,
            ],
        })
    }
}

#[derive(Debug)]
struct NpmCommandOutput {
    code: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

impl NpmCommandOutput {
    fn success(&self) -> bool {
        self.code == Some(0)
    }
}

#[derive(Debug, Deserialize)]
struct NpmMetadata {
    version: Option<String>,
    bin: Option<Value>,
}

impl NpmMetadata {
    fn bin_names(&self, package_name: &str) -> BTreeSet<String> {
        match self.bin.as_ref() {
            Some(Value::String(path)) if !path.trim().is_empty() => package_name
                .rsplit('/')
                .next()
                .filter(|name| valid_bin_name(name))
                .map(str::to_owned)
                .into_iter()
                .collect(),
            Some(Value::Object(bins)) => bins
                .iter()
                .filter(|(name, path)| {
                    valid_bin_name(name)
                        && path.as_str().is_some_and(|path| !path.trim().is_empty())
                })
                .map(|(name, _)| name.clone())
                .collect(),
            _ => BTreeSet::new(),
        }
    }
}

fn parse_single_line(
    bytes: &[u8],
    field: &str,
    provider_id: &ProviderId,
    operation: ProviderOperation,
) -> Result<String, ProviderError> {
    let value = String::from_utf8_lossy(bytes);
    let lines: Vec<_> = value
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    if lines.len() != 1 {
        return Err(ProviderError::new(
            provider_id.clone(),
            operation,
            Recoverability::Permanent,
            format!("npm returned an invalid {field}"),
        ));
    }
    Ok(lines[0].trim().to_owned())
}

fn parse_absolute_path(
    bytes: &[u8],
    field: &str,
    provider_id: &ProviderId,
    operation: ProviderOperation,
) -> Result<PathBuf, ProviderError> {
    let value = PathBuf::from(parse_single_line(bytes, field, provider_id, operation)?);
    if !value.is_absolute() {
        return Err(ProviderError::new(
            provider_id.clone(),
            operation,
            Recoverability::RequiresConfiguration,
            format!("npm returned a non-absolute {field}"),
        ));
    }
    Ok(value)
}

fn parse_list_output(
    provider_id: &ProviderId,
    output: NpmCommandOutput,
) -> Result<BTreeMap<String, Value>, ProviderError> {
    let value: Value = serde_json::from_slice(&output.stdout).map_err(|error| {
        command_parse_failure(
            provider_id,
            ProviderOperation::Scan,
            &output,
            format!("npm list returned malformed JSON: {error}"),
        )
    })?;
    let object = value.as_object().ok_or_else(|| {
        command_parse_failure(
            provider_id,
            ProviderOperation::Scan,
            &output,
            "npm list JSON was not an object",
        )
    })?;
    if object.contains_key("error") && !object.contains_key("dependencies") {
        return Err(command_parse_failure(
            provider_id,
            ProviderOperation::Scan,
            &output,
            "npm list reported an error",
        ));
    }
    match object.get("dependencies") {
        None | Some(Value::Null) => Ok(BTreeMap::new()),
        Some(Value::Object(dependencies)) => Ok(dependencies
            .iter()
            .map(|(name, entry)| (name.clone(), entry.clone()))
            .collect()),
        Some(_) => Err(command_parse_failure(
            provider_id,
            ProviderOperation::Scan,
            &output,
            "npm list dependencies was not an object",
        )),
    }
}

fn parse_outdated_output(
    provider_id: &ProviderId,
    output: NpmCommandOutput,
) -> Result<BTreeMap<String, Value>, ProviderError> {
    let value: Value = serde_json::from_slice(&output.stdout).map_err(|error| {
        command_parse_failure(
            provider_id,
            ProviderOperation::CheckUpdates,
            &output,
            format!("npm outdated returned malformed JSON: {error}"),
        )
    })?;
    let object = value.as_object().ok_or_else(|| {
        command_parse_failure(
            provider_id,
            ProviderOperation::CheckUpdates,
            &output,
            "npm outdated JSON was not an object",
        )
    })?;
    if object.contains_key("error") || !matches!(output.code, Some(0 | 1)) {
        return Err(command_parse_failure(
            provider_id,
            ProviderOperation::CheckUpdates,
            &output,
            "npm outdated reported a registry error",
        ));
    }
    Ok(object
        .iter()
        .map(|(name, entry)| (name.clone(), entry.clone()))
        .collect())
}

fn command_failure(
    provider_id: &ProviderId,
    operation: ProviderOperation,
    output: &NpmCommandOutput,
    recoverability: Recoverability,
) -> ProviderError {
    ProviderError::new(
        provider_id.clone(),
        operation,
        recoverability,
        format!(
            "npm exited with {}; {}",
            output
                .code
                .map(|code| code.to_string())
                .unwrap_or_else(|| "no status".into()),
            sanitized_output(&output.stderr)
        ),
    )
}

fn command_parse_failure(
    provider_id: &ProviderId,
    operation: ProviderOperation,
    output: &NpmCommandOutput,
    summary: impl AsRef<str>,
) -> ProviderError {
    ProviderError::new(
        provider_id.clone(),
        operation,
        Recoverability::Retryable,
        format!("{}; {}", summary.as_ref(), sanitized_output(&output.stderr)),
    )
}

fn sanitized_output(bytes: &[u8]) -> String {
    const MAX_LENGTH: usize = 512;
    let value = String::from_utf8_lossy(bytes);
    let value = super::redact_summary(value.trim());
    value.chars().take(MAX_LENGTH).collect()
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

fn successful_check(
    installation: &Installation,
    latest_version: Option<VersionValue>,
) -> UpdateCheck {
    let status = if installation.installed_version.is_none() {
        ComponentStatus::unknown(
            "installed_version_unavailable",
            "npm did not report an installed version",
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

fn installation_id(
    provider_id: &ProviderId,
    root: &Path,
    package_name: &str,
) -> Result<InstallationId, ProviderError> {
    InstallationId::new(format!(
        "{}:{}:{}",
        provider_id,
        source_key(root),
        escape_id_segment(package_name)
    ))
    .map_err(|error| {
        ProviderError::new(
            provider_id.clone(),
            ProviderOperation::Scan,
            Recoverability::Permanent,
            error.to_string(),
        )
    })
}

fn parse_installation_id(id: &InstallationId) -> Option<(String, String)> {
    let rest = id
        .as_str()
        .strip_prefix(&format!("{NPM_GLOBAL_PROVIDER_ID}:"))?;
    let (source, package) = rest.split_once(':')?;
    Some((source.to_owned(), unescape_id_segment(package)?))
}

fn source_key(root: &Path) -> String {
    blake3::hash(root.to_string_lossy().as_bytes()).to_hex()[..16].to_owned()
}

fn escape_id_segment(value: &str) -> String {
    value.replace('%', "%25").replace(':', "%3A")
}

fn unescape_id_segment(value: &str) -> Option<String> {
    let mut result = String::with_capacity(value.len());
    let mut characters = value.chars();
    while let Some(character) = characters.next() {
        if character != '%' {
            result.push(character);
            continue;
        }
        let first = characters.next()?;
        let second = characters.next()?;
        match (first.to_ascii_uppercase(), second.to_ascii_uppercase()) {
            ('2', '5') => result.push('%'),
            ('3', 'A') => result.push(':'),
            _ => return None,
        }
    }
    Some(result)
}

fn valid_package_name(name: &str) -> bool {
    if name.is_empty()
        || name.len() > 214
        || name.trim() != name
        || name
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
        || name.contains('\\')
        || name.contains(':')
    {
        return false;
    }
    let valid_segment = |segment: &str| {
        !segment.is_empty()
            && !segment.starts_with(['.', '-'])
            && segment != "."
            && segment != ".."
            && !segment.contains('/')
    };
    if let Some(scoped) = name.strip_prefix('@') {
        let Some((scope, package)) = scoped.split_once('/') else {
            return false;
        };
        !package.contains('/') && valid_segment(scope) && valid_segment(package)
    } else {
        valid_segment(name)
    }
}

fn valid_target(target: &str) -> bool {
    !target.is_empty()
        && target.len() <= 256
        && target.trim() == target
        && !target.starts_with('-')
        && target
            .chars()
            .all(|character| !character.is_control() && !character.is_whitespace())
}

fn valid_bin_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\'])
        && !name.chars().any(char::is_control)
}

fn global_bin_path(prefix: &Path, name: &str) -> PathBuf {
    if cfg!(windows) {
        prefix.join(format!("{name}.cmd"))
    } else {
        prefix.join("bin").join(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_name_validation_handles_scopes_and_rejects_option_like_names() {
        assert!(valid_package_name("typescript"));
        assert!(valid_package_name("@scope/tool-name"));
        assert!(!valid_package_name("--global"));
        assert!(!valid_package_name("@scope"));
        assert!(!valid_package_name("@scope/tool/extra"));
        assert!(!valid_package_name("../tool"));
    }

    #[test]
    fn source_qualified_id_round_trips_scoped_packages() {
        let provider = ProviderId::new(NPM_GLOBAL_PROVIDER_ID).unwrap();
        let id =
            installation_id(&provider, Path::new("/prefix one/lib/node_modules"), "@s/p").unwrap();
        let (source, package) = parse_installation_id(&id).unwrap();
        assert_eq!(
            source,
            source_key(Path::new("/prefix one/lib/node_modules"))
        );
        assert_eq!(package, "@s/p");
    }

    #[test]
    fn metadata_supports_string_object_and_missing_bin() {
        let string: NpmMetadata =
            serde_json::from_str(r#"{"version":"1.0.0","bin":"cli.js"}"#).unwrap();
        assert_eq!(
            string.bin_names("@scope/tool"),
            BTreeSet::from(["tool".into()])
        );
        let multiple: NpmMetadata = serde_json::from_str(
            r#"{"bin":{"tool":"cli.js","tool-lsp":"lsp.js","bad/name":"bad.js"}}"#,
        )
        .unwrap();
        assert_eq!(
            multiple.bin_names("tool"),
            BTreeSet::from(["tool".into(), "tool-lsp".into()])
        );
        let missing: NpmMetadata = serde_json::from_str(r#"{"version":"1.0.0"}"#).unwrap();
        assert!(missing.bin_names("tool").is_empty());
    }
}
