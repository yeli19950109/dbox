use std::collections::BTreeSet;
use std::fmt::{self, Display};
use std::path::PathBuf;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;

use crate::version::{
    aggregate_component_statuses, ComponentStatus, ToolStatus, VerificationStatus, VersionValue,
};

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{kind} must be non-empty, trimmed, and contain no control characters")]
pub struct InvalidIdError {
    kind: &'static str,
}

macro_rules! id_type {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, InvalidIdError> {
                let value = value.into();
                if value.is_empty() || value.trim() != value || value.chars().any(char::is_control)
                {
                    return Err(InvalidIdError {
                        kind: stringify!($name),
                    });
                }
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }

        impl FromStr for $name {
            type Err = InvalidIdError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::new(value)
            }
        }

        impl TryFrom<String> for $name {
            type Error = InvalidIdError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.serialize_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::new(value).map_err(serde::de::Error::custom)
            }
        }
    };
}

id_type!(ProviderId);
id_type!(InstallationId);
id_type!(ToolId);
id_type!(ComponentId);
id_type!(StrategyId);
id_type!(RunId);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackageKind {
    NpmGlobal,
    HomebrewFormula,
    HomebrewCask,
    Mise,
    Rustup,
    CargoInstall,
    Direct,
    Other(String),
}

