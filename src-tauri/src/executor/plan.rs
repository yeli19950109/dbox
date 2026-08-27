use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::str::FromStr;

use chrono::{DateTime, Duration, Utc};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use shell_quote::Bash;
use thiserror::Error;
use uuid::Uuid;

use crate::domain::{ComponentId, InstallationId, StrategyId, ToolId};

#[derive(Clone)]
pub struct EnvironmentValue {
    value: SecretString,
    sensitive: bool,
}

impl EnvironmentValue {
    pub fn plain(value: impl Into<String>) -> Self {
        Self {
            value: SecretString::from(value.into()),
            sensitive: false,
        }
    }

    pub fn secret(value: impl Into<String>) -> Self {
        Self {
            value: SecretString::from(value.into()),
            sensitive: true,
        }
    }

    pub fn is_sensitive(&self) -> bool {
        self.sensitive
    }

    pub(crate) fn expose(&self) -> &str {
        self.value.expose_secret()
    }
}

impl fmt::Debug for EnvironmentValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EnvironmentValue")
            .field(
                "value",
                &if self.sensitive {
                    "[REDACTED]"
                } else {
                    self.expose()
                },
            )
            .field("sensitive", &self.sensitive)
            .finish()
    }
}

impl PartialEq for EnvironmentValue {
    fn eq(&self, other: &Self) -> bool {
        self.sensitive == other.sensitive && self.expose() == other.expose()
    }
}

impl Eq for EnvironmentValue {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: BTreeMap<String, EnvironmentValue>,
    pub timeout_seconds: u64,
    pub success_exit_codes: BTreeSet<i32>,
    pub network_required: bool,
    pub post_check: Option<Box<CommandSpec>>,
}

impl CommandSpec {
    pub fn new(program: impl Into<PathBuf>, args: Vec<String>) -> Self {
        Self {
            program: program.into(),
            args,
            cwd: None,
            env: BTreeMap::new(),
            timeout_seconds: 600,
            success_exit_codes: BTreeSet::from([0]),
            network_required: false,
            post_check: None,
        }
    }

    pub fn validate(&self) -> Result<(), CommandValidationError> {
        self.validate_at("command", true)
    }

    pub fn to_std_command(&self) -> Result<Command, CommandValidationError> {
        self.validate()?;
        let mut command = Command::new(&self.program);
        command.args(&self.args).stdin(Stdio::null());
        if let Some(cwd) = &self.cwd {
            command.current_dir(cwd);
        }
        for (name, value) in &self.env {
            command.env(name, value.expose());
        }
        Ok(command)
    }

