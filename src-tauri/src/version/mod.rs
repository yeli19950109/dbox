use std::cmp::Ordering;
use std::sync::OnceLock;

use chrono::{DateTime, Utc};
use regex::Regex;
use semver::Version;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionValue {
    raw: String,
    semantic: Option<Version>,
}

impl VersionValue {
    pub fn new(raw: impl Into<String>) -> Self {
        let raw = raw.into();
        let semantic = extract_semver(&raw);
        Self { raw, semantic }
    }

    pub fn raw(&self) -> &str {
        &self.raw
    }

    pub fn semantic(&self) -> Option<&Version> {
        self.semantic.as_ref()
    }
}

impl Serialize for VersionValue {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.raw)
    }
}

impl<'de> Deserialize<'de> for VersionValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(Self::new(String::deserialize(deserializer)?))
    }
}

fn extract_semver(raw: &str) -> Option<Version> {
    static SEMVER_CANDIDATE: OnceLock<Regex> = OnceLock::new();
    let regex = SEMVER_CANDIDATE.get_or_init(|| {
        Regex::new(
            r"(?x)
            (?P<version>
                (?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)
                (?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?
                (?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?
            )",
        )
        .expect("the embedded semantic version regex is valid")
    });
    regex
        .captures(raw)
        .and_then(|captures| Version::parse(&captures["version"]).ok())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VersionSource {
    InstalledPackage,
    Command,
    Registry,
    ProviderMetadata,
    Catalog,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComparisonResult {
    Older,
    Equal,
    Newer,
    Incomparable,
}

pub fn compare_versions(current: &VersionValue, latest: &VersionValue) -> ComparisonResult {
    match (current.semantic(), latest.semantic()) {
        (Some(current), Some(latest)) => match current
            .major
            .cmp(&latest.major)
            .then_with(|| current.minor.cmp(&latest.minor))
            .then_with(|| current.patch.cmp(&latest.patch))
            .then_with(|| current.pre.cmp(&latest.pre))
        {
            Ordering::Less => ComparisonResult::Older,
            Ordering::Equal => ComparisonResult::Equal,
            Ordering::Greater => ComparisonResult::Newer,
        },
        _ => ComparisonResult::Incomparable,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatusReason {
    pub code: String,
    pub summary: String,
    pub retryable: bool,
}

impl StatusReason {
    pub fn new(code: impl Into<String>, summary: impl Into<String>, retryable: bool) -> Self {
        Self {
            code: code.into(),
            summary: summary.into(),
            retryable,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum VerificationStatus {
    Verified,
    VerificationUnknown { reason: StatusReason },
    VerificationFailed { reason: StatusReason },
}

pub fn verify_updated_version(
    previous: Option<&VersionValue>,
    post_check: Result<Option<&VersionValue>, StatusReason>,
) -> VerificationStatus {
    let current = match post_check {
        Ok(Some(current)) => current,
        Ok(None) => {
            return VerificationStatus::VerificationUnknown {
                reason: StatusReason::new(
                    "post_check_missing_version",
                    "The post-check did not return a version",
                    true,
                ),
            };
        }
        Err(reason) => return VerificationStatus::VerificationFailed { reason },
    };
    let Some(previous) = previous else {
        return VerificationStatus::VerificationUnknown {
            reason: StatusReason::new(
                "previous_version_unknown",
                "There is no previous version to verify against",
                false,
            ),
        };
    };

    match compare_versions(current, previous) {
        ComparisonResult::Newer => VerificationStatus::Verified,
        ComparisonResult::Equal => VerificationStatus::VerificationUnknown {
            reason: StatusReason::new(
                "version_unchanged",
                "The update command succeeded but the detected version did not change",
                true,
            ),
        },
        ComparisonResult::Older => VerificationStatus::VerificationFailed {
            reason: StatusReason::new(
                "version_regressed",
                "The detected version is older after the update command",
                false,
            ),
        },
        ComparisonResult::Incomparable => VerificationStatus::VerificationUnknown {
            reason: StatusReason::new(
                "versions_incomparable",
                "The versions before and after the update cannot be compared reliably",
                false,
            ),
        },
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ComponentStatus {
    NotInstalled,
    Checking,
    UpToDate,
    UpdateAvailable,
    Unknown { reason: StatusReason },
    Unsupported { reason: StatusReason },
    CheckFailed { reason: StatusReason },
    Updating,
    UpdateSucceeded { verification: VerificationStatus },
    UpdateFailed { reason: StatusReason },
    Cancelled,
}

impl ComponentStatus {
    pub fn unknown(code: impl Into<String>, summary: impl Into<String>) -> Self {
        Self::Unknown {
            reason: StatusReason::new(code, summary, false),
        }
    }

    pub fn from_versions(installed: Option<&VersionValue>, latest: Option<&VersionValue>) -> Self {
        let Some(installed) = installed else {
            return Self::NotInstalled;
        };
        let Some(latest) = latest else {
            return Self::unknown("latest_unavailable", "No latest version is available");
        };
        match compare_versions(installed, latest) {
            ComparisonResult::Older => Self::UpdateAvailable,
            ComparisonResult::Equal | ComparisonResult::Newer => Self::UpToDate,
            ComparisonResult::Incomparable => Self::unknown(
                "incomparable_versions",
                "The installed and latest versions cannot be compared reliably",
            ),
        }
    }

    pub fn transition_to(self, next: Self) -> Result<Self, InvalidStatusTransition> {
        if is_valid_transition(&self, &next) {
            Ok(next)
        } else {
            Err(InvalidStatusTransition {
                from: status_name(&self),
                to: status_name(&next),
            })
        }
    }
}

fn is_valid_transition(from: &ComponentStatus, to: &ComponentStatus) -> bool {
    use ComponentStatus::*;
    matches!(
        (from, to),
        (
            NotInstalled
                | UpToDate
                | UpdateAvailable
                | Unknown { .. }
                | Unsupported { .. }
                | CheckFailed { .. },
            Checking
        ) | (
            Checking,
            NotInstalled
                | UpToDate
                | UpdateAvailable
                | Unknown { .. }
                | Unsupported { .. }
                | CheckFailed { .. }
        ) | (
            UpToDate | UpdateAvailable | Unknown { .. } | Unsupported { .. } | CheckFailed { .. },
            Updating
        ) | (
            Updating,
            UpdateSucceeded { .. } | UpdateFailed { .. } | Cancelled
        ) | (
            UpdateSucceeded { .. } | UpdateFailed { .. } | Cancelled,
            Checking
        )
    )
}

fn status_name(status: &ComponentStatus) -> &'static str {
    match status {
        ComponentStatus::NotInstalled => "not_installed",
        ComponentStatus::Checking => "checking",
        ComponentStatus::UpToDate => "up_to_date",
        ComponentStatus::UpdateAvailable => "update_available",
        ComponentStatus::Unknown { .. } => "unknown",
        ComponentStatus::Unsupported { .. } => "unsupported",
        ComponentStatus::CheckFailed { .. } => "check_failed",
        ComponentStatus::Updating => "updating",
        ComponentStatus::UpdateSucceeded { .. } => "update_succeeded",
        ComponentStatus::UpdateFailed { .. } => "update_failed",
        ComponentStatus::Cancelled => "cancelled",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("invalid component status transition from {from} to {to}")]
pub struct InvalidStatusTransition {
    pub from: &'static str,
    pub to: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ToolStatus {
    NotInstalled,
    Checking,
    UpToDate,
    UpdateAvailable,
    Unknown,
    Unsupported,
    Updating,
    UpdateSucceeded,
    UpdateFailed,
    Cancelled,
    Partial {
        has_updates: bool,
        failed_components: usize,
        unknown_components: usize,
        unsupported_components: usize,
    },
}

pub fn aggregate_component_statuses<'a>(
    statuses: impl IntoIterator<Item = &'a ComponentStatus>,
) -> ToolStatus {
    let statuses: Vec<_> = statuses.into_iter().collect();
    if statuses.is_empty() {
        return ToolStatus::Unknown;
    }

    let failed_components = statuses
        .iter()
        .filter(|status| matches!(status, ComponentStatus::CheckFailed { .. }))
        .count();
    let unknown_components = statuses
        .iter()
        .filter(|status| matches!(status, ComponentStatus::Unknown { .. }))
        .count();
    let unsupported_components = statuses
        .iter()
        .filter(|status| matches!(status, ComponentStatus::Unsupported { .. }))
        .count();
    let has_updates = statuses
        .iter()
        .any(|status| matches!(status, ComponentStatus::UpdateAvailable));

    if failed_components + unknown_components > 0 {
        return ToolStatus::Partial {
            has_updates,
            failed_components,
            unknown_components,
            unsupported_components,
        };
    }
    if has_updates {
        return ToolStatus::UpdateAvailable;
    }
    if statuses
        .iter()
        .any(|status| matches!(status, ComponentStatus::Updating))
    {
        return ToolStatus::Updating;
    }
    if statuses
        .iter()
        .any(|status| matches!(status, ComponentStatus::UpdateFailed { .. }))
    {
        return ToolStatus::UpdateFailed;
    }
    if statuses
        .iter()
        .any(|status| matches!(status, ComponentStatus::Checking))
    {
        return ToolStatus::Checking;
    }
    if statuses
        .iter()
        .any(|status| matches!(status, ComponentStatus::Cancelled))
    {
        return ToolStatus::Cancelled;
    }
    if statuses
        .iter()
        .any(|status| matches!(status, ComponentStatus::UpdateSucceeded { .. }))
    {
        return ToolStatus::UpdateSucceeded;
    }
    if statuses
        .iter()
        .all(|status| matches!(status, ComponentStatus::NotInstalled))
    {
        return ToolStatus::NotInstalled;
    }
    if statuses
        .iter()
        .all(|status| matches!(status, ComponentStatus::Unsupported { .. }))
    {
        return ToolStatus::Unsupported;
    }
    if statuses.iter().all(|status| {
        matches!(
            status,
            ComponentStatus::UpToDate | ComponentStatus::Unsupported { .. }
        )
    }) {
        return ToolStatus::UpToDate;
    }

    ToolStatus::Unknown
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VersionCheck {
    pub installed: Option<VersionValue>,
    pub latest: Option<VersionValue>,
    pub installed_source: VersionSource,
    pub latest_source: VersionSource,
    pub status: ComponentStatus,
    pub checked_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reason() -> StatusReason {
        StatusReason::new("network", "registry unavailable", true)
    }

    #[test]
    fn semantic_versions_are_not_compared_lexicographically() {
        assert_eq!(
            compare_versions(&VersionValue::new("1.10.0"), &VersionValue::new("1.9.0")),
            ComparisonResult::Newer
        );
    }

    #[test]
    fn preserves_raw_prefix_prerelease_build_and_non_semver_values() {
        let prefixed = VersionValue::new("release-v1.2.3-beta.2+macos (stable)");
        assert_eq!(prefixed.raw(), "release-v1.2.3-beta.2+macos (stable)");
        assert_eq!(
            prefixed.semantic().unwrap(),
            &Version::parse("1.2.3-beta.2+macos").unwrap()
        );

        let text = VersionValue::new("rolling-2026-08");
        assert!(text.semantic().is_none());
        assert_eq!(
            compare_versions(&text, &text),
            ComparisonResult::Incomparable
        );
    }

    #[test]
    fn build_metadata_does_not_report_an_update() {
        let status = ComponentStatus::from_versions(
            Some(&VersionValue::new("v1.2.3+local")),
            Some(&VersionValue::new("1.2.3+registry")),
        );
        assert_eq!(status, ComponentStatus::UpToDate);
    }

    #[test]
    fn missing_latest_is_unknown_not_up_to_date() {
        let status = ComponentStatus::from_versions(Some(&VersionValue::new("1.0.0")), None);
        assert!(matches!(status, ComponentStatus::Unknown { .. }));
    }

    #[test]
    fn update_available_wins_but_failed_checks_remain_partial() {
        let available = ComponentStatus::UpdateAvailable;
        let failed = ComponentStatus::CheckFailed { reason: reason() };
        let aggregate = aggregate_component_statuses([&available, &failed]);
        assert_eq!(
            aggregate,
            ToolStatus::Partial {
                has_updates: true,
                failed_components: 1,
                unknown_components: 0,
                unsupported_components: 0,
            }
        );
    }

    #[test]
    fn unsupported_component_does_not_hide_known_up_to_date_component() {
        let current = ComponentStatus::UpToDate;
        let unsupported = ComponentStatus::Unsupported { reason: reason() };
        assert_eq!(
            aggregate_component_statuses([&current, &unsupported]),
            ToolStatus::UpToDate
        );
    }

    #[test]
    fn verification_outcomes_remain_distinct() {
        let unknown = ComponentStatus::UpdateSucceeded {
            verification: VerificationStatus::VerificationUnknown { reason: reason() },
        };
        let failed = ComponentStatus::UpdateSucceeded {
            verification: VerificationStatus::VerificationFailed { reason: reason() },
        };
        assert_ne!(unknown, failed);
    }

    #[test]
    fn post_check_distinguishes_changed_unchanged_failed_and_incomparable_versions() {
        let previous = VersionValue::new("1.0.0");
        assert_eq!(
            verify_updated_version(
                Some(&previous),
                Ok(Some(&VersionValue::new("release-v1.1.0")))
            ),
            VerificationStatus::Verified
        );
        assert!(matches!(
            verify_updated_version(Some(&previous), Ok(Some(&VersionValue::new("1.0.0")))),
            VerificationStatus::VerificationUnknown { .. }
        ));
        assert!(matches!(
            verify_updated_version(Some(&previous), Ok(Some(&VersionValue::new("rolling")))),
            VerificationStatus::VerificationUnknown { .. }
        ));
        assert!(matches!(
            verify_updated_version(Some(&previous), Err(reason())),
            VerificationStatus::VerificationFailed { .. }
        ));
    }

    #[test]
    fn invalid_status_transition_is_rejected() {
        let result = ComponentStatus::UpToDate.transition_to(ComponentStatus::UpdateSucceeded {
            verification: VerificationStatus::Verified,
        });
        assert!(result.is_err());
    }

    #[test]
    fn version_value_serde_reparses_semver_without_losing_display_value() {
        let original = VersionValue::new("tool v2.0.0-rc.1+arm64");
        let encoded = serde_json::to_string(&original).unwrap();
        let decoded: VersionValue = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, original);
    }
}
