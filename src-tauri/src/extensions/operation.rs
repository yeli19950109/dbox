use super::*;
use crate::{
    api::{
        ApiEvent, ExtensionChangedEventDto, RunOutputChunkDto, RunOutputEventDto,
        RunOutputStreamDto, RunStateEventDto, RunStateKindDto,
    },
    domain::{Run, RunId, RunStatus, RunSubject, RunSummary},
    persistence::{RunLogEntry, RunLogStream},
};
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Replacement {
    Absent,
    File {
        bytes: Vec<u8>,
    },
    Directory {
        source: PathBuf,
        hash: String,
    },
    Link {
        source: PathBuf,
        fallback: Option<PathBuf>,
        hash: String,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) enum Effect {
    None,
    SkillContent(Box<SkillRecord>),
    SkillDeploy {
        id: String,
        targets: Vec<String>,
        path: String,
        enabled: bool,
        mode: DeployMode,
        hash: String,
    },
    ForgetSkill(String),
    McpApplied(Vec<(String, String, Option<String>)>),
    ForgetMcp(Vec<String>),
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct FileStep {
    pub view: OperationStepView,
    pub destination: PathBuf,
    pub resolved: PathBuf,
    pub expected: String,
    pub replacement: Replacement,
    pub dependencies: Vec<usize>,
    pub effect: Effect,
}
impl FileStep {
    pub fn resolve_file(&mut self) -> ExtResult<()> {
        self.resolved = resolve_destination(&self.destination, &self.replacement)?;
        self.expected = files::fingerprint(&self.resolved)?;
        Ok(())
    }
    pub fn new(
        target: &str,
        destination: PathBuf,
        replacement: Replacement,
        action: &str,
        effect: Effect,
    ) -> ExtResult<Self> {
        let resolved = resolve_destination(&destination, &replacement)?;
        let expected = files::fingerprint(&resolved)?;
        Ok(Self {
            view: OperationStepView {
                target_id: target.into(),
                path: destination.display().to_string(),
                action: action.into(),
                before: expected.clone(),
                after: replacement_label(&replacement),
                conflict: None,
            },
            destination,
            resolved,
            expected,
            replacement,
            dependencies: vec![],
            effect,
        })
    }
}
fn replacement_label(r: &Replacement) -> String {
    match r {
        Replacement::Absent => "移除".into(),
        Replacement::File { .. } => "写入配置".into(),
        Replacement::Directory { hash, .. } => format!("copy · {hash}"),
        Replacement::Link { fallback, .. } => if fallback.is_some() {
            "symlink（不可用时回退 copy）"
        } else {
            "symlink"
        }
        .into(),
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct PreparedPlan {
    pub view: ExtensionPlan,
    pub resource_ids: Vec<String>,
    pub revision: String,
    pub agent_revision: String,
    pub before_skills: Option<SkillsDocument>,
    pub skills: Option<SkillsDocument>,
    pub before_mcp: Option<McpDocument>,
    pub mcp: Option<McpDocument>,
    pub guards: Vec<(PathBuf, String)>,
    pub steps: Vec<FileStep>,
    pub restore_of: Option<String>,
}
impl PreparedPlan {
    pub fn new(
        resource: ResourceKind,
        operation: &str,
        revision: String,
        agent_revision: String,
    ) -> Self {
        Self {
            view: ExtensionPlan {
                plan_id: uuid::Uuid::new_v4().to_string(),
                plan_hash: String::new(),
                resource,
                operation: operation.into(),
                names: vec![],
                expires_at: (Utc::now() + Duration::minutes(10)).to_rfc3339(),
                steps: vec![],
                warnings: vec![],
            },
            resource_ids: vec![],
            revision,
            agent_revision,
            before_skills: None,
            skills: None,
            before_mcp: None,
            mcp: None,
            guards: vec![],
            steps: vec![],
            restore_of: None,
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
struct BackupStep {
    step: FileStep,
    original: Replacement,
    after: Option<String>,
    status: String,
}
#[derive(Clone, Serialize, Deserialize)]
struct BackupManifest {
    #[serde(default)]
    previous_skills: Option<SkillsDocument>,
    #[serde(default)]
    previous_mcp: Option<McpDocument>,
    schema_version: u32,
    view: ExtensionBackup,
    plan: PreparedPlan,
    steps: Vec<BackupStep>,
    result: ExtensionResult,
}
impl ExtensionService {
    pub(crate) fn register_plan(&self, mut plan: PreparedPlan) -> ExtResult<ExtensionPlan> {
        // Reject aliases and parent/child overlaps; a single physical entry gets one replacement per operation.
        for (i, a) in plan.steps.iter().enumerate() {
            for b in plan.steps.iter().skip(i + 1) {
                if a.resolved == b.resolved
                    || a.resolved.starts_with(&b.resolved)
                    || b.resolved.starts_with(&a.resolved)
                {
                    return Err(err(
                        "conflict",
                        "操作目标存在路径别名或父子目录重叠，请分别处理",
                    ));
                }
            }
        }
        plan.view.steps = plan.steps.iter().map(|s| s.view.clone()).collect();
        if plan
            .steps
            .iter()
            .any(|s| !s.destination.parent().is_some_and(Path::exists))
        {
            plan.view
                .warnings
                .push("此操作会初始化预览路径中缺失的应用目录。".into());
        }
        if plan.steps.is_empty() {
            plan.view
                .warnings
                .push("仅更新 dbox 管理记录；应用文件保持原样。".into());
        }
        plan.view.plan_hash =
            files::hash(&serde_json::to_vec(&plan).map_err(|_| err("failed", "无法生成操作指纹"))?);
        let mut cache = self.cache.lock().unwrap();
        cache
            .plans
            .retain(|_, p| p.view.expires_at > Utc::now().to_rfc3339());
        if cache.plans.len() >= 64 {
            return Err(err("unavailable", "待确认计划过多，请稍后重试"));
        }
        let view = plan.view.clone();
        cache.plans.insert(view.plan_id.clone(), plan);
        Ok(view)
    }
    pub fn confirm(
        self: &Arc<Self>,
        request: ConfirmExtensionRequest,
    ) -> ExtResult<ExtensionStarted> {
        let mut cache = self.cache.lock().unwrap();
        let plan = cache
            .plans
            .get(&request.plan_id)
            .ok_or_else(|| err("invalid_plan", "计划已失效，请重新预览"))?;
        if plan.view.plan_hash != request.plan_hash
            || plan.view.expires_at < Utc::now().to_rfc3339()
        {
            return Err(err("invalid_plan", "计划指纹不匹配或已过期"));
        }
        self.check_revision(plan)?;
        let plan = cache.plans.remove(&request.plan_id).unwrap();
        drop(cache);
        let run_id = uuid::Uuid::new_v4().to_string();
        let token = self.begin_request(&run_id)?;
        let run = self.make_run(&plan, &run_id, RunStatus::Queued);
        if let Err(error) = self.persist_run(&run) {
            self.active.lock().unwrap().remove(&run_id);
            return Err(error);
        }
        let service = Arc::clone(self);
        let id = run_id.clone();
        let error_sequence = plan.steps.len() as u64 + 4;
        tokio::task::spawn_blocking(move || {
            let result = service.execute_plan(plan, &id, token);
            if let Err(error) = result {
                let mut failed = run;
                failed.status = RunStatus::Failed;
                failed.finished_at = Some(Utc::now());
                failed.summary = Some(RunSummary {
                    exit_code: None,
                    output_tail: error.message.clone(),
                    verification: None,
                    error: Some(error.message.clone()),
                });
                let _ = service.persist_run(&failed);
                service.emit_progress(&id, error_sequence, "failed", &error.message);
                let backup_dir = service.backups_dir().join(&id);
                let result = if let Ok(mut manifest) = read_manifest(&backup_dir) {
                    manifest.view.status = "failed".into();
                    manifest.result.status = "failed".into();
                    let _ = write_manifest(&backup_dir, &manifest);
                    manifest.result
                } else {
                    ExtensionResult {
                        run_id: id.clone(),
                        status: "failed".into(),
                        targets: vec![],
                        backup_id: None,
                    }
                };
                service
                    .cache
                    .lock()
                    .unwrap()
                    .results
                    .insert(id.clone(), result);
            }
            service.active.lock().unwrap().remove(&id);
        });
        Ok(ExtensionStarted { run_id })
    }
    pub(crate) fn check_revision(&self, p: &PreparedPlan) -> ExtResult<()> {
        let current = if p.view.resource == ResourceKind::Skill {
            files::revision(&self.skills_file())?
        } else {
            files::revision(&self.mcp_file())?
        };
        if current != p.revision || files::revision(&self.skills_file())? != p.agent_revision {
            return Err(err("conflict", "管理记录或应用路径已变化，请重新预览"));
        }
        for (path, fingerprint) in &p.guards {
            if files::fingerprint(path)? != *fingerprint {
                return Err(err("conflict", "来源或管理库内容已变化，请重新预览"));
            }
        }
        Ok(())
    }
    fn make_run(&self, p: &PreparedPlan, id: &str, status: RunStatus) -> Run {
        Run {
            id: RunId::new(id).unwrap(),
            tool_id: None,
            subject: match p.view.resource {
                ResourceKind::Skill => RunSubject::Skill,
                ResourceKind::Mcp => RunSubject::Mcp,
            },
            operation: Some(p.view.operation.clone()),
            resource_names: p.view.names.clone(),
            component_ids: vec![],
            status,
            created_at: Utc::now(),
            started_at: None,
            finished_at: None,
            batch_id: None,
            retry_of: None,
            log_path: Some(self.store.paths().runs_dir().join(format!("{id}.jsonl"))),
            summary: None,
        }
    }
    fn persist_run(&self, run: &Run) -> ExtResult<()> {
        self.store
            .update_cached_state(|state| {
                if let Some(old) = state.runs.iter_mut().find(|r| r.id == run.id) {
                    *old = run.clone();
                } else {
                    state.runs.push(run.clone());
                }
            })
            .map_err(|_| err("failed", "无法保存运行记录"))?;
        Ok(())
    }
    fn emit_progress(&self, id: &str, sequence: u64, status: &str, message: &str) {
        let now = Utc::now();
        let run_id = RunId::new(id).unwrap();
        let _ = self.store.append_run_log(
            &run_id,
            &RunLogEntry {
                sequence,
                timestamp: now,
                stream: RunLogStream::System,
                message: message.into(),
            },
        );
        let _ = self.emitter.emit(ApiEvent::RunOutput(RunOutputEventDto {
            run_id: id.into(),
            first_sequence: sequence.to_string(),
            last_sequence: sequence.to_string(),
            timestamp: now.to_rfc3339(),
            chunks: vec![RunOutputChunkDto {
                sequence: sequence.to_string(),
                stream: RunOutputStreamDto::System,
                message: message.into(),
            }],
        }));
        let _ = self.emitter.emit(ApiEvent::RunState(RunStateEventDto {
            run_id: id.into(),
            sequence: sequence.to_string(),
            timestamp: now.to_rfc3339(),
            state: RunStateKindDto::StateChanged {
                status: status.into(),
            },
        }));
    }
    fn execute_plan(
        &self,
        mut p: PreparedPlan,
        id: &str,
        token: CancellationToken,
    ) -> ExtResult<ExtensionResult> {
        let _guard = self.serial.lock().unwrap();
        self.check_revision(&p)?;
        let mut run = self.make_run(&p, id, RunStatus::Running);
        run.started_at = Some(Utc::now());
        self.persist_run(&run)?;
        if p.view.operation == "delete_backup" {
            if token.is_cancelled() {
                return Err(err("cancelled", "删除备份已取消"));
            }
            for step in &p.steps {
                apply_step(step)?;
            }
            run.status = RunStatus::Succeeded;
            run.finished_at = Some(Utc::now());
            self.persist_run(&run)?;
            self.emit_progress(id, 1, "succeeded", "已删除选中的备份");
            let result = ExtensionResult {
                run_id: id.into(),
                status: "succeeded".into(),
                targets: vec![],
                backup_id: None,
            };
            self.cache
                .lock()
                .unwrap()
                .results
                .insert(id.into(), result.clone());
            return Ok(result);
        }
        let backup_dir = self.backups_dir().join(id);
        let existing = self.list_backups()?;
        let total: u64 = existing
            .iter()
            .filter_map(|b| b.size_bytes.parse::<u64>().ok())
            .sum();
        if existing.len() >= 100 || total > 1024 * 1024 * 1024 {
            return Err(err(
                "unavailable",
                "备份达到 100 份或 1 GiB 上限，请先删除已完成操作的旧备份",
            ));
        }
        files::private_dir(&backup_dir)?;
        let mut manifest = BackupManifest {
            previous_skills: p.before_skills.clone(),
            previous_mcp: p.before_mcp.clone(),
            schema_version: 1,
            view: ExtensionBackup {
                id: id.into(),
                resource: p.view.resource.clone(),
                operation: p.view.operation.clone(),
                names: p.view.names.clone(),
                created_at: Utc::now().to_rfc3339(),
                status: "running".into(),
                size_bytes: "0".into(),
            },
            plan: p.clone(),
            steps: vec![],
            result: ExtensionResult {
                run_id: id.into(),
                status: "running".into(),
                targets: vec![],
                backup_id: Some(id.into()),
            },
        };
        // Complete backups before any write. Broken/unreadable targets are isolated as conflicts.
        for (index, step) in p.steps.iter_mut().enumerate() {
            let original = if step.view.conflict.is_some() {
                Replacement::Absent
            } else {
                match backup_original(&step.resolved, &backup_dir.join(index.to_string())) {
                    Ok(r) => r,
                    Err(e) => {
                        step.view.conflict = Some(e.message);
                        Replacement::Absent
                    }
                }
            };
            manifest.steps.push(BackupStep {
                step: step.clone(),
                original,
                after: None,
                status: "pending".into(),
            });
        }
        write_manifest(&backup_dir, &manifest)?;
        let prepared_bytes: u64 = walkdir::WalkDir::new(&backup_dir)
            .follow_links(false)
            .into_iter()
            .filter_map(Result::ok)
            .filter_map(|entry| entry.metadata().ok())
            .filter(|m| m.is_file())
            .map(|m| m.len())
            .sum();
        if total.saturating_add(prepared_bytes) > 1024 * 1024 * 1024 {
            files::remove(&backup_dir)?;
            return Err(err(
                "unavailable",
                "本次备份将超过 1 GiB 上限，请先清理已完成的旧备份",
            ));
        }
        self.emit_progress(id, 1, "running", "已准备备份，开始执行原生文件操作");
        // Record desired state before touching an application; unsuccessful bindings retain their last applied hashes.
        let defer_records = p.restore_of.is_some() || token.is_cancelled();
        let mut revision = if defer_records {
            p.revision.clone()
        } else {
            self.save_plan_document(&p, &p.revision)?
        };
        let mut successful = vec![false; p.steps.len()];
        for i in 0..p.steps.len() {
            let step = p.steps[i].clone();
            let outcome = if token.is_cancelled() {
                Err(err("cancelled", "运行已取消，后续步骤未执行"))
            } else if let Some(reason) = &step.view.conflict {
                Err(err("conflict", reason.clone()))
            } else if step.dependencies.iter().any(|d| !successful[*d]) {
                Err(err("conflict", "依赖步骤未完成，已保留原内容"))
            } else {
                apply_step(&step)
            };
            let (status, message) = match outcome {
                Ok(actual_mode) => {
                    successful[i] = true;
                    if !defer_records {
                        manifest.previous_skills = p.skills.clone();
                        manifest.previous_mcp = p.mcp.clone();
                    }
                    apply_effect(&mut p, i, actual_mode);
                    if !defer_records {
                        // Persist both sides before replacing a management JSON file. A
                        // restart can then recognize either side of that atomic write.
                        manifest.plan.skills = p.skills.clone();
                        manifest.plan.mcp = p.mcp.clone();
                        manifest.steps[i].after = files::fingerprint(&step.resolved).ok();
                        write_manifest(&backup_dir, &manifest)?;
                    }
                    match if defer_records {
                        Ok(revision.clone())
                    } else {
                        self.save_plan_document(&p, &revision)
                    } {
                        Ok(next) => revision = next,
                        Err(e) => {
                            manifest.steps[i].after = files::fingerprint(&step.resolved).ok();
                            manifest.steps[i].status = "applied_record_conflict".into();
                            manifest.plan = p.clone();
                            write_manifest(&backup_dir, &manifest)?;
                            return Err(e);
                        }
                    }
                    (
                        if step.expected == predicted(&step.replacement) {
                            "unchanged"
                        } else {
                            "applied"
                        },
                        "已校验配置；应用可能需要重新加载".to_owned(),
                    )
                }
                Err(e) => (
                    if token.is_cancelled() {
                        "cancelled"
                    } else if e.code == ApiErrorCode::Conflict {
                        "conflict"
                    } else {
                        "failed"
                    },
                    e.message,
                ),
            };
            let step = &p.steps[i];
            manifest.steps[i].after = files::fingerprint(&step.resolved).ok();
            manifest.steps[i].status = status.into();
            manifest.result.targets.push(TargetResult {
                target_id: step.view.target_id.clone(),
                path: step.view.path.clone(),
                status: status.into(),
                message: message.clone(),
            });
            manifest.plan.skills = p.skills.clone();
            manifest.plan.mcp = p.mcp.clone();
            write_manifest(&backup_dir, &manifest)?;
            self.emit_progress(
                id,
                i as u64 + 2,
                "running",
                &format!("{}: {} — {}", step.view.target_id, status, message),
            );
        }
        let successes = successful.iter().filter(|s| **s).count();
        let status = if token.is_cancelled() {
            if successes > 0 {
                "partial"
            } else {
                "cancelled"
            }
        } else if successes == p.steps.len() {
            "succeeded"
        } else if successes > 0 {
            "partial"
        } else {
            "failed"
        };
        if defer_records {
            if status == "succeeded" {
                revision = self.save_plan_document(&p, &revision)?;
            } else {
                p.skills = p.before_skills.clone();
                p.mcp = p.before_mcp.clone();
            }
        }
        manifest.previous_skills = None;
        manifest.previous_mcp = None;
        manifest.result.status = status.into();
        manifest.view.status = status.into();
        manifest.plan.skills = p.skills.clone();
        manifest.plan.mcp = p.mcp.clone();
        write_manifest(&backup_dir, &manifest)?;
        if status == "succeeded" {
            if let Some(restore_id) = &p.restore_of {
                let previous_dir = self.backups_dir().join(restore_id);
                let mut previous = read_manifest(&previous_dir)?;
                previous.view.status = "restored".into();
                write_manifest(&previous_dir, &previous)?;
            }
        }
        run.status = match status {
            "succeeded" => RunStatus::Succeeded,
            "partial" => RunStatus::Partial,
            "cancelled" => RunStatus::Cancelled,
            _ => RunStatus::Failed,
        };
        run.finished_at = Some(Utc::now());
        run.summary = Some(RunSummary {
            exit_code: None,
            output_tail: format!("{}/{} 个文件步骤完成", successes, p.steps.len()),
            verification: None,
            error: if status == "succeeded" {
                None
            } else {
                Some("部分目标未完成；请查看结果并重新预览或从备份恢复".into())
            },
        });
        self.persist_run(&run)?;
        self.emit_progress(
            id,
            p.steps.len() as u64 + 2,
            status,
            &run.summary.as_ref().unwrap().output_tail,
        );
        let event = ExtensionChangedEventDto {
            revision,
            sequence: Utc::now().timestamp_micros().to_string(),
            run_id: id.into(),
            resource_ids: p.resource_ids.clone(),
        };
        let _ = self
            .emitter
            .emit(if p.view.resource == ResourceKind::Skill {
                ApiEvent::SkillsChanged(event)
            } else {
                ApiEvent::McpChanged(event.into())
            });
        self.cache
            .lock()
            .unwrap()
            .results
            .insert(id.into(), manifest.result.clone());
        Ok(manifest.result)
    }
    fn save_plan_document(&self, p: &PreparedPlan, revision: &str) -> ExtResult<String> {
        if let Some(doc) = &p.skills {
            files::save(&self.skills_file(), revision, doc)
        } else if let Some(doc) = &p.mcp {
            files::save(&self.mcp_file(), revision, doc)
        } else {
            Ok(revision.into())
        }
    }
    pub fn result(&self, id: &str) -> ExtResult<ExtensionResult> {
        uuid::Uuid::parse_str(id).map_err(|_| err("invalid_request", "运行 ID 无效"))?;
        if let Some(result) = self.cache.lock().unwrap().results.get(id).cloned() {
            return Ok(result);
        }
        if self.active.lock().unwrap().contains_key(id) {
            return Ok(ExtensionResult {
                run_id: id.into(),
                status: "running".into(),
                targets: vec![],
                backup_id: None,
            });
        }
        let mut result = match read_manifest(&self.backups_dir().join(id)) {
            Ok(manifest) => manifest.result,
            Err(error) => {
                let state = self
                    .store
                    .load_state()
                    .map_err(|_| err("failed", "无法读取历史"))?;
                let run = state
                    .value
                    .runs
                    .iter()
                    .find(|r| r.id.as_str() == id && r.subject != RunSubject::ToolUpdate)
                    .ok_or(error)?;
                return Ok(ExtensionResult {
                    run_id: id.into(),
                    status: crate::api::run_status_name(run.status).into(),
                    targets: vec![],
                    backup_id: None,
                });
            }
        };
        if result.status == "running" {
            result.status = "interrupted".into();
        }
        Ok(result)
    }
    pub fn list_backups(&self) -> ExtResult<Vec<ExtensionBackup>> {
        let root = self.backups_dir();
        if !root.exists() {
            return Ok(vec![]);
        }
        let mut list = vec![];
        for entry in fs::read_dir(root).map_err(|_| err("unavailable", "备份目录无法读取"))?
        {
            let entry = entry.map_err(|_| err("unavailable", "备份目录无法读取"))?;
            if !entry.path().join("manifest.json").exists() {
                continue;
            }
            let mut manifest = read_manifest(&entry.path())?;
            if manifest.view.status == "running"
                && !self.active.lock().unwrap().contains_key(&manifest.view.id)
            {
                manifest.view.status = "interrupted".into();
            }
            let size: u64 = walkdir::WalkDir::new(entry.path())
                .follow_links(false)
                .into_iter()
                .filter_map(Result::ok)
                .filter_map(|e| e.metadata().ok())
                .filter(|m| m.is_file())
                .map(|m| m.len())
                .sum();
            manifest.view.size_bytes = size.to_string();
            list.push(manifest.view);
        }
        list.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(list)
    }
    pub(crate) fn restore_plan(
        &self,
        id: &str,
        resource: ResourceKind,
        delete: bool,
    ) -> ExtResult<ExtensionPlan> {
        uuid::Uuid::parse_str(id).map_err(|_| err("invalid_request", "备份 ID 无效"))?;
        let dir = self.backups_dir().join(id);
        let backup = read_manifest(&dir)?;
        if backup.view.resource != resource || self.active.lock().unwrap().contains_key(id) {
            return Err(err("conflict", "备份类型不匹配或操作仍在运行"));
        }
        let (agent_rev, mut skills) = self.load_skills()?;
        let (mcp_rev, mut mcp) = self.load_mcp()?;
        let rev = if resource == ResourceKind::Skill {
            agent_rev.clone()
        } else {
            mcp_rev
        };
        let mut plan = PreparedPlan::new(
            resource.clone(),
            if delete { "delete_backup" } else { "restore" },
            rev,
            agent_rev,
        );
        plan.view.names = backup.view.names.clone();
        plan.resource_ids = backup.plan.resource_ids.clone();
        if delete {
            if !["succeeded", "restored"].contains(&backup.view.status.as_str()) {
                return Err(err(
                    "conflict",
                    "未完成操作的最后恢复内容不可删除；请先恢复",
                ));
            }
            plan.steps.push(FileStep::new(
                "backup",
                dir,
                Replacement::Absent,
                "删除备份",
                Effect::None,
            )?);
        } else {
            plan.restore_of = Some(id.into());
            for b in &backup.steps {
                if b.step.view.conflict.is_some() || b.after.as_ref() == Some(&b.step.expected) {
                    continue;
                }
                let mut step = FileStep::new(
                    &b.step.view.target_id,
                    b.step.destination.clone(),
                    b.original.clone(),
                    "恢复备份",
                    Effect::None,
                )?;
                let expected = b
                    .after
                    .clone()
                    .unwrap_or_else(|| predicted(&b.step.replacement));
                if step.expected != expected && step.expected != b.step.expected {
                    step.view.conflict =
                        Some("目标在原操作之后发生变化；恢复不会覆盖外部修改".into());
                }
                plan.steps.push(step);
            }
            // Restore resource records individually, retaining later unrelated additions.
            if resource == ResourceKind::Skill {
                for sid in &plan.resource_ids {
                    let current = skills.skills.iter().find(|s| &s.id == sid);
                    let after = backup
                        .plan
                        .skills
                        .as_ref()
                        .and_then(|d| d.skills.iter().find(|s| &s.id == sid));
                    let previous = backup
                        .previous_skills
                        .as_ref()
                        .and_then(|d| d.skills.iter().find(|s| &s.id == sid));
                    let recognized_previous = backup.previous_skills.is_some()
                        && serde_json::to_value(current).ok()
                            == serde_json::to_value(previous).ok();
                    if serde_json::to_value(current).ok() != serde_json::to_value(after).ok()
                        && !recognized_previous
                    {
                        return Err(err("conflict", "资源记录已变化，不能恢复旧记录"));
                    }
                }
                plan.before_skills = Some(skills.clone());
                skills.skills.retain(|s| !plan.resource_ids.contains(&s.id));
                if let Some(before) = &backup.plan.before_skills {
                    skills.skills.extend(
                        before
                            .skills
                            .iter()
                            .filter(|s| plan.resource_ids.contains(&s.id))
                            .cloned(),
                    );
                }
                plan.skills = Some(skills);
            } else {
                for sid in &plan.resource_ids {
                    let current = mcp.servers.iter().find(|s| &s.id == sid);
                    let after = backup
                        .plan
                        .mcp
                        .as_ref()
                        .and_then(|d| d.servers.iter().find(|s| &s.id == sid));
                    let previous = backup
                        .previous_mcp
                        .as_ref()
                        .and_then(|d| d.servers.iter().find(|s| &s.id == sid));
                    let recognized_previous = backup.previous_mcp.is_some()
                        && serde_json::to_value(current).ok()
                            == serde_json::to_value(previous).ok();
                    if serde_json::to_value(current).ok() != serde_json::to_value(after).ok()
                        && !recognized_previous
                    {
                        return Err(err("conflict", "服务器记录已变化，不能恢复旧记录"));
                    }
                }
                plan.before_mcp = Some(mcp.clone());
                mcp.servers.retain(|s| !plan.resource_ids.contains(&s.id));
                if let Some(before) = &backup.plan.before_mcp {
                    mcp.servers.extend(
                        before
                            .servers
                            .iter()
                            .filter(|s| plan.resource_ids.contains(&s.id))
                            .cloned(),
                    );
                }
                plan.mcp = Some(mcp);
            }
            if plan.steps.iter().any(|s| s.view.conflict.is_some()) {
                return Err(err("conflict", "恢复目标存在外部修改；请先处理冲突"));
            }
        }
        self.register_plan(plan)
    }
    pub fn recover(&self) -> ExtResult<()> {
        self.store
            .update_cached_state(|state| {
                for run in &mut state.runs {
                    if run.subject != RunSubject::ToolUpdate && !run.status.is_terminal() {
                        run.status = RunStatus::Interrupted;
                        run.finished_at = Some(Utc::now());
                    }
                }
            })
            .map_err(|_| err("failed", "无法标记中断操作"))?;
        if self.staging().exists() {
            files::remove(&self.staging())?;
        }
        Ok(())
    }
}
fn write_manifest(dir: &Path, manifest: &BackupManifest) -> ExtResult<()> {
    files::atomic_write(
        &dir.join("manifest.json"),
        &serde_json::to_vec_pretty(manifest).map_err(|_| err("failed", "备份清单序列化失败"))?,
    )
}
fn read_manifest(dir: &Path) -> ExtResult<BackupManifest> {
    let bytes = files::read(&dir.join("manifest.json"))?;
    let manifest: BackupManifest =
        serde_json::from_slice(&bytes).map_err(|_| err("corrupt", "备份清单损坏"))?;
    if manifest.schema_version != 1 {
        return Err(err("corrupt", "备份版本不支持"));
    }
    Ok(manifest)
}
fn backup_original(path: &Path, target: &Path) -> ExtResult<Replacement> {
    if !files::exists(path) {
        return Ok(Replacement::Absent);
    }
    let meta = fs::symlink_metadata(path).map_err(|_| err("unavailable", "备份目标不可读"))?;
    if meta.file_type().is_symlink() {
        return Ok(Replacement::Link {
            source: fs::read_link(path).map_err(|_| err("unavailable", "链接不可读"))?,
            fallback: None,
            hash: String::new(),
        });
    }
    if meta.is_dir() {
        files::copy_tree(path, target)?;
        return Ok(Replacement::Directory {
            source: target.into(),
            hash: files::tree_hash(target)?,
        });
    }
    Ok(Replacement::File {
        bytes: files::read(path)?,
    })
}
fn predicted(r: &Replacement) -> String {
    match r {
        Replacement::Absent => "missing".into(),
        Replacement::File { bytes } => format!("file:{}", files::hash(bytes)),
        Replacement::Directory { hash, .. } => format!("dir:{hash}"),
        Replacement::Link { source, .. } => format!("link:{}", source.display()),
    }
}
fn resolve_destination(path: &Path, replacement: &Replacement) -> ExtResult<PathBuf> {
    if matches!(replacement, Replacement::File { .. })
        && fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink())
    {
        path.canonicalize()
            .map_err(|_| err("conflict", "MCP 配置存在断链"))
    } else {
        files::physical(path)
    }
}
fn apply_step(step: &FileStep) -> ExtResult<Option<DeployMode>> {
    if resolve_destination(&step.destination, &step.replacement)? != step.resolved
        || files::fingerprint(&step.resolved)? != step.expected
    {
        return Err(err("conflict", "目标或父目录在预览后发生变化"));
    }
    let destination = &step.resolved;
    if files::fingerprint(destination)? == predicted(&step.replacement) {
        return Ok(match &step.replacement {
            Replacement::Link { .. } => Some(DeployMode::Symlink),
            Replacement::Directory { .. } => Some(DeployMode::Copy),
            _ => None,
        });
    }
    if let Replacement::File { bytes } = &step.replacement {
        files::atomic_write(destination, bytes)?;
        return Ok(None);
    }
    let parent = destination
        .parent()
        .ok_or_else(|| err("invalid_request", "目标缺少父目录"))?;
    if !parent.exists() {
        files::private_dir(parent)?;
    }
    let prepared = parent.join(format!(".dbox-new-{}", uuid::Uuid::new_v4()));
    let mut actual = None;
    match &step.replacement {
        Replacement::Directory { source, hash } => {
            if files::tree_hash(source)? != *hash {
                return Err(err("conflict", "准备内容已变化"));
            }
            files::copy_tree(source, &prepared)?;
            actual = Some(DeployMode::Copy);
        }
        Replacement::Link {
            source,
            fallback,
            hash,
        } => {
            if files::symlink(source, &prepared).is_err() {
                if let Some(copy) = fallback {
                    if files::tree_hash(copy)? != *hash {
                        return Err(err("conflict", "准备内容已变化"));
                    }
                    files::copy_tree(copy, &prepared)?;
                    actual = Some(DeployMode::Copy);
                } else {
                    return Err(err("failed", "符号链接创建失败"));
                }
            } else {
                actual = Some(DeployMode::Symlink);
            }
        }
        _ => {}
    }
    if files::fingerprint(destination)? != step.expected {
        let _ = files::remove(&prepared);
        return Err(err("conflict", "替换前目标已变化"));
    }
    let previous = parent.join(format!(".dbox-old-{}", uuid::Uuid::new_v4()));
    let had_old = files::exists(destination);
    if had_old {
        fs::rename(destination, &previous)
            .map_err(|_| err("failed", "无法保留旧目录；目标未替换"))?;
    }
    if !matches!(step.replacement, Replacement::Absent)
        && fs::rename(&prepared, destination).is_err()
    {
        if had_old && !files::exists(destination) {
            let _ = fs::rename(&previous, destination);
        }
        let _ = files::remove(&prepared);
        return Err(err("failed", "替换失败；已尝试恢复旧目录"));
    }
    if had_old {
        files::remove(&previous)?;
    }
    let expected = if actual == Some(DeployMode::Copy) {
        match &step.replacement {
            Replacement::Directory { hash, .. } | Replacement::Link { hash, .. } => {
                format!("dir:{hash}")
            }
            _ => predicted(&step.replacement),
        }
    } else {
        predicted(&step.replacement)
    };
    if files::fingerprint(destination)? != expected {
        return Err(err("conflict", "替换后内容校验失败；可从备份恢复"));
    }
    Ok(actual)
}
fn apply_effect(p: &mut PreparedPlan, index: usize, actual: Option<DeployMode>) {
    match p.steps[index].effect.clone() {
        Effect::SkillContent(record) => {
            if let Some(s) = p
                .skills
                .as_mut()
                .and_then(|d| d.skills.iter_mut().find(|s| s.id == record.id))
            {
                s.name = record.name;
                s.description = record.description;
                s.origin = record.origin;
                s.content_hash = record.content_hash;
                s.updated_at = record.updated_at;
            }
        }
        Effect::SkillDeploy {
            id,
            targets,
            path,
            enabled,
            mode,
            hash,
        } => {
            if let Some(s) = p
                .skills
                .as_mut()
                .and_then(|d| d.skills.iter_mut().find(|s| s.id == id))
            {
                for target in targets {
                    if let Some(d) = s
                        .deployments
                        .iter_mut()
                        .find(|d| d.target_id == target && d.path == path)
                    {
                        d.mode = actual.clone().unwrap_or_else(|| mode.clone());
                        d.last_applied_hash = if enabled { Some(hash.clone()) } else { None };
                        d.observed_state = if enabled { "enabled" } else { "disabled" }.into();
                    }
                }
            }
        }
        Effect::ForgetSkill(id) => {
            if let Some(doc) = p.skills.as_mut() {
                doc.skills.retain(|s| s.id != id);
            }
        }
        Effect::McpApplied(bindings) => {
            for (id, target, hash) in bindings {
                if let Some(b) = p
                    .mcp
                    .as_mut()
                    .and_then(|d| d.servers.iter_mut().find(|s| s.id == id))
                    .and_then(|s| s.bindings.iter_mut().find(|b| b.target_id == target))
                {
                    b.last_applied_hash = hash;
                }
            }
        }
        Effect::ForgetMcp(ids) => {
            if let Some(doc) = p.mcp.as_mut() {
                doc.servers.retain(|s| !ids.contains(&s.id));
            }
        }
        Effect::None => {}
    }
    if let Some(doc) = p.mcp.as_mut() {
        doc.servers.retain(|s| {
            !(s.pending_delete && s.bindings.iter().all(|b| b.last_applied_hash.is_none()))
        });
    }
}
