//! Configuration-driven, read-only checks for macOS apps published on GitHub.
//! Only Info.plist and release metadata are read; apps and installers never run.
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use reqwest::{Client, StatusCode, Url};
use semver::Version;
use serde::Deserialize;

use super::{
    ProviderCapabilities, ProviderError, ProviderOperation, ProviderProgress,
    ProviderProgressCallback, ProviderStatus, ProviderUpdatePlan, ProviderUpdateRequest,
    Recoverability, ToolProvider, UpdateCheck,
};
use crate::domain::{
    ComponentId, Installation, InstallationId, InstallationScope, PackageCoordinate, PackageKind,
    ProviderId,
};
use crate::version::{ComponentStatus, StatusReason, VersionValue};

pub const GITHUB_RELEASES_PROVIDER_ID: &str = "github-releases";
const MAX_CONFIG_BYTES: u64 = 64 * 1024;
const MAX_METADATA_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AppConfig {
    schema_version: u32,
    id: String,
    display_name: String,
    app_path: PathBuf,
    bundle_id: String,
    repository: String,
}

#[derive(Clone)]
struct AppSnapshot {
    config: AppConfig,
    installation: Installation,
    issue: Option<StatusReason>,
}

#[derive(Clone)]
pub struct GitHubReleasesProvider {
    config_dir: PathBuf,
    client: Client,
    api_base: Url,
    request_timeout: Duration,
    scanned: Arc<Mutex<BTreeMap<InstallationId, AppSnapshot>>>,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
}

impl GitHubReleasesProvider {
    /// `config_dir` is the directory containing the provider's TOML app definitions.
    pub fn new(config_dir: PathBuf) -> Self {
        Self::create(
            config_dir,
            Url::parse("https://api.github.com/").expect("static GitHub URL is valid"),
            Duration::from_secs(15),
        )
    }

