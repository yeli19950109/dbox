use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    Skill,
    Mcp,
}
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeployMode {
    Auto,
    Copy,
    Symlink,
    External,
}
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Local,
    Zip,
    Github,
}
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SkillSource {
    pub id: String,
    pub kind: SourceKind,
    pub uri: String,
    pub requested_ref: Option<String>,
    pub enabled: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SkillOrigin {
    pub source: SkillSource,
    pub relative_path: String,
    pub resolved_revision: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SkillDeployment {
    pub target_id: String,
    pub path: String,
    pub desired_enabled: bool,
    pub mode: DeployMode,
    pub last_applied_hash: Option<String>,
    pub observed_state: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SkillRecord {
    pub id: String,
    pub name: String,
    pub description: String,
    pub directory_name: String,
    pub origin: Option<SkillOrigin>,
    pub content_hash: String,
    pub updated_at: String,
    pub deployments: Vec<SkillDeployment>,
    pub pending_delete: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SkillCandidate {
    pub id: String,
    pub name: String,
    pub description: String,
    pub directory_name: String,
    pub path: String,
    pub canonical_path: String,
    pub link_target: Option<String>,
    pub content_hash: String,
    pub content: String,
    pub target_ids: Vec<String>,
    pub origin: Option<SkillOrigin>,
}
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostic {
    pub target_id: String,
    pub message: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SkillDiscovery {
    pub candidates: Vec<SkillCandidate>,
    pub errors: Vec<Diagnostic>,
    pub checked_at: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SkillUpdate {
    pub skill_id: String,
    pub status: String,
    pub message: String,
    pub candidate_id: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum McpTransport {
    Stdio,
    Http,
    Sse,
}
// Secret-bearing config is persisted internally; API views contain only redacted JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpConfig {
    pub transport: McpTransport,
    pub fields: serde_json::Value,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpBinding {
    pub target_id: String,
    pub key: String,
    pub desired_enabled: bool,
    pub last_applied_hash: Option<String>,
    pub extra: serde_json::Value,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpRecord {
    pub id: String,
    pub name: String,
    pub description: String,
    pub config: McpConfig,
    pub bindings: Vec<McpBinding>,
    pub pending_delete: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct McpBindingView {
    pub target_id: String,
    pub key: String,
    pub desired_enabled: bool,
    pub observed_state: String,
    pub extra_json: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct McpServerView {
    pub id: String,
    pub name: String,
    pub description: String,
    pub transport: McpTransport,
    pub config_json: String,
    pub bindings: Vec<McpBindingView>,
    pub pending_delete: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct McpCandidate {
    pub id: String,
    pub target_id: String,
    pub key: String,
    pub transport: McpTransport,
    pub config_json: String,
    pub enabled: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct McpScan {
    pub candidates: Vec<McpCandidate>,
    pub errors: Vec<Diagnostic>,
}
#[derive(Debug, Clone, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "camelCase")]
pub struct AgentOverride {
    pub skills_dir: Option<String>,
    pub mcp_file: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillsDocument {
    pub schema_version: u32,
    pub sources: Vec<SkillSource>,
    pub skills: Vec<SkillRecord>,
    #[serde(default)]
    pub agent_overrides: BTreeMap<String, AgentOverride>,
}
impl Default for SkillsDocument {
    fn default() -> Self {
        Self {
            schema_version: 1,
            sources: vec![],
            skills: vec![],
            agent_overrides: BTreeMap::new(),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpDocument {
    pub schema_version: u32,
    pub servers: Vec<McpRecord>,
}
impl Default for McpDocument {
    fn default() -> Self {
        Self {
            schema_version: 1,
            servers: vec![],
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SkillsSnapshot {
    pub revision: String,
    pub sources: Vec<SkillSource>,
    pub skills: Vec<SkillRecord>,
}
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct McpSnapshot {
    pub revision: String,
    pub servers: Vec<McpServerView>,
}
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SaveSourceRequest {
    pub expected_revision: String,
    pub source: SkillSource,
}
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DeleteSourceRequest {
    pub expected_revision: String,
    pub source_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SaveAgentsRequest {
    pub expected_revision: String,
    pub overrides: BTreeMap<String, AgentOverride>,
}
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DiscoverRequest {
    pub request_id: String,
    pub source: SkillSource,
}
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SearchRequest {
    pub request_id: String,
    pub query: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SearchSkill {
    pub name: String,
    pub repository: String,
    pub skill_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CheckUpdatesRequest {
    pub request_id: String,
    pub skill_ids: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SkillOperationRequest {
    pub action: SkillAction,
    pub skill_ids: Vec<String>,
    pub candidate_ids: Vec<String>,
    pub target_ids: Vec<String>,
    pub enabled: bool,
    pub mode: DeployMode,
    pub backup_id: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkillAction {
    Install,
    Import,
    Adopt,
    Toggle,
    Update,
    Uninstall,
    Restore,
    DeleteBackup,
}
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(tag = "action", content = "value", rename_all = "snake_case")]
pub enum SecretEdit {
    Keep,
    Set(String),
    Remove,
}
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct McpEdit {
    pub id: Option<String>,
    pub name: String,
    pub description: String,
    pub transport: McpTransport,
    pub config_json: String,
    pub secrets: BTreeMap<String, SecretEdit>,
    pub bindings: Vec<McpBindingView>,
}
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum McpAction {
    Import,
    Upsert,
    Toggle,
    Delete,
    Sync,
    Restore,
    DeleteBackup,
}
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct McpOperationRequest {
    pub action: McpAction,
    pub server_ids: Vec<String>,
    pub candidate_ids: Vec<String>,
    pub target_ids: Vec<String>,
    pub enabled: bool,
    pub edit: Option<McpEdit>,
    pub import_json: Option<String>,
    pub backup_id: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct OperationStepView {
    pub target_id: String,
    pub path: String,
    pub action: String,
    pub before: String,
    pub after: String,
    pub conflict: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionPlan {
    pub plan_id: String,
    pub plan_hash: String,
    pub resource: ResourceKind,
    pub operation: String,
    pub names: Vec<String>,
    pub expires_at: String,
    pub steps: Vec<OperationStepView>,
    pub warnings: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmExtensionRequest {
    pub plan_id: String,
    pub plan_hash: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionStarted {
    pub run_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TargetResult {
    pub target_id: String,
    pub path: String,
    pub status: String,
    pub message: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionResult {
    pub run_id: String,
    pub status: String,
    pub targets: Vec<TargetResult>,
    pub backup_id: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionBackup {
    pub id: String,
    pub resource: ResourceKind,
    pub operation: String,
    pub names: Vec<String>,
    pub created_at: String,
    pub status: String,
    pub size_bytes: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct IdRequest {
    pub id: String,
}