    pub fn display_command(&self) -> String {
        let mut values = Vec::with_capacity(self.args.len() + 1);
        values.push(Bash::quote_vec(self.program.as_path()));
        values.extend(self.args.iter().map(Bash::quote_vec));
        values
            .into_iter()
            .map(|value| String::from_utf8_lossy(&value).into_owned())
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub fn view(&self) -> CommandSpecView {
        CommandSpecView {
            program: self.program.clone(),
            args: self.args.clone(),
            cwd: self.cwd.clone(),
            env: self
                .env
                .iter()
                .map(|(name, value)| {
                    (
                        name.clone(),
                        EnvironmentValueView {
                            value: if value.sensitive {
                                "[REDACTED]".into()
                            } else {
                                value.expose().into()
                            },
                            sensitive: value.sensitive,
                        },
                    )
                })
                .collect(),
            timeout_seconds: self.timeout_seconds,
            success_exit_codes: self.success_exit_codes.clone(),
            network_required: self.network_required,
            post_check: self
                .post_check
                .as_deref()
                .map(CommandSpec::view)
                .map(Box::new),
            display_command: self.display_command(),
        }
    }

    fn validate_at(
        &self,
        field_path: &str,
        allow_post_check: bool,
    ) -> Result<(), CommandValidationError> {
        if !self.program.is_absolute() {
            return Err(CommandValidationError::new(
                format!("{field_path}.program"),
                CommandValidationErrorKind::ProgramNotAbsolute,
                "program must be an absolute path resolved before plan creation",
            ));
        }
        if contains_nul_path(&self.program) {
            return Err(CommandValidationError::new(
                format!("{field_path}.program"),
                CommandValidationErrorKind::Nul,
                "program cannot contain NUL",
            ));
        }
        if !self.program.is_file() {
            return Err(CommandValidationError::new(
                format!("{field_path}.program"),
                CommandValidationErrorKind::ProgramUnavailable,
                "resolved program does not exist or is not a file",
            ));
        }
        if let Some(cwd) = &self.cwd {
            if !cwd.is_absolute() || !cwd.is_dir() || contains_nul_path(cwd) {
                return Err(CommandValidationError::new(
                    format!("{field_path}.cwd"),
                    CommandValidationErrorKind::InvalidCwd,
                    "cwd must be an existing absolute directory without NUL",
                ));
            }
        }
        if self.timeout_seconds == 0 {
            return Err(CommandValidationError::new(
                format!("{field_path}.timeout_seconds"),
                CommandValidationErrorKind::InvalidTimeout,
                "timeout must be greater than zero",
            ));
        }
        if self.success_exit_codes.is_empty() {
            return Err(CommandValidationError::new(
                format!("{field_path}.success_exit_codes"),
                CommandValidationErrorKind::EmptySuccessCodes,
                "at least one success exit code is required",
            ));
        }
        for (index, arg) in self.args.iter().enumerate() {
            if arg.contains('\0') {
                return Err(CommandValidationError::new(
                    format!("{field_path}.args[{index}]"),
                    CommandValidationErrorKind::Nul,
                    "arguments cannot contain NUL",
                ));
            }
        }
        for (name, value) in &self.env {
            if name.is_empty()
                || name.contains('=')
                || name.contains('\0')
                || value.expose().contains('\0')
            {
                return Err(CommandValidationError::new(
                    format!("{field_path}.env.{name}"),
                    CommandValidationErrorKind::InvalidEnvironment,
                    "environment names and values must be valid process values",
                ));
            }
        }
        if invokes_shell_string(&self.program, &self.args) {
            return Err(CommandValidationError::new(
                field_path,
                CommandValidationErrorKind::ShellNotAllowed,
                "shell command strings are not supported; use a program and argv",
            ));
        }
        if let Some(post_check) = &self.post_check {
            if !allow_post_check || post_check.post_check.is_some() {
                return Err(CommandValidationError::new(
                    format!("{field_path}.post_check"),
                    CommandValidationErrorKind::NestedPostCheck,
                    "post-check commands cannot contain another post-check",
                ));
            }
            post_check.validate_at(&format!("{field_path}.post_check"), false)?;
        }
        Ok(())
    }
}

fn invokes_shell_string(program: &Path, args: &[String]) -> bool {
    let basename = program
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    (matches!(basename.as_str(), "sh" | "bash" | "zsh") && args.iter().any(|arg| arg == "-c"))
        || (matches!(basename.as_str(), "cmd" | "cmd.exe")
            && args.iter().any(|arg| arg.eq_ignore_ascii_case("/c")))
        || (matches!(basename.as_str(), "powershell" | "powershell.exe" | "pwsh")
            && args.iter().any(|arg| arg.eq_ignore_ascii_case("-command")))
}

fn contains_nul_path(path: &Path) -> bool {
    path.to_string_lossy().contains('\0')
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentValueView {
    pub value: String,
    pub sensitive: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandSpecView {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: BTreeMap<String, EnvironmentValueView>,
    pub timeout_seconds: u64,
    pub success_exit_codes: BTreeSet<i32>,
    pub network_required: bool,
    pub post_check: Option<Box<CommandSpecView>>,
    pub display_command: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandValidationErrorKind {
    ProgramNotAbsolute,
    ProgramUnavailable,
    InvalidCwd,
    InvalidTimeout,
    EmptySuccessCodes,
    InvalidEnvironment,
    ShellNotAllowed,
    NestedPostCheck,
    Nul,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("invalid command field {field_path}: {message}")]
pub struct CommandValidationError {
    pub field_path: Box<str>,
    pub kind: CommandValidationErrorKind,
    pub message: Box<str>,
}

impl CommandValidationError {
    fn new(
        field_path: impl Into<Box<str>>,
        kind: CommandValidationErrorKind,
        message: impl Into<Box<str>>,
    ) -> Self {
        Self {
            field_path: field_path.into(),
            kind,
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PlanId(Uuid);

impl PlanId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for PlanId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for PlanId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for PlanId {
    type Err = uuid::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(value).map(Self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PlanHash(String);

impl PlanHash {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for PlanHash {
    type Err = InvalidPlanHash;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            Ok(Self(value.to_owned()))
        } else {
            Err(InvalidPlanHash)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("plan hash must be a 64-character lowercase hexadecimal hash")]
pub struct InvalidPlanHash;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdatePlan {
    pub plan_id: PlanId,
    pub plan_hash: PlanHash,
    pub tool_id: ToolId,
    pub installation_id: InstallationId,
    pub component_id: ComponentId,
    pub strategy_id: StrategyId,
    pub command: CommandSpec,
    pub config_revision: String,
    pub environment_revision: String,
    pub version_snapshot: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanContext {
    pub config_revision: String,
    pub environment_revision: String,
    pub version_snapshot: String,
    pub now: DateTime<Utc>,
    pub ttl: Duration,
}

impl UpdatePlan {
    pub fn new(
        tool_id: ToolId,
        installation_id: InstallationId,
        component_id: ComponentId,
        strategy_id: StrategyId,
        command: CommandSpec,
        context: PlanContext,
    ) -> Result<Self, PlanError> {
        command.validate()?;
        if context.ttl <= Duration::zero() {
            return Err(PlanError::InvalidTtl);
        }
        let mut plan = Self {
            plan_id: PlanId::new(),
            plan_hash: PlanHash(String::new()),
            tool_id,
            installation_id,
            component_id,
            strategy_id,
            command,
            config_revision: context.config_revision,
            environment_revision: context.environment_revision,
            version_snapshot: context.version_snapshot,
            created_at: context.now,
            expires_at: context.now + context.ttl,
        };
        plan.plan_hash = plan.recompute_hash()?;
        Ok(plan)
    }

    pub fn recompute_hash(&self) -> Result<PlanHash, PlanError> {
        let material = PlanHashMaterial::from(self);
        let encoded = serde_json::to_vec(&material).map_err(|error| PlanError::Hash {
            summary: error.to_string().into_boxed_str(),
        })?;
        Ok(PlanHash(blake3::hash(&encoded).to_hex().to_string()))
    }

    pub fn validate_confirmation(
        &self,
        submitted_id: PlanId,
        submitted_hash: &PlanHash,
        current: &ConfirmationContext,
    ) -> Result<(), PlanError> {
        self.command.validate()?;
        if self.plan_id != submitted_id {
            return Err(PlanError::IdMismatch);
        }
        let actual_hash = self.recompute_hash()?;
        if &self.plan_hash != submitted_hash || &actual_hash != submitted_hash {
            return Err(PlanError::HashMismatch);
        }
        if current.now >= self.expires_at {
            return Err(PlanError::Expired);
        }
        if current.config_revision != self.config_revision {
            return Err(PlanError::ConfigChanged);
        }
        if current.environment_revision != self.environment_revision {
            return Err(PlanError::EnvironmentChanged);
        }
        if current.version_snapshot != self.version_snapshot {
            return Err(PlanError::VersionChanged);
        }
        Ok(())
    }

    pub fn confirm(
        &self,
        submitted_id: PlanId,
        submitted_hash: &PlanHash,
        current: &ConfirmationContext,
    ) -> Result<ConfirmedPlan, PlanError> {
        self.validate_confirmation(submitted_id, submitted_hash, current)?;
        Ok(ConfirmedPlan(self.clone()))
    }

    pub fn view(&self) -> UpdatePlanView {
        UpdatePlanView {
            plan_id: self.plan_id,
            plan_hash: self.plan_hash.clone(),
            tool_id: self.tool_id.clone(),
            installation_id: self.installation_id.clone(),
            component_id: self.component_id.clone(),
            strategy_id: self.strategy_id.clone(),
            command: self.command.view(),
            config_revision: self.config_revision.clone(),
            environment_revision: self.environment_revision.clone(),
            version_snapshot: self.version_snapshot.clone(),
            created_at: self.created_at,
            expires_at: self.expires_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmedPlan(UpdatePlan);

impl ConfirmedPlan {
    pub fn plan(&self) -> &UpdatePlan {
        &self.0
    }

    pub fn command(&self) -> &CommandSpec {
        &self.0.command
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmationContext {
    pub config_revision: String,
    pub environment_revision: String,
    pub version_snapshot: String,
    pub now: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdatePlanView {
    pub plan_id: PlanId,
    pub plan_hash: PlanHash,
    pub tool_id: ToolId,
    pub installation_id: InstallationId,
    pub component_id: ComponentId,
    pub strategy_id: StrategyId,
    pub command: CommandSpecView,
    pub config_revision: String,
    pub environment_revision: String,
    pub version_snapshot: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Serialize)]
struct PlanHashMaterial<'a> {
    tool_id: &'a ToolId,
    installation_id: &'a InstallationId,
    component_id: &'a ComponentId,
    strategy_id: &'a StrategyId,
    command: CommandHashMaterial<'a>,
    config_revision: &'a str,
    environment_revision: &'a str,
    version_snapshot: &'a str,
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
}

impl<'a> From<&'a UpdatePlan> for PlanHashMaterial<'a> {
    fn from(plan: &'a UpdatePlan) -> Self {
        Self {
            tool_id: &plan.tool_id,
            installation_id: &plan.installation_id,
            component_id: &plan.component_id,
            strategy_id: &plan.strategy_id,
            command: CommandHashMaterial::from(&plan.command),
            config_revision: &plan.config_revision,
            environment_revision: &plan.environment_revision,
            version_snapshot: &plan.version_snapshot,
            created_at: plan.created_at,
            expires_at: plan.expires_at,
        }
    }
}

#[derive(Serialize)]
struct CommandHashMaterial<'a> {
    program: &'a Path,
    args: &'a [String],
    cwd: Option<&'a Path>,
    env: Vec<(&'a str, &'a str, bool)>,
    timeout_seconds: u64,
    success_exit_codes: &'a BTreeSet<i32>,
    network_required: bool,
    post_check: Option<Box<CommandHashMaterial<'a>>>,
}

impl<'a> From<&'a CommandSpec> for CommandHashMaterial<'a> {
    fn from(command: &'a CommandSpec) -> Self {
        Self {
            program: &command.program,
            args: &command.args,
            cwd: command.cwd.as_deref(),
            env: command
                .env
                .iter()
                .map(|(name, value)| (name.as_str(), value.expose(), value.sensitive))
                .collect(),
            timeout_seconds: command.timeout_seconds,
            success_exit_codes: &command.success_exit_codes,
            network_required: command.network_required,
            post_check: command
                .post_check
                .as_deref()
                .map(CommandHashMaterial::from)
                .map(Box::new),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PlanError {
    #[error(transparent)]
    InvalidCommand(#[from] CommandValidationError),
    #[error("plan lifetime must be positive")]
    InvalidTtl,
    #[error("plan hash could not be generated: {summary}")]
    Hash { summary: Box<str> },
    #[error("submitted plan ID does not match")]
    IdMismatch,
    #[error("plan hash does not match the confirmed plan")]
    HashMismatch,
    #[error("plan has expired")]
    Expired,
    #[error("configuration changed after preview")]
    ConfigChanged,
    #[error("resolved command environment changed after preview")]
    EnvironmentChanged,
    #[error("version snapshot changed after preview")]
    VersionChanged,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn fixture_program() -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(r"C:\Windows\System32\cmd.exe")
        } else {
            PathBuf::from("/usr/bin/true")
        }
    }

    fn command() -> CommandSpec {
        CommandSpec::new(
            fixture_program(),
            vec![
                "space value".into(),
                "quote'value".into(),
                ";".into(),
                "$(touch never)".into(),
                "`touch never`".into(),
            ],
        )
    }

    fn context(now: DateTime<Utc>) -> PlanContext {
        PlanContext {
            config_revision: "config-1".into(),
            environment_revision: "env-1".into(),
            version_snapshot: "version-1".into(),
            now,
            ttl: Duration::minutes(5),
        }
    }

    fn plan() -> UpdatePlan {
        UpdatePlan::new(
            ToolId::new("tool").unwrap(),
            InstallationId::new("provider:tool").unwrap(),
            ComponentId::new("core").unwrap(),
            StrategyId::new("default").unwrap(),
            command(),
            context(DateTime::from_timestamp(1_700_000_000, 0).unwrap()),
        )
        .unwrap()
    }

    #[test]
    fn shell_metacharacters_remain_separate_argv() {
        let spec = command();
        let process = spec.to_std_command().unwrap();
        let args: Vec<_> = process
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args, spec.args);
        assert!(!spec.display_command().is_empty());
    }

    #[test]
    fn changing_security_relevant_fields_changes_the_hash() {
        let original = plan();
        let original_hash = original.plan_hash.clone();
        let mut mutations = Vec::new();
        let mut changed = original.clone();
        changed.command.args.push("changed".into());
        mutations.push(changed);
        let mut changed = original.clone();
        changed.command.cwd = Some(std::env::temp_dir());
        mutations.push(changed);
        let mut changed = original.clone();
        changed
            .command
            .env
            .insert("TOKEN".into(), EnvironmentValue::secret("changed"));
        mutations.push(changed);
        let mut changed = original.clone();
        changed.strategy_id = StrategyId::new("other").unwrap();
        mutations.push(changed);
        let mut changed = original.clone();
        changed.installation_id = InstallationId::new("provider:other").unwrap();
        mutations.push(changed);
        let mut changed = original.clone();
        changed.component_id = ComponentId::new("extension").unwrap();
        mutations.push(changed);
        assert!(mutations
            .iter()
            .all(|changed| changed.recompute_hash().unwrap() != original_hash));
    }

    #[test]
    fn identical_inputs_produce_stable_hashes_despite_unique_plan_ids() {
        let now = DateTime::from_timestamp(1_700_000_000, 0).unwrap();
        let first = UpdatePlan::new(
            ToolId::new("tool").unwrap(),
            InstallationId::new("provider:tool").unwrap(),
            ComponentId::new("core").unwrap(),
            StrategyId::new("default").unwrap(),
            command(),
            context(now),
        )
        .unwrap();
        let second = UpdatePlan::new(
            ToolId::new("tool").unwrap(),
            InstallationId::new("provider:tool").unwrap(),
            ComponentId::new("core").unwrap(),
            StrategyId::new("default").unwrap(),
            command(),
            context(now),
        )
        .unwrap();
        assert_ne!(first.plan_id, second.plan_id);
        assert_eq!(first.plan_hash, second.plan_hash);
    }

    #[test]
    fn expiration_and_snapshot_changes_reject_confirmation() {
        let plan = plan();
        let confirmed = ConfirmationContext {
            config_revision: plan.config_revision.clone(),
            environment_revision: plan.environment_revision.clone(),
            version_snapshot: plan.version_snapshot.clone(),
            now: plan.created_at,
        };
        assert!(plan
            .validate_confirmation(plan.plan_id, &plan.plan_hash, &confirmed)
            .is_ok());
        let mut expired = confirmed.clone();
        expired.now = plan.expires_at;
        assert_eq!(
            plan.validate_confirmation(plan.plan_id, &plan.plan_hash, &expired),
            Err(PlanError::Expired)
        );
        let mut changed = confirmed;
        changed.environment_revision = "env-2".into();
        assert_eq!(
            plan.validate_confirmation(plan.plan_id, &plan.plan_hash, &changed),
            Err(PlanError::EnvironmentChanged)
        );
    }

    #[test]
    fn relative_missing_program_invalid_cwd_and_nul_are_rejected() {
        let relative = CommandSpec::new("tool", Vec::new());
        assert_eq!(
            relative.validate().unwrap_err().kind,
            CommandValidationErrorKind::ProgramNotAbsolute
        );
        let missing = CommandSpec::new("/definitely/missing/dbox-fixture", Vec::new());
        assert_eq!(
            missing.validate().unwrap_err().kind,
            CommandValidationErrorKind::ProgramUnavailable
        );
        let temporary = TempDir::new().unwrap();
        let invalid_cwd = temporary.path().join("file");
        std::fs::write(&invalid_cwd, b"not a directory").unwrap();
        let mut invalid = command();
        invalid.cwd = Some(invalid_cwd);
        assert_eq!(
            invalid.validate().unwrap_err().kind,
            CommandValidationErrorKind::InvalidCwd
        );
        let mut nul = command();
        nul.args.push("bad\0arg".into());
        assert_eq!(
            nul.validate().unwrap_err().kind,
            CommandValidationErrorKind::Nul
        );
    }

    #[test]
    fn secret_values_are_redacted_from_views_but_affect_hash_and_execution() {
        let mut first = plan();
        first
            .command
            .env
            .insert("API_TOKEN".into(), EnvironmentValue::secret("alpha"));
        first.plan_hash = first.recompute_hash().unwrap();
        let view = first.view();
        assert_eq!(view.command.env["API_TOKEN"].value, "[REDACTED]");
        assert_eq!(
            first.command.to_std_command().unwrap().get_envs().next(),
            Some((
                std::ffi::OsStr::new("API_TOKEN"),
                Some(std::ffi::OsStr::new("alpha"))
            ))
        );
        let mut second = first.clone();
        second
            .command
            .env
            .insert("API_TOKEN".into(), EnvironmentValue::secret("beta"));
        assert_ne!(first.plan_hash, second.recompute_hash().unwrap());
    }

    #[test]
    fn shell_quote_public_vectors_cover_display_edge_cases() {
        assert_eq!(Bash::quote_vec(""), b"''");
        assert_eq!(Bash::quote_vec("foo bar"), b"$'foo bar'");
        assert_eq!(Bash::quote_vec("雪"), "$'雪'".as_bytes());
        assert_eq!(Bash::quote_vec("a'b"), b"$'a\\'b'");
        assert_eq!(Bash::quote_vec("line\nnext"), b"$'line\\nnext'");
    }
}