    fn create(config_dir: PathBuf, api_base: Url, request_timeout: Duration) -> Self {
        Self {
            config_dir,
            client: Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("read-only GitHub HTTP client can be constructed"),
            api_base,
            request_timeout,
            scanned: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    /// Inject a loopback fixture server without making release endpoints configurable
    /// in user TOML. Production construction always uses api.github.com.
    #[doc(hidden)]
    pub fn with_test_endpoint(
        config_dir: PathBuf,
        endpoint: &str,
        request_timeout: Duration,
    ) -> Result<Self, ProviderError> {
        let url = Url::parse(endpoint)
            .map_err(|_| provider_error(ProviderOperation::Probe, "Invalid fixture endpoint"))?;
        if url.scheme() != "http"
            || !matches!(url.host_str(), Some("127.0.0.1" | "[::1]"))
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path() != "/"
        {
            return Err(provider_error(
                ProviderOperation::Probe,
                "Fixture endpoint must be a loopback HTTP origin",
            ));
        }
        Ok(Self::create(config_dir, url, request_timeout))
    }

    async fn configs(&self, operation: ProviderOperation) -> Result<Vec<AppConfig>, ProviderError> {
        let mut entries = match tokio::fs::read_dir(&self.config_dir).await {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(_) => {
                return Err(provider_error(
                    operation,
                    "Could not read GitHub Releases providers.d directory",
                ))
            }
        };
        let mut paths = Vec::new();
        while let Some(entry) = entries.next_entry().await.map_err(|_| {
            provider_error(
                operation,
                "Could not enumerate GitHub Releases provider configurations",
            )
        })? {
            let path = entry.path();
            if path
                .extension()
                .is_some_and(|extension| extension == "toml")
            {
                paths.push(path);
            }
        }
        paths.sort();
        let mut ids = BTreeSet::new();
        let mut apps = BTreeSet::new();
        let mut configs = Vec::new();
        for path in paths {
            let label = path.file_name().unwrap_or_default().to_string_lossy();
            let bytes = read_limited(&path, MAX_CONFIG_BYTES).await.map_err(|_| {
                provider_error(
                    operation,
                    &format!("Could not read provider config {label} (maximum 64 KiB)"),
                )
            })?;
            let config: AppConfig = std::str::from_utf8(&bytes).ok()
                .and_then(|text| toml::from_str(text).ok())
                .ok_or_else(|| provider_error(operation, &format!("Invalid provider config {label}: expected schema_version, id, display_name, app_path, bundle_id, repository")))?;
            let issue = validate_config(&config);
            if let Some(issue) = issue {
                return Err(provider_error(
                    operation,
                    &format!("Invalid provider config {label}: {issue}"),
                ));
            }
            if !ids.insert(config.id.clone()) || !apps.insert(config.app_path.clone()) {
                return Err(provider_error(
                    operation,
                    &format!("Duplicate provider config id or app_path in {label}"),
                ));
            }
            configs.push(config);
        }
        Ok(configs)
    }

    async fn app_snapshot(&self, config: AppConfig) -> Result<Option<AppSnapshot>, ProviderError> {
        match tokio::fs::metadata(&config.app_path).await {
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => {
                return Err(provider_error(
                    ProviderOperation::Scan,
                    &format!(
                        "{} app_path is not an application directory",
                        config.display_name
                    ),
                ))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => {
                return Err(provider_error(
                    ProviderOperation::Scan,
                    &format!(
                        "Could not inspect {} application directory",
                        config.display_name
                    ),
                ))
            }
        }
        let mut installation = Installation::new(
            self.id(),
            PackageCoordinate::new(PackageKind::Other("macos_app".into()), &config.display_name)
                .expect("validated display name is a package coordinate"),
            if config.app_path.starts_with("/Applications") {
                InstallationScope::System
            } else {
                InstallationScope::User
            },
        )
        .expect("static provider and package IDs are valid");
        let path_hash = blake3::hash(config.app_path.as_os_str().as_encoded_bytes()).to_hex();
        installation.id = InstallationId::new(format!(
            "github-releases:{}:{}",
            config.id,
            &path_hash[..16]
        ))
        .expect("validated config ID is valid");
        installation.install_path = Some(config.app_path.clone());
        let result = read_limited(
            &config.app_path.join("Contents/Info.plist"),
            MAX_METADATA_BYTES,
        )
        .await
        .map_err(|_| {
            StatusReason::new(
                "app_metadata_unreadable",
                "Could not read application Info.plist (maximum 1 MiB)",
                false,
            )
        })
        .and_then(|bytes| {
            plist::Value::from_reader(std::io::Cursor::new(bytes)).map_err(|_| {
                StatusReason::new(
                    "app_metadata_invalid",
                    "Application Info.plist is invalid",
                    false,
                )
            })
        })
        .and_then(|value| {
            let values = value.as_dictionary().ok_or_else(|| {
                StatusReason::new(
                    "app_metadata_invalid",
                    "Application Info.plist must contain a dictionary",
                    false,
                )
            })?;
            if values
                .get("CFBundleIdentifier")
                .and_then(plist::Value::as_string)
                != Some(config.bundle_id.as_str())
            {
                return Err(StatusReason::new(
                    "app_bundle_mismatch",
                    "Application bundle identifier does not match the provider configuration",
                    false,
                ));
            }
            let raw = values
                .get("CFBundleShortVersionString")
                .and_then(plist::Value::as_string)
                .ok_or_else(|| {
                    StatusReason::new(
                        "app_version_missing",
                        "Application Info.plist has no CFBundleShortVersionString",
                        false,
                    )
                })?;
            Version::parse(raw).map_err(|_| {
                StatusReason::new(
                    "app_version_invalid",
                    "Application version is not a strict semantic version",
                    false,
                )
            })?;
            Ok(VersionValue::new(raw))
        });
        let issue = match result {
            Ok(version) => {
                installation.installed_version = Some(version);
                None
            }
            Err(issue) => Some(issue),
        };
        Ok(Some(AppSnapshot {
            config,
            installation,
            issue,
        }))
    }

    async fn latest(&self, repository: &str) -> Result<VersionValue, StatusReason> {
        let endpoint = self
            .api_base
            .join(&format!("repos/{repository}/releases/latest"))
            .expect("validated repository produces a valid GitHub URL");
        let response = self
            .client
            .get(endpoint)
            .header(
                reqwest::header::USER_AGENT,
                concat!("dbox/", env!("CARGO_PKG_VERSION")),
            )
            .header(reqwest::header::ACCEPT, "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .timeout(self.request_timeout)
            .send()
            .await
            .map_err(|error| {
                if error.is_timeout() {
                    StatusReason::new("github_timeout", "GitHub release check timed out", true)
                } else {
                    StatusReason::new(
                        "github_network_failed",
                        "Could not reach GitHub Releases",
                        true,
                    )
                }
            })?;
        let status = response.status();
        if status == StatusCode::TOO_MANY_REQUESTS
            || (status == StatusCode::FORBIDDEN
                && response
                    .headers()
                    .get("x-ratelimit-remaining")
                    .is_some_and(|value| value == "0"))
        {
            return Err(StatusReason::new(
                "github_rate_limited",
                "GitHub API rate limit reached; retry later",
                true,
            ));
        }
        if status == StatusCode::NOT_FOUND {
            return Err(StatusReason::new(
                "github_release_missing",
                "No public stable GitHub release was found; check the repository configuration",
                false,
            ));
        }
        if !status.is_success() {
            return Err(StatusReason::new(
                "github_http_failed",
                format!("GitHub release request failed (HTTP {})", status.as_u16()),
                status.is_server_error(),
            ));
        }
        if response
            .content_length()
            .is_some_and(|size| size > MAX_METADATA_BYTES)
        {
            return Err(StatusReason::new(
                "github_metadata_invalid",
                "GitHub release metadata exceeds 1 MiB",
                false,
            ));
        }
        let mut response = response;
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|error| {
            StatusReason::new(
                if error.is_timeout() {
                    "github_timeout"
                } else {
                    "github_network_failed"
                },
                "Could not read GitHub release metadata",
                true,
            )
        })? {
            if bytes.len() + chunk.len() > MAX_METADATA_BYTES as usize {
                return Err(StatusReason::new(
                    "github_metadata_invalid",
                    "GitHub release metadata exceeds 1 MiB",
                    false,
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        let release: Release = serde_json::from_slice(&bytes).map_err(|_| {
            StatusReason::new(
                "github_metadata_invalid",
                "GitHub returned invalid release metadata",
                false,
            )
        })?;
        if release.draft || release.prerelease {
            return Err(StatusReason::new(
                "github_release_unstable",
                "GitHub latest release is a draft or prerelease; no stable version was accepted",
                false,
            ));
        }
        let version = release
            .tag_name
            .strip_prefix('v')
            .unwrap_or(&release.tag_name);
        let parsed = Version::parse(version).map_err(|_| {
            StatusReason::new(
                "github_version_invalid",
                "GitHub release tag is not a semantic version (expected v1.2.3 or 1.2.3)",
                false,
            )
        })?;
        if !parsed.pre.is_empty() {
            return Err(StatusReason::new(
                "github_release_unstable",
                "GitHub release tag is a prerelease; no stable version was accepted",
                false,
            ));
        }
        Ok(VersionValue::new(version))
    }
}

#[async_trait]
impl ToolProvider for GitHubReleasesProvider {
    fn id(&self) -> ProviderId {
        ProviderId::new(GITHUB_RELEASES_PROVIDER_ID).expect("static provider ID is valid")
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            full_scan: true,
            check_updates: true,
            ..ProviderCapabilities::default()
        }
    }

    async fn probe(&self) -> Result<ProviderStatus, ProviderError> {
        let configs = self.configs(ProviderOperation::Probe).await?;
        Ok(ProviderStatus {
            available: true,
            version: None,
            detail: Some(format!(
                "{} configured applications; read-only Info.plist and GitHub stable release checks",
                configs.len()
            )),
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
            .expect("GitHub snapshot mutex is not poisoned")
            .clear();
        let configs = self.configs(ProviderOperation::Scan).await?;
        let total = configs.len();
        let mut snapshots = BTreeMap::new();
        on_progress(ProviderProgress::items(None, 0, total));
        for (index, config) in configs.into_iter().enumerate() {
            let name = config.display_name.clone();
            on_progress(ProviderProgress::items(Some(name.clone()), index, total));
            if let Some(snapshot) = self.app_snapshot(config).await? {
                snapshots.insert(snapshot.installation.id.clone(), snapshot);
            }
            on_progress(ProviderProgress::items(Some(name), index + 1, total));
        }
        let installations = snapshots
            .values()
            .map(|snapshot| snapshot.installation.clone())
            .collect();
        *self
            .scanned
            .lock()
            .expect("GitHub snapshot mutex is not poisoned") = snapshots;
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
        let snapshots = self
            .scanned
            .lock()
            .expect("GitHub snapshot mutex is not poisoned")
            .clone();
        let total = installations.len();
        let mut checks = Vec::with_capacity(total);
        on_progress(ProviderProgress::items(None, 0, total));
        for (index, installation) in installations.iter().enumerate() {
            on_progress(ProviderProgress::items(
                Some(installation.package.name.clone()),
                index,
                total,
            ));
            let result = match snapshots
                .get(&installation.id)
                .filter(|snapshot| &snapshot.installation == installation)
            {
                Some(snapshot) => match &snapshot.issue {
                    Some(issue) => Err(issue.clone()),
                    None => self.latest(&snapshot.config.repository).await,
                },
                None => Err(StatusReason::new(
                    "scan_required",
                    "Refresh configured applications before checking this installation",
                    false,
                )),
            };
            let (latest_version, status) = match result {
                Ok(latest) => {
                    let status = ComponentStatus::from_versions(
                        installation.installed_version.as_ref(),
                        Some(&latest),
                    );
                    (Some(latest), status)
                }
                Err(reason) => (None, ComponentStatus::CheckFailed { reason }),
            };
            checks.push(UpdateCheck {
                installation_id: installation.id.clone(),
                component_id: ComponentId::new("core").expect("static component ID is valid"),
                installed_version: installation.installed_version.clone(),
                latest_version,
                status,
            });
            on_progress(ProviderProgress::items(
                Some(installation.package.name.clone()),
                index + 1,
                total,
            ));
        }
        Ok(checks)
    }

    async fn plan_update(
        &self,
        _request: ProviderUpdateRequest,
    ) -> Result<ProviderUpdatePlan, ProviderError> {
        Err(ProviderError::new(self.id(), ProviderOperation::PlanUpdate, Recoverability::Permanent, "GitHub Releases provider only checks versions; update this application using its own updater"))
    }
}

async fn read_limited(path: &Path, max_bytes: u64) -> Result<Vec<u8>, std::io::Error> {
    use tokio::io::AsyncReadExt;
    let file = tokio::fs::File::open(path).await?;
    if !file.metadata().await?.is_file() {
        return Err(std::io::Error::other("expected a regular file"));
    }
    let mut bytes = Vec::new();
    file.take(max_bytes + 1).read_to_end(&mut bytes).await?;
    if bytes.len() as u64 > max_bytes {
        return Err(std::io::Error::other("file exceeds size limit"));
    }
    Ok(bytes)
}

fn provider_error(operation: ProviderOperation, summary: &str) -> ProviderError {
    ProviderError::new(
        ProviderId::new(GITHUB_RELEASES_PROVIDER_ID).expect("static provider ID is valid"),
        operation,
        Recoverability::RequiresConfiguration,
        summary,
    )
}

fn validate_config(config: &AppConfig) -> Option<&'static str> {
    if config.schema_version != 1 {
        return Some("schema_version must be 1");
    }
    if !identifier(&config.id) {
        return Some("id must contain only ASCII letters, digits, dots, hyphens or underscores");
    }
    if config.display_name.is_empty()
        || config.display_name.trim() != config.display_name
        || config.display_name.chars().any(char::is_control)
    {
        return Some("display_name must be non-empty, trimmed and contain no control characters");
    }
    if !config.app_path.is_absolute()
        || config
            .app_path
            .extension()
            .is_none_or(|extension| extension != "app")
        || config
            .app_path
            .components()
            .any(|component| component == std::path::Component::ParentDir)
    {
        return Some("app_path must be an absolute .app path without parent traversals");
    }
    if config.bundle_id.is_empty()
        || config.bundle_id.trim() != config.bundle_id
        || config.bundle_id.chars().any(char::is_control)
    {
        return Some("bundle_id must be non-empty, trimmed and contain no control characters");
    }
    let mut parts = config.repository.split('/');
    if !parts.next().is_some_and(identifier)
        || !parts.next().is_some_and(identifier)
        || parts.next().is_some()
    {
        return Some("repository must be owner/repo, without a URL or query string");
    }
    None
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
}
