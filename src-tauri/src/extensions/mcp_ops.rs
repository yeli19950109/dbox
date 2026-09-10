use super::operation::{Effect, FileStep, PreparedPlan, Replacement};
use super::*;
use serde_json::json;
use std::{collections::BTreeSet, path::PathBuf};
impl ExtensionService {
    pub fn validate_mcp(&self, edit: McpEdit) -> ExtResult<Vec<Diagnostic>> {
        let (_, doc) = self.load_mcp()?;
        let old = edit
            .id
            .as_ref()
            .and_then(|id| doc.servers.iter().find(|s| &s.id == id));
        let config = mcp::parse_edit(&edit, old)?;
        let targets = self.targets()?;
        let mut diagnostics = vec![];
        for b in &edit.bindings {
            let target = targets
                .iter()
                .find(|t| t.id == b.target_id)
                .ok_or_else(|| err("invalid_request", "未知应用 ID"))?;
            let extra = mcp::parse_extra(
                &b.extra_json,
                old.and_then(|s| s.bindings.iter().find(|v| v.target_id == b.target_id))
                    .map(|b| &b.extra),
            )?;
            if let Err(e) = mcp::encode(target, &config, &extra) {
                diagnostics.push(Diagnostic {
                    target_id: target.id.clone(),
                    message: e.message,
                });
            }
        }
        Ok(diagnostics)
    }
    pub fn preview_mcp(&self, request: McpOperationRequest) -> ExtResult<ExtensionPlan> {
        let _guard = self.serial.lock().unwrap();
        if matches!(request.action, McpAction::Restore | McpAction::DeleteBackup) {
            return self.restore_plan(
                request
                    .backup_id
                    .as_deref()
                    .ok_or_else(|| err("invalid_request", "缺少备份 ID"))?,
                ResourceKind::Mcp,
                request.action == McpAction::DeleteBackup,
            );
        }
        let (revision, mut doc) = self.load_mcp()?;
        let (agent_revision, _) = self.load_skills()?;
        let mut plan = PreparedPlan::new(
            ResourceKind::Mcp,
            match request.action {
                McpAction::Import => "import",
                McpAction::Upsert => "upsert",
                McpAction::Toggle => "toggle",
                McpAction::Delete => "delete",
                McpAction::Sync => "sync",
                _ => unreachable!(),
            },
            revision,
            agent_revision,
        );
        plan.before_mcp = Some(doc.clone());
        let targets = self.targets()?;
        if request
            .target_ids
            .iter()
            .any(|id| !targets.iter().any(|t| &t.id == id))
        {
            return Err(err("invalid_request", "未知应用 ID"));
        }
        if request.action == McpAction::Import {
            for id in &request.candidate_ids {
                let candidate = self
                    .cache
                    .lock()
                    .unwrap()
                    .mcp
                    .get(id)
                    .cloned()
                    .ok_or_else(|| err("invalid_plan", "候选已失效，请重新扫描"))?;
                let target = targets
                    .iter()
                    .find(|t| t.id == candidate.view.target_id)
                    .ok_or_else(|| err("invalid_request", "未知应用"))?;
                if mcp::read_config(target)?.0 != candidate.file_revision {
                    return Err(err("conflict", "应用配置自扫描后已变化"));
                }
                if doc
                    .servers
                    .iter()
                    .flat_map(|s| &s.bindings)
                    .any(|b| b.target_id == target.id && b.key == candidate.view.key)
                {
                    return Err(err("conflict", "此条目已被管理"));
                }
                files::safe_name(&candidate.view.key)?;
                plan.guards.push((
                    PathBuf::from(&target.mcp_file),
                    format!("file:{}", candidate.file_revision),
                ));
                let record = McpRecord {
                    id: uuid::Uuid::new_v4().to_string(),
                    name: candidate.view.key.clone(),
                    description: String::new(),
                    config: candidate.config,
                    bindings: vec![McpBinding {
                        target_id: target.id.clone(),
                        key: candidate.view.key,
                        desired_enabled: candidate.view.enabled,
                        last_applied_hash: Some(candidate.entry_hash),
                        extra: candidate.extra,
                    }],
                    pending_delete: false,
                };
                plan.resource_ids.push(record.id.clone());
                plan.view.names.push(record.name.clone());
                doc.servers.push(record);
            }
            if let Some(text) = &request.import_json {
                let input: Value = serde_json::from_str(text)
                    .map_err(|_| err("invalid_request", "导入 JSON 无效"))?;
                let list = input
                    .get("mcpServers")
                    .unwrap_or(&input)
                    .as_object()
                    .ok_or_else(|| err("invalid_request", "导入格式需要 mcpServers 对象"))?;
                for (key, value) in list {
                    files::safe_name(key)?;
                    let (config, extra) = mcp::decode("claude", value)?;
                    if extra.as_object().is_some_and(|v| !v.is_empty()) {
                        return Err(err(
                            "unsupported",
                            "JSON 导入包含应用特有字段；请从对应应用扫描导入以保留映射",
                        ));
                    }
                    let record = McpRecord {
                        id: uuid::Uuid::new_v4().to_string(),
                        name: key.clone(),
                        description: String::new(),
                        config,
                        bindings: vec![],
                        pending_delete: false,
                    };
                    plan.resource_ids.push(record.id.clone());
                    plan.view.names.push(record.name.clone());
                    doc.servers.push(record);
                }
            }
            if plan.resource_ids.is_empty() {
                return Err(err("invalid_request", "请选择导入条目或输入 JSON"));
            }
        } else {
            let mut ids = request.server_ids.clone();
            if request.action == McpAction::Upsert {
                let edit = request
                    .edit
                    .as_ref()
                    .ok_or_else(|| err("invalid_request", "缺少服务器配置"))?;
                let old = edit
                    .id
                    .as_ref()
                    .and_then(|id| doc.servers.iter().find(|s| &s.id == id));
                if edit.id.is_some() && old.is_none() {
                    return Err(err("not_found", "服务器不存在"));
                }
                let config = mcp::parse_edit(edit, old)?;
                let mut bindings = vec![];
                let mut seen = BTreeSet::new();
                for b in &edit.bindings {
                    files::safe_name(&b.key)?;
                    if !seen.insert(&b.target_id) || !targets.iter().any(|t| t.id == b.target_id) {
                        return Err(err("invalid_request", "应用绑定重复或无效"));
                    }
                    let previous =
                        old.and_then(|s| s.bindings.iter().find(|v| v.target_id == b.target_id));
                    if previous.is_some_and(|v| v.key != b.key && v.last_applied_hash.is_some()) {
                        return Err(err("conflict", "更换配置 key 前请先停用此应用绑定"));
                    }
                    let extra = mcp::parse_extra(&b.extra_json, previous.map(|b| &b.extra))?;
                    bindings.push(McpBinding {
                        target_id: b.target_id.clone(),
                        key: b.key.clone(),
                        desired_enabled: b.desired_enabled,
                        last_applied_hash: previous.and_then(|b| b.last_applied_hash.clone()),
                        extra,
                    });
                }
                // Missing bindings still need their old entries removed explicitly.
                if let Some(old) = old {
                    for b in &old.bindings {
                        if !bindings.iter().any(|n| n.target_id == b.target_id) {
                            let mut disabled = b.clone();
                            disabled.desired_enabled = false;
                            bindings.push(disabled);
                        }
                    }
                }
                let record = McpRecord {
                    id: edit
                        .id
                        .clone()
                        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                    name: edit.name.clone(),
                    description: edit.description.clone(),
                    config,
                    bindings,
                    pending_delete: false,
                };
                ids = vec![record.id.clone()];
                doc.servers.retain(|s| s.id != record.id);
                doc.servers.push(record);
            }
            if ids.is_empty() {
                return Err(err("invalid_request", "请选择服务器"));
            }
            for id in &ids {
                let s = doc
                    .servers
                    .iter_mut()
                    .find(|s| &s.id == id)
                    .ok_or_else(|| err("not_found", "服务器不存在"))?;
                if request.action == McpAction::Toggle {
                    if request.target_ids.is_empty() {
                        return Err(err("invalid_request", "请选择目标应用"));
                    }
                    for target in &request.target_ids {
                        if let Some(b) = s.bindings.iter_mut().find(|b| &b.target_id == target) {
                            b.desired_enabled = request.enabled;
                        } else if request.enabled {
                            s.bindings.push(McpBinding {
                                target_id: target.clone(),
                                key: s.name.clone(),
                                desired_enabled: true,
                                last_applied_hash: None,
                                extra: json!({}),
                            });
                        }
                    }
                }
                if request.action == McpAction::Delete {
                    s.pending_delete = true;
                    for b in &mut s.bindings {
                        b.desired_enabled = false;
                    }
                }
                plan.view.names.push(s.name.clone());
            }
            plan.resource_ids = ids;
            self.plan_mcp_files(
                &mut plan,
                &doc,
                &targets,
                &request.target_ids,
                request.action == McpAction::Toggle || request.action == McpAction::Sync,
            )?;
            // Library-only definitions can be deleted without file operations.
            doc.servers
                .retain(|s| !(s.pending_delete && s.bindings.is_empty()));
        }
        plan.view.warnings.push("管理范围：用户级配置。写入不表示服务已启动、应用已重新加载或连接成功。JSON 会保留语义并重新排版。".into());
        plan.mcp = Some(doc);
        self.register_plan(plan)
    }
    fn plan_mcp_files(
        &self,
        plan: &mut PreparedPlan,
        doc: &McpDocument,
        targets: &[AgentTarget],
        selected: &[String],
        filtered: bool,
    ) -> ExtResult<()> {
        let mut paths = BTreeSet::new();
        for target in targets {
            if filtered && !selected.is_empty() && !selected.contains(&target.id) {
                continue;
            }
            let bindings: Vec<_> = doc
                .servers
                .iter()
                .filter(|s| plan.resource_ids.contains(&s.id))
                .flat_map(|s| {
                    s.bindings
                        .iter()
                        .filter(|b| {
                            b.target_id == target.id
                                && (b.desired_enabled || b.last_applied_hash.is_some())
                        })
                        .map(move |b| (s, b))
                })
                .collect();
            if bindings.is_empty() {
                continue;
            }
            let destination = PathBuf::from(&target.mcp_file);
            if !paths.insert(files::physical(&destination)?) {
                return Err(err(
                    "conflict",
                    "不同应用映射到同一 MCP 文件，无法无损转换，请调整应用路径",
                ));
            }
            let mut step = FileStep::new(
                &target.id,
                destination,
                Replacement::Absent,
                "同步 MCP 配置",
                Effect::None,
            )?;
            let (_, original, bytes) = match mcp::read_config(target) {
                Ok(v) => v,
                Err(e) => {
                    step.view.conflict = Some(e.message);
                    plan.steps.push(step);
                    continue;
                }
            };
            let mut changes = BTreeMap::new();
            let mut before = serde_json::Map::new();
            let mut after = serde_json::Map::new();
            let mut effects = vec![];
            for (s, b) in bindings {
                if doc
                    .servers
                    .iter()
                    .filter(|other| other.id != s.id)
                    .flat_map(|s| &s.bindings)
                    .any(|other| other.target_id == b.target_id && other.key == b.key)
                {
                    step.view.conflict =
                        Some("多个服务器绑定到相同应用 key，请分别处理同名冲突".into());
                    break;
                }
                let needs_auth_mapping = s
                    .bindings
                    .iter()
                    .filter(|other| other.target_id != b.target_id)
                    .any(|other| {
                        [
                            "bearer_token_env_var",
                            "env_http_headers",
                            "oauth",
                            "auth",
                            "headersHelper",
                            "authProviderType",
                            "targetAudience",
                            "targetServiceAccount",
                        ]
                        .iter()
                        .any(|key| other.extra.get(key).is_some())
                    });
                if b.desired_enabled
                    && needs_auth_mapping
                    && b.extra.as_object().is_some_and(|o| o.is_empty())
                    && s.config
                        .fields
                        .get("headers")
                        .is_none_or(|h| h.as_object().is_none_or(|o| o.is_empty()))
                {
                    step.view.conflict = Some(
                        "来源应用包含专有认证字段；请在编辑器中明确配置此应用的认证映射".into(),
                    );
                    break;
                }
                let current = mcp::entries(&target.id, &original).and_then(|o| o.get(&b.key));
                if b.desired_enabled
                    && !mcp::effective_enabled(&target.id, &b.key, &json!({}), &original)
                {
                    step.view.conflict = Some(
                        "应用的全局 MCP allowed/excluded 规则阻止启用；请先调整应用规则".into(),
                    );
                    break;
                }
                if current.map(mcp::entry_hash) != b.last_applied_hash
                    && !(current.is_none() && !b.desired_enabled)
                {
                    step.view.conflict =
                        Some("应用条目有外部修改或未托管同名配置；不会覆盖".into());
                    break;
                }
                if let Some(current) = current {
                    before.insert(b.key.clone(), current.clone());
                }
                let value = if b.desired_enabled {
                    match mcp::encode(target, &s.config, &b.extra) {
                        Ok(v) => Some(v),
                        Err(e) => {
                            step.view.conflict = Some(e.message);
                            break;
                        }
                    }
                } else {
                    None
                };
                if let Some(v) = &value {
                    after.insert(b.key.clone(), v.clone());
                }
                effects.push((
                    s.id.clone(),
                    target.id.clone(),
                    value.as_ref().map(mcp::entry_hash),
                ));
                changes.insert(b.key.clone(), value);
            }
            step.view.before = mcp::redacted_json(&Value::Object(before));
            step.view.after = mcp::redacted_json(&Value::Object(after));
            if step.view.conflict.is_none() {
                match mcp::render(&target.id, &bytes, &changes) {
                    Ok(bytes) => {
                        step.replacement = Replacement::File { bytes };
                        step.effect = Effect::McpApplied(effects);
                        step.resolve_file()?;
                    }
                    Err(e) => step.view.conflict = Some(e.message),
                }
            }
            plan.steps.push(step);
        }
        Ok(())
    }
}