impl PackageKind {
    pub fn as_slug(&self) -> &str {
        match self {
            Self::NpmGlobal => "npm_global",
            Self::HomebrewFormula => "homebrew_formula",
            Self::HomebrewCask => "homebrew_cask",
            Self::Mise => "mise",
            Self::Rustup => "rustup",
            Self::CargoInstall => "cargo_install",
            Self::Direct => "direct",
            Self::Other(value) => value,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageCoordinate {
    pub kind: PackageKind,
    pub name: String,
}

impl PackageCoordinate {
    pub fn new(kind: PackageKind, name: impl Into<String>) -> Result<Self, InvalidIdError> {
        let name = name.into();
        if name.is_empty() || name.trim() != name || name.chars().any(char::is_control) {
            return Err(InvalidIdError {
                kind: "package coordinate",
            });
        }
        Ok(Self { kind, name })
    }
}

impl InstallationId {
    pub fn from_package(
        provider_id: &ProviderId,
        package: &PackageCoordinate,
    ) -> Result<Self, InvalidIdError> {
        let coordinate = escape_coordinate(&package.name);
        let value = match (&package.kind, provider_id.as_str()) {
            (PackageKind::NpmGlobal, _) => format!("{provider_id}:{coordinate}"),
            (PackageKind::HomebrewFormula, "homebrew") => {
                format!("homebrew-formula:{coordinate}")
            }
            (PackageKind::HomebrewCask, "homebrew") => format!("homebrew-cask:{coordinate}"),
            (kind, _) => format!("{provider_id}:{}:{coordinate}", kind.as_slug()),
        };
        Self::new(value)
    }
}

fn escape_coordinate(value: &str) -> String {
    value
        .chars()
        .flat_map(|character| match character {
            '%' => "%25".chars().collect::<Vec<_>>(),
            ':' => "%3A".chars().collect(),
            _ => vec![character],
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallationScope {
    System,
    Global,
    User,
    Project,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Executable {
    pub name: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Installation {
    pub id: InstallationId,
    pub provider_id: ProviderId,
    pub package: PackageCoordinate,
    pub scope: InstallationScope,
    pub install_path: Option<PathBuf>,
    #[serde(default)]
    pub executables: Vec<Executable>,
    pub installed_version: Option<VersionValue>,
    #[serde(default)]
    pub hidden: bool,
}

impl Installation {
    pub fn new(
        provider_id: ProviderId,
        package: PackageCoordinate,
        scope: InstallationScope,
    ) -> Result<Self, InvalidIdError> {
        let id = InstallationId::from_package(&provider_id, &package)?;
        Ok(Self {
            id,
            provider_id,
            package,
            scope,
            install_path: None,
            executables: Vec::new(),
            installed_version: None,
            hidden: false,
        })
    }

    pub fn set_executables(&mut self, executables: impl IntoIterator<Item = Executable>) {
        let unique: BTreeSet<_> = executables.into_iter().collect();
        self.executables = unique.into_iter().collect();
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StrategyKind {
    ProviderDefault,
    Command {
        program: String,
        #[serde(default)]
        args: Vec<String>,
    },
    PackageManager {
        manager: String,
        #[serde(default)]
        args: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Strategy {
    pub id: StrategyId,
    pub display_name: String,
    pub kind: StrategyKind,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

const fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Component {
    pub id: ComponentId,
    pub display_name: String,
    pub installation_id: Option<InstallationId>,
    #[serde(default)]
    pub strategies: Vec<Strategy>,
    pub default_strategy: Option<StrategyId>,
    pub installed_version: Option<VersionValue>,
    pub latest_version: Option<VersionValue>,
    pub status: ComponentStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tool {
    pub id: ToolId,
    pub display_name: String,
    #[serde(default)]
    pub installation_ids: Vec<InstallationId>,
    #[serde(default)]
    pub components: Vec<Component>,
    #[serde(default)]
    pub categories: Vec<String>,
    pub homepage: Option<String>,
    #[serde(default)]
    pub hidden: bool,
}

impl Tool {
    pub fn from_installation(installation: &Installation) -> Result<Self, InvalidIdError> {
        let tool_id = ToolId::new(installation.id.as_str())?;
        let component_id = ComponentId::new("core")?;
        let strategy_id = StrategyId::new("provider-default")?;
        Ok(Self {
            id: tool_id,
            display_name: installation.package.name.clone(),
            installation_ids: vec![installation.id.clone()],
            components: vec![Component {
                id: component_id,
                display_name: "Core".to_owned(),
                installation_id: Some(installation.id.clone()),
                strategies: vec![Strategy {
                    id: strategy_id.clone(),
                    display_name: "Provider default".to_owned(),
                    kind: StrategyKind::ProviderDefault,
                    enabled: true,
                }],
                default_strategy: Some(strategy_id),
                installed_version: installation.installed_version.clone(),
                latest_version: None,
                status: ComponentStatus::unknown("not_checked", "Updates have not been checked"),
            }],
            categories: Vec::new(),
            homepage: None,
            hidden: installation.hidden,
        })
    }

    pub fn status(&self) -> ToolStatus {
        aggregate_component_statuses(self.components.iter().map(|component| &component.status))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
    TimedOut,
    Interrupted,
    Partial,
}

impl RunStatus {
    pub fn transition_to(self, next: Self) -> Result<Self, InvalidRunStatusTransition> {
        use RunStatus::{
            Cancelled, Failed, Interrupted, Partial, Queued, Running, Succeeded, TimedOut,
        };
        if matches!(
            (self, next),
            (Queued, Running | Cancelled | Interrupted)
                | (
                    Running,
                    Succeeded | Failed | Cancelled | TimedOut | Interrupted | Partial
                )
        ) {
            Ok(next)
        } else {
            Err(InvalidRunStatusTransition {
                from: self,
                to: next,
            })
        }
    }

    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded
                | Self::Failed
                | Self::Cancelled
                | Self::TimedOut
                | Self::Interrupted
                | Self::Partial
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("invalid run status transition from {from:?} to {to:?}")]
pub struct InvalidRunStatusTransition {
    pub from: RunStatus,
    pub to: RunStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Run {
    pub id: RunId,
    pub tool_id: ToolId,
    pub component_ids: Vec<ComponentId>,
    pub status: RunStatus,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub batch_id: Option<String>,
    #[serde(default)]
    pub retry_of: Option<RunId>,
    #[serde(default)]
    pub log_path: Option<PathBuf>,
    #[serde(default)]
    pub summary: Option<RunSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunSummary {
    pub exit_code: Option<i32>,
    pub output_tail: String,
    pub verification: Option<VerificationStatus>,
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::version::StatusReason;

    fn installation(provider: &str, kind: PackageKind, package: &str) -> Installation {
        Installation::new(
            ProviderId::new(provider).unwrap(),
            PackageCoordinate::new(kind, package).unwrap(),
            InstallationScope::Global,
        )
        .unwrap()
    }

    #[test]
    fn provider_and_package_coordinate_define_installation_identity() {
        let npm = installation("npm-global", PackageKind::NpmGlobal, "jq");
        let brew = installation("homebrew", PackageKind::HomebrewFormula, "jq");

        assert_eq!(npm.id.as_str(), "npm-global:jq");
        assert_eq!(brew.id.as_str(), "homebrew-formula:jq");
        assert_ne!(npm.id, brew.id);
    }

    #[test]
    fn scoped_packages_and_paths_with_spaces_have_stable_ids() {
        let mut first = installation("npm-global", PackageKind::NpmGlobal, "@scope/tool-name");
        first.install_path = Some(PathBuf::from("/Users/Test User/npm packages/tool"));
        let mut second = first.clone();
        second.install_path = Some(PathBuf::from("/another path/tool"));

        assert_eq!(first.id.as_str(), "npm-global:@scope/tool-name");
        assert_eq!(first.id, second.id);
    }

    #[test]
    fn formula_and_cask_use_separate_namespaces() {
        let formula = installation("homebrew", PackageKind::HomebrewFormula, "docker");
        let cask = installation("homebrew", PackageKind::HomebrewCask, "docker");
        assert_ne!(formula.id, cask.id);
    }

    #[test]
    fn one_installation_keeps_multiple_executables() {
        let mut item = installation("npm-global", PackageKind::NpmGlobal, "language-suite");
        item.set_executables([
            Executable {
                name: "suite".into(),
                path: PathBuf::from("/prefix with spaces/bin/suite"),
            },
            Executable {
                name: "suite-lsp".into(),
                path: PathBuf::from("/prefix with spaces/bin/suite-lsp"),
            },
        ]);

        assert_eq!(item.executables.len(), 2);
        assert_eq!(item.id.as_str(), "npm-global:language-suite");
    }

    #[test]
    fn tool_aggregation_preserves_unknown_and_failed_components() {
        let item = installation("npm-global", PackageKind::NpmGlobal, "suite");
        let mut tool = Tool::from_installation(&item).unwrap();
        tool.components.push(Component {
            id: ComponentId::new("extensions").unwrap(),
            display_name: "Extensions".into(),
            installation_id: Some(item.id),
            strategies: Vec::new(),
            default_strategy: None,
            installed_version: None,
            latest_version: None,
            status: ComponentStatus::CheckFailed {
                reason: StatusReason::new("network", "registry unavailable", true),
            },
        });

        assert!(matches!(tool.status(), ToolStatus::Partial { .. }));
    }

    #[test]
    fn serde_round_trip_preserves_fields_and_rejects_empty_ids() {
        let mut item = installation("npm-global", PackageKind::NpmGlobal, "@scope/tool");
        item.install_path = Some(PathBuf::from("/path with spaces/tool"));
        item.set_executables([Executable {
            name: "tool".into(),
            path: PathBuf::from("/path with spaces/bin/tool"),
        }]);

        let encoded = serde_json::to_string(&item).unwrap();
        let decoded: Installation = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, item);

        let empty: Result<ProviderId, _> = serde_json::from_str("\"\"");
        assert!(empty.is_err());
    }

    #[test]
    fn run_status_machine_rejects_terminal_and_skipped_transitions() {
        assert_eq!(
            RunStatus::Queued.transition_to(RunStatus::Running),
            Ok(RunStatus::Running)
        );
        assert!(RunStatus::Queued
            .transition_to(RunStatus::Succeeded)
            .is_err());
        assert!(RunStatus::Succeeded
            .transition_to(RunStatus::Running)
            .is_err());
    }
}
