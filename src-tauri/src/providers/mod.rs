use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use regex::Regex;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::domain::{ComponentId, Installation, InstallationId, ProviderId, StrategyId};
use crate::version::{ComponentStatus, VersionValue};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderCapabilities {
    pub full_scan: bool,
    pub check_updates: bool,
    pub update: bool,
    pub version_pinning: bool,
    pub multiple_versions: bool,
    pub executables: bool,
    pub hierarchical_components: bool,
}

impl ProviderCapabilities {
    pub const fn all() -> Self {
        Self {
            full_scan: true,
            check_updates: true,
            update: true,
            version_pinning: true,
            multiple_versions: true,
            executables: true,
            hierarchical_components: true,
        }
    }

    pub const fn supports(self, operation: ProviderOperation) -> bool {
        match operation {
            ProviderOperation::Probe => true,
            ProviderOperation::Scan => self.full_scan,
            ProviderOperation::CheckUpdates => self.check_updates,
            ProviderOperation::PlanUpdate => self.update,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderOperation {
    Probe,
    Scan,
    CheckUpdates,
    PlanUpdate,
}

impl fmt::Display for ProviderOperation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::Probe => "probe",
            Self::Scan => "scan",
            Self::CheckUpdates => "check_updates",
            Self::PlanUpdate => "plan_update",
        };
        formatter.write_str(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Recoverability {
    Retryable,
    RequiresConfiguration,
    Permanent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Error)]
#[error("provider {provider_id} failed during {operation}: {summary}")]
pub struct ProviderError {
    pub provider_id: ProviderId,
    pub operation: ProviderOperation,
    pub recoverability: Recoverability,
    pub summary: String,
}

impl ProviderError {
    pub fn new(
        provider_id: ProviderId,
        operation: ProviderOperation,
        recoverability: Recoverability,
        summary: impl AsRef<str>,
    ) -> Self {
        Self {
            provider_id,
            operation,
            recoverability,
            summary: redact_summary(summary.as_ref()),
        }
    }
}

fn redact_summary(summary: &str) -> String {
    let regex = Regex::new(r"(?i)\b(TOKEN|KEY|SECRET|PASSWORD)\s*[:=]\s*[^\s,;]+")
        .expect("the embedded secret regex is valid");
    regex.replace_all(summary, "$1=[REDACTED]").into_owned()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderStatus {
    pub available: bool,
    pub version: Option<VersionValue>,
    pub detail: Option<String>,
}

impl ProviderStatus {
    pub const fn available() -> Self {
        Self {
            available: true,
            version: None,
            detail: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateCheck {
    pub installation_id: InstallationId,
    pub component_id: ComponentId,
    pub installed_version: Option<VersionValue>,
    pub latest_version: Option<VersionValue>,
    pub status: ComponentStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderUpdateRequest {
    pub installation_id: InstallationId,
    pub component_id: ComponentId,
    pub strategy_id: StrategyId,
    pub target_version: Option<VersionValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderUpdatePlan {
    pub provider_id: ProviderId,
    pub installation_id: InstallationId,
    pub component_id: ComponentId,
    pub strategy_id: StrategyId,
    pub program: String,
    pub args: Vec<String>,
}

#[async_trait]
pub trait ToolProvider: Send + Sync {
    fn id(&self) -> ProviderId;

    fn capabilities(&self) -> ProviderCapabilities;

    async fn probe(&self) -> Result<ProviderStatus, ProviderError>;

    async fn scan(&self) -> Result<Vec<Installation>, ProviderError>;

    async fn check_updates(
        &self,
        installations: &[Installation],
    ) -> Result<Vec<UpdateCheck>, ProviderError>;

    async fn plan_update(
        &self,
        request: ProviderUpdateRequest,
    ) -> Result<ProviderUpdatePlan, ProviderError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderRegistration {
    pub id: ProviderId,
    pub capabilities: ProviderCapabilities,
    pub enabled: bool,
}

struct RegisteredProvider {
    provider: Arc<dyn ToolProvider>,
    enabled: bool,
}

#[derive(Default)]
pub struct ProviderRegistry {
    providers: BTreeMap<ProviderId, RegisteredProvider>,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(
        &mut self,
        provider: Arc<dyn ToolProvider>,
    ) -> Result<(), ProviderRegistryError> {
        let id = provider.id();
        if self.providers.contains_key(&id) {
            return Err(ProviderRegistryError::DuplicateProvider { provider_id: id });
        }
        self.providers.insert(
            id,
            RegisteredProvider {
                provider,
                enabled: true,
            },
        );
        Ok(())
    }

    pub fn get(&self, id: &ProviderId) -> Option<Arc<dyn ToolProvider>> {
        self.providers
            .get(id)
            .map(|entry| Arc::clone(&entry.provider))
    }

    pub fn list(&self) -> Vec<ProviderRegistration> {
        self.providers
            .iter()
            .map(|(id, entry)| ProviderRegistration {
                id: id.clone(),
                capabilities: entry.provider.capabilities(),
                enabled: entry.enabled,
            })
            .collect()
    }

    pub fn set_enabled(
        &mut self,
        id: &ProviderId,
        enabled: bool,
    ) -> Result<(), ProviderRegistryError> {
        let entry =
            self.providers
                .get_mut(id)
                .ok_or_else(|| ProviderRegistryError::UnknownProvider {
                    provider_id: id.clone(),
                })?;
        entry.enabled = enabled;
        Ok(())
    }

    pub fn is_enabled(&self, id: &ProviderId) -> Result<bool, ProviderRegistryError> {
        self.providers
            .get(id)
            .map(|entry| entry.enabled)
            .ok_or_else(|| ProviderRegistryError::UnknownProvider {
                provider_id: id.clone(),
            })
    }

    pub async fn probe(&self, id: &ProviderId) -> Result<ProviderStatus, ProviderRegistryError> {
        let provider = self.provider_for(id, ProviderOperation::Probe)?;
        provider
            .probe()
            .await
            .map_err(ProviderRegistryError::Provider)
    }

    pub async fn scan(&self, id: &ProviderId) -> Result<Vec<Installation>, ProviderRegistryError> {
        let provider = self.provider_for(id, ProviderOperation::Scan)?;
        provider
            .scan()
            .await
            .map_err(ProviderRegistryError::Provider)
    }

    pub async fn check_updates(
        &self,
        id: &ProviderId,
        installations: &[Installation],
    ) -> Result<Vec<UpdateCheck>, ProviderRegistryError> {
        let provider = self.provider_for(id, ProviderOperation::CheckUpdates)?;
        provider
            .check_updates(installations)
            .await
            .map_err(ProviderRegistryError::Provider)
    }

    pub async fn plan_update(
        &self,
        id: &ProviderId,
        request: ProviderUpdateRequest,
    ) -> Result<ProviderUpdatePlan, ProviderRegistryError> {
        let provider = self.provider_for(id, ProviderOperation::PlanUpdate)?;
        provider
            .plan_update(request)
            .await
            .map_err(ProviderRegistryError::Provider)
    }

    fn provider_for(
        &self,
        id: &ProviderId,
        operation: ProviderOperation,
    ) -> Result<Arc<dyn ToolProvider>, ProviderRegistryError> {
        let entry =
            self.providers
                .get(id)
                .ok_or_else(|| ProviderRegistryError::UnknownProvider {
                    provider_id: id.clone(),
                })?;
        if !entry.enabled {
            return Err(ProviderRegistryError::Disabled {
                provider_id: id.clone(),
            });
        }
        if !entry.provider.capabilities().supports(operation) {
            return Err(ProviderRegistryError::UnsupportedOperation {
                provider_id: id.clone(),
                operation,
            });
        }
        Ok(Arc::clone(&entry.provider))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProviderRegistryError {
    #[error("provider {provider_id} is already registered")]
    DuplicateProvider { provider_id: ProviderId },
    #[error("provider {provider_id} is not registered")]
    UnknownProvider { provider_id: ProviderId },
    #[error("provider {provider_id} is disabled")]
    Disabled { provider_id: ProviderId },
    #[error("provider {provider_id} does not support {operation}")]
    UnsupportedOperation {
        provider_id: ProviderId,
        operation: ProviderOperation,
    },
    #[error(transparent)]
    Provider(ProviderError),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FakeCallCounts {
    pub probe: usize,
    pub scan: usize,
    pub check_updates: usize,
    pub plan_update: usize,
}

#[derive(Clone)]
pub struct FakeProvider {
    id: ProviderId,
    capabilities: ProviderCapabilities,
    delay: Duration,
    probe_result: Result<ProviderStatus, ProviderError>,
    scan_result: Result<Vec<Installation>, ProviderError>,
    check_result: Result<Vec<UpdateCheck>, ProviderError>,
    plan_result: Result<ProviderUpdatePlan, ProviderError>,
    calls: Arc<Mutex<FakeCallCounts>>,
}

impl FakeProvider {
    pub fn new(id: ProviderId) -> Self {
        let component_id = ComponentId::new("core").expect("static fake ID is valid");
        let strategy_id = StrategyId::new("provider-default").expect("static fake ID is valid");
        let placeholder_installation =
            InstallationId::new(format!("{id}:placeholder")).expect("fake provider ID is valid");
        Self {
            capabilities: ProviderCapabilities::all(),
            delay: Duration::ZERO,
            probe_result: Ok(ProviderStatus::available()),
            scan_result: Ok(Vec::new()),
            check_result: Ok(Vec::new()),
            plan_result: Ok(ProviderUpdatePlan {
                provider_id: id.clone(),
                installation_id: placeholder_installation,
                component_id,
                strategy_id,
                program: "/usr/bin/true".into(),
                args: Vec::new(),
            }),
            id,
            calls: Arc::new(Mutex::new(FakeCallCounts::default())),
        }
    }

    pub fn with_capabilities(mut self, capabilities: ProviderCapabilities) -> Self {
        self.capabilities = capabilities;
        self
    }

    pub fn with_delay(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }

    pub fn with_probe_result(mut self, result: Result<ProviderStatus, ProviderError>) -> Self {
        self.probe_result = result;
        self
    }

    pub fn with_scan_result(mut self, result: Result<Vec<Installation>, ProviderError>) -> Self {
        self.scan_result = result;
        self
    }

    pub fn with_check_result(mut self, result: Result<Vec<UpdateCheck>, ProviderError>) -> Self {
        self.check_result = result;
        self
    }

    pub fn with_plan_result(mut self, result: Result<ProviderUpdatePlan, ProviderError>) -> Self {
        self.plan_result = result;
        self
    }

    pub fn calls(&self) -> FakeCallCounts {
        self.calls
            .lock()
            .expect("fake call mutex is not poisoned")
            .clone()
    }

    async fn wait(&self) {
        if !self.delay.is_zero() {
            tokio::time::sleep(self.delay).await;
        }
    }

    fn record(&self, update: impl FnOnce(&mut FakeCallCounts)) {
        update(&mut self.calls.lock().expect("fake call mutex is not poisoned"));
    }
}

#[async_trait]
impl ToolProvider for FakeProvider {
    fn id(&self) -> ProviderId {
        self.id.clone()
    }

    fn capabilities(&self) -> ProviderCapabilities {
        self.capabilities
    }

    async fn probe(&self) -> Result<ProviderStatus, ProviderError> {
        self.record(|calls| calls.probe += 1);
        self.wait().await;
        self.probe_result.clone()
    }

    async fn scan(&self) -> Result<Vec<Installation>, ProviderError> {
        self.record(|calls| calls.scan += 1);
        self.wait().await;
        self.scan_result.clone()
    }

    async fn check_updates(
        &self,
        _installations: &[Installation],
    ) -> Result<Vec<UpdateCheck>, ProviderError> {
        self.record(|calls| calls.check_updates += 1);
        self.wait().await;
        self.check_result.clone()
    }

    async fn plan_update(
        &self,
        _request: ProviderUpdateRequest,
    ) -> Result<ProviderUpdatePlan, ProviderError> {
        self.record(|calls| calls.plan_update += 1);
        self.wait().await;
        self.plan_result.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{InstallationScope, PackageCoordinate, PackageKind};
    use std::time::Instant;

    fn id(value: &str) -> ProviderId {
        ProviderId::new(value).unwrap()
    }

    fn installation(provider: &str, package: &str) -> Installation {
        Installation::new(
            id(provider),
            PackageCoordinate::new(PackageKind::Other("test".into()), package).unwrap(),
            InstallationScope::Global,
        )
        .unwrap()
    }

    #[tokio::test]
    async fn duplicate_registration_does_not_replace_original() {
        let provider_id = id("first");
        let original = FakeProvider::new(provider_id.clone())
            .with_scan_result(Ok(vec![installation("first", "original")]));
        let replacement = FakeProvider::new(provider_id.clone())
            .with_scan_result(Ok(vec![installation("first", "replacement")]));
        let mut registry = ProviderRegistry::new();
        registry.register(Arc::new(original)).unwrap();

        let error = registry.register(Arc::new(replacement)).unwrap_err();
        assert!(matches!(
            error,
            ProviderRegistryError::DuplicateProvider { .. }
        ));
        let scanned = registry.scan(&provider_id).await.unwrap();
        assert_eq!(scanned[0].package.name, "original");
    }

    #[tokio::test]
    async fn capabilities_prevent_unsupported_calls_without_invoking_provider() {
        let provider_id = id("read-only");
        let provider =
            FakeProvider::new(provider_id.clone()).with_capabilities(ProviderCapabilities {
                full_scan: true,
                ..ProviderCapabilities::default()
            });
        let observer = provider.clone();
        let mut registry = ProviderRegistry::new();
        registry.register(Arc::new(provider)).unwrap();

        let error = registry.check_updates(&provider_id, &[]).await.unwrap_err();
        assert!(matches!(
            error,
            ProviderRegistryError::UnsupportedOperation { .. }
        ));
        assert_eq!(observer.calls().check_updates, 0);
    }

    #[tokio::test]
    async fn one_provider_failure_does_not_mutate_other_registrations() {
        let broken_id = id("broken");
        let healthy_id = id("healthy");
        let broken =
            FakeProvider::new(broken_id.clone()).with_scan_result(Err(ProviderError::new(
                broken_id.clone(),
                ProviderOperation::Scan,
                Recoverability::Retryable,
                "TOKEN=do-not-leak registry unavailable",
            )));
        let healthy = FakeProvider::new(healthy_id.clone())
            .with_scan_result(Ok(vec![installation("healthy", "tool")]));
        let mut registry = ProviderRegistry::new();
        registry.register(Arc::new(broken)).unwrap();
        registry.register(Arc::new(healthy)).unwrap();

        let error = registry.scan(&broken_id).await.unwrap_err();
        let ProviderRegistryError::Provider(error) = error else {
            panic!("expected provider error")
        };
        assert!(!error.summary.contains("do-not-leak"));
        assert_eq!(registry.scan(&healthy_id).await.unwrap().len(), 1);
        assert_eq!(registry.list().len(), 2);
    }

    #[tokio::test]
    async fn listing_and_enablement_are_stable() {
        let mut registry = ProviderRegistry::new();
        registry
            .register(Arc::new(FakeProvider::new(id("z-provider"))))
            .unwrap();
        registry
            .register(Arc::new(FakeProvider::new(id("a-provider"))))
            .unwrap();

        assert_eq!(registry.list()[0].id.as_str(), "a-provider");
        registry.set_enabled(&id("a-provider"), false).unwrap();
        assert!(!registry.is_enabled(&id("a-provider")).unwrap());
        assert!(matches!(
            registry.scan(&id("a-provider")).await,
            Err(ProviderRegistryError::Disabled { .. })
        ));
        registry.set_enabled(&id("a-provider"), true).unwrap();
        assert!(registry.scan(&id("a-provider")).await.is_ok());
    }

    #[tokio::test]
    async fn fake_provider_supports_empty_failure_and_delay_behaviors() {
        let provider_id = id("delayed");
        let provider = FakeProvider::new(provider_id.clone())
            .with_delay(Duration::from_millis(10))
            .with_scan_result(Ok(Vec::new()));
        let mut registry = ProviderRegistry::new();
        registry.register(Arc::new(provider)).unwrap();

        let started = Instant::now();
        assert!(registry.scan(&provider_id).await.unwrap().is_empty());
        assert!(started.elapsed() >= Duration::from_millis(10));
    }
}
