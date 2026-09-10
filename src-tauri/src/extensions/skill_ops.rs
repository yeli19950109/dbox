use super::operation::{Effect, FileStep, PreparedPlan, Replacement};
use super::*;
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};
impl ExtensionService {
    pub fn preview_skill(&self, request: SkillOperationRequest) -> ExtResult<ExtensionPlan> {
        let _guard = self.serial.lock().unwrap();
        if matches!(
            request.action,
            SkillAction::Restore | SkillAction::DeleteBackup
        ) {
            return self.restore_plan(
                request
                    .backup_id
                    .as_deref()
                    .ok_or_else(|| err("invalid_request", "缺少备份 ID"))?,
                ResourceKind::Skill,
                request.action == SkillAction::DeleteBackup,
            );
        }
        let (revision, mut doc) = self.load_skills()?;
        let mut plan = PreparedPlan::new(
            ResourceKind::Skill,
            match request.action {
                SkillAction::Install => "install",
                SkillAction::Import => "import",
                SkillAction::Adopt => "adopt",
                SkillAction::Toggle => "toggle",
                SkillAction::Update => "update",
                SkillAction::Uninstall => "uninstall",
                _ => unreachable!(),
            },
            revision.clone(),
            revision,
        );
        plan.before_skills = Some(doc.clone());
        let targets = self.targets()?;
        let target_ids = expand_targets(&targets, &request.target_ids)?;
        if target_ids.len() != request.target_ids.len() {
            plan.view
                .warnings
                .push("Codex/Gemini 或自定义路径共享 Skill 目录，关联应用会一起生效。".into());
        }
        let candidates: Vec<_> = {
            let cache = self.cache.lock().unwrap();
            request
                .candidate_ids
                .iter()
                .map(|id| {
                    cache
                        .skills
                        .get(id)
                        .cloned()
                        .ok_or_else(|| err("invalid_plan", "发现结果已失效，请重新扫描"))
                })
                .collect::<ExtResult<_>>()?
        };
        match request.action {
            SkillAction::Install | SkillAction::Import => {
                if candidates.is_empty() {
                    return Err(err("invalid_request", "请选择要安装或导入的 Skill"));
                }
                for c in candidates {
                    let id = uuid::Uuid::new_v4().to_string();
                    let source = self.freeze_candidate(&c, &mut plan)?;
                    let content_dir = self.content_dir(&id);
                    let mut record = SkillRecord {
                        id: id.clone(),
                        name: c.name.clone(),
                        description: c.description,
                        directory_name: c.directory_name,
                        origin: c.origin,
                        content_hash: c.content_hash,
                        updated_at: chrono::Utc::now().to_rfc3339(),
                        deployments: vec![],
                        pending_delete: false,
                    };
                    if request.action == SkillAction::Import {
                        for target in &c.target_ids {
                            record.deployments.push(SkillDeployment {
                                target_id: target.clone(),
                                path: c.path.clone(),
                                desired_enabled: true,
                                mode: DeployMode::External,
                                last_applied_hash: Some(record.content_hash.clone()),
                                observed_state: "external".into(),
                            });
                        }
                        if c.target_ids.is_empty() {
                            return Err(err("invalid_request", "请从已有安装扫描结果中导入"));
                        }
                    }
                    let index = plan.steps.len();
                    plan.steps.push(FileStep::new(
                        "library",
                        content_dir.clone(),
                        Replacement::Directory {
                            source: source.clone(),
                            hash: record.content_hash.clone(),
                        },
                        "保存 Skill 到管理库",
                        Effect::SkillContent(Box::new(record.clone())),
                    )?);
                    if request.action == SkillAction::Install {
                        self.plan_deployments(
                            &mut plan,
                            &mut record,
                            &targets,
                            &target_ids,
                            true,
                            &request.mode,
                            false,
                            Some(index),
                            &source,
                        )?;
                    }
                    plan.resource_ids.push(id);
                    plan.view.names.push(record.name.clone());
                    doc.skills.push(record);
                }
            }
            _ => {
                if request.skill_ids.is_empty() {
                    return Err(err("invalid_request", "请选择 Skill"));
                }
                for id in &request.skill_ids {
                    let record = doc
                        .skills
                        .iter_mut()
                        .find(|s| &s.id == id)
                        .ok_or_else(|| err("not_found", "Skill 不存在"))?;
                    let library = self.content_dir(id);
                    if files::tree_hash(&library).ok().as_ref() != Some(&record.content_hash)
                        && !(request.action == SkillAction::Uninstall && !files::exists(&library))
                    {
                        return Err(err("conflict", "管理库已被修改或缺失，请先恢复或重新导入"));
                    }
                    plan.guards
                        .push((library.clone(), files::fingerprint(&library)?));
                    plan.resource_ids.push(id.clone());
                    plan.view.names.push(record.name.clone());
                    match request.action {
                        SkillAction::Toggle | SkillAction::Adopt => {
                            if target_ids.is_empty() {
                                return Err(err("invalid_request", "请选择目标应用"));
                            }
                            self.plan_deployments(
                                &mut plan,
                                record,
                                &targets,
                                &target_ids,
                                request.enabled,
                                &request.mode,
                                request.action == SkillAction::Adopt,
                                None,
                                &library,
                            )?;
                        }
                        SkillAction::Update => {
                            let origin = record.origin.as_ref().ok_or_else(|| {
                                err("invalid_request", "此 Skill 没有可更新的远程来源")
                            })?;
                            let candidate = candidates
                                .iter()
                                .find(|c| {
                                    c.origin.as_ref().is_some_and(|o| {
                                        o.source.kind == SourceKind::Github
                                            && o.source.uri == origin.source.uri
                                            && o.source.requested_ref == origin.source.requested_ref
                                            && o.relative_path == origin.relative_path
                                    })
                                })
                                .ok_or_else(|| {
                                    err(
                                        "invalid_request",
                                        "更新必须使用相同仓库、ref 和相对路径的检查结果",
                                    )
                                })?;
                            if self
                                .cache
                                .lock()
                                .unwrap()
                                .update_bases
                                .get(&format!("{}/{}", record.id, candidate.id))
                                != Some(&record.content_hash)
                            {
                                return Err(err(
                                    "invalid_plan",
                                    "更新检查已过期，请重新检查当前版本",
                                ));
                            }
                            let source = self.freeze_candidate(candidate, &mut plan)?;
                            if record.deployments.iter().any(|d| {
                                d.mode != DeployMode::External
                                    && observe_skill(d, &library) == "conflict"
                            }) {
                                return Err(err("conflict", "应用副本有本地修改，更新前请先处理"));
                            }
                            let mut updated = record.clone();
                            updated.name = candidate.name.clone();
                            updated.description = candidate.description.clone();
                            updated.origin = candidate.origin.clone();
                            updated.content_hash = candidate.content_hash.clone();
                            updated.updated_at = chrono::Utc::now().to_rfc3339();
                            let index = plan.steps.len();
                            plan.steps.push(FileStep::new(
                                "library",
                                library.clone(),
                                Replacement::Directory {
                                    source: source.clone(),
                                    hash: updated.content_hash.clone(),
                                },
                                "更新 Skill 内容",
                                Effect::SkillContent(Box::new(updated.clone())),
                            )?);
                            let enabled: Vec<_> = record
                                .deployments
                                .iter()
                                .filter(|d| d.desired_enabled && d.mode != DeployMode::External)
                                .map(|d| d.target_id.clone())
                                .collect();
                            plan.view.warnings.push(format!(
                                "{}：所有指向管理库的符号链接会随内容替换一起更新。",
                                record.name
                            ));
                            // Keep the persisted library hash old until the replacement has succeeded.
                            self.plan_deployments(
                                &mut plan,
                                &mut updated,
                                &targets,
                                &enabled,
                                true,
                                &request.mode,
                                false,
                                Some(index),
                                &source,
                            )?;
                            record.deployments = updated.deployments;
                        }
                        SkillAction::Uninstall => {
                            record.pending_delete = true;
                            let managed: Vec<_> = record
                                .deployments
                                .iter()
                                .filter(|d| d.mode != DeployMode::External)
                                .map(|d| d.target_id.clone())
                                .collect();
                            let start = plan.steps.len();
                            self.plan_deployments(
                                &mut plan,
                                record,
                                &targets,
                                &managed,
                                false,
                                &request.mode,
                                false,
                                None,
                                &library,
                            )?;
                            let deps = (start..plan.steps.len()).collect();
                            let mut remove = FileStep::new(
                                "library",
                                library,
                                Replacement::Absent,
                                "卸载管理库内容（已备份）",
                                Effect::ForgetSkill(id.clone()),
                            )?;
                            remove.dependencies = deps;
                            plan.steps.push(remove);
                            if record
                                .deployments
                                .iter()
                                .any(|d| d.mode == DeployMode::External)
                            {
                                plan.view
                                    .warnings
                                    .push("未接管的外部安装会保留在原位置。".into());
                            }
                        }
                        _ => unreachable!(),
                    }
                }
            }
        }
        plan.skills = Some(doc);
        self.register_plan(plan)
    }
    fn freeze_candidate(&self, c: &SkillCandidate, plan: &mut PreparedPlan) -> ExtResult<PathBuf> {
        if let Some(origin) = &c.origin {
            if let Some(commit) = &origin.resolved_revision {
                plan.view.warnings.push(format!(
                    "固定来源：{} @ {} · {}",
                    origin.source.uri, commit, origin.relative_path
                ));
            }
        }
        if files::tree_hash(Path::new(&c.path))? != c.content_hash {
            return Err(err("conflict", "候选内容已经变化，请重新扫描"));
        }
        plan.guards.push((
            PathBuf::from(&c.path),
            files::fingerprint(Path::new(&c.path))?,
        ));
        if !c.target_ids.is_empty() {
            plan.guards.push((
                PathBuf::from(&c.canonical_path),
                format!("dir:{}", c.content_hash),
            ));
        }
        let source = self.staging().join(&plan.view.plan_id).join(&c.id);
        files::copy_tree(Path::new(&c.path), &source)?;
        if files::tree_hash(&source)? != c.content_hash {
            return Err(err("conflict", "复制候选时内容发生变化"));
        }
        Ok(source)
    }
    #[allow(clippy::too_many_arguments)]
    fn plan_deployments(
        &self,
        plan: &mut PreparedPlan,
        record: &mut SkillRecord,
        targets: &[AgentTarget],
        ids: &[String],
        enabled: bool,
        mode: &DeployMode,
        adopt: bool,
        dependency: Option<usize>,
        source: &Path,
    ) -> ExtResult<()> {
        if *mode == DeployMode::External {
            return Err(err("invalid_request", "External 只能用于只读导入"));
        }
        let library = self.content_dir(&record.id);
        let mut paths: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for id in ids {
            let target = targets
                .iter()
                .find(|t| &t.id == id)
                .ok_or_else(|| err("invalid_request", "未知应用"))?;
            let path = record
                .deployments
                .iter()
                .find(|d| &d.target_id == id)
                .map(|d| d.path.clone())
                .unwrap_or_else(|| {
                    Path::new(&target.skills_dir)
                        .join(&record.directory_name)
                        .display()
                        .to_string()
                });
            paths
                .entry(files::physical(Path::new(&path))?.display().to_string())
                .or_default()
                .push(id.clone());
            if !record.deployments.iter().any(|d| &d.target_id == id) {
                record.deployments.push(SkillDeployment {
                    target_id: id.clone(),
                    path: path.clone(),
                    desired_enabled: false,
                    mode: mode.clone(),
                    last_applied_hash: None,
                    observed_state: "disabled".into(),
                });
            }
        }
        for (physical, affected) in paths {
            let first = record
                .deployments
                .iter()
                .find(|d| affected.contains(&d.target_id))
                .unwrap()
                .clone();
            let destination = PathBuf::from(&first.path);
            let valid_root = affected.iter().all(|id| {
                targets.iter().find(|t| &t.id == id).is_some_and(|t| {
                    t.scan_dirs.iter().any(|root| {
                        files::physical(&Path::new(root).join(&record.directory_name))
                            .ok()
                            .as_ref()
                            == Some(&PathBuf::from(&physical))
                    })
                })
            });
            if !valid_root {
                return Err(err(
                    "conflict",
                    "部署不在当前应用的用户 Skill 目录内；请核对路径覆盖设置",
                ));
            }
            let observed = observe_skill(&first, &library);
            let chosen = if first.mode != DeployMode::External && first.last_applied_hash.is_some()
            {
                first.mode.clone()
            } else {
                mode.clone()
            };
            let replacement = if !enabled {
                Replacement::Absent
            } else if chosen == DeployMode::Copy {
                Replacement::Directory {
                    source: source.into(),
                    hash: record.content_hash.clone(),
                }
            } else {
                Replacement::Link {
                    source: library.clone(),
                    fallback: if chosen == DeployMode::Auto {
                        Some(source.into())
                    } else {
                        None
                    },
                    hash: record.content_hash.clone(),
                }
            };
            let effect = Effect::SkillDeploy {
                id: record.id.clone(),
                targets: affected.clone(),
                path: first.path.clone(),
                enabled,
                mode: chosen.clone(),
                hash: record.content_hash.clone(),
            };
            let mut step = FileStep::new(
                &affected.join(" + "),
                destination,
                replacement,
                if adopt {
                    "备份并接管外部部署"
                } else if enabled {
                    "启用 Skill"
                } else {
                    "停用 Skill（保留管理库）"
                },
                effect,
            )?;
            if let Some(dep) = dependency {
                step.dependencies.push(dep);
            }
            if first.mode == DeployMode::External && !adopt {
                step.view.conflict = Some("外部安装仅观察；请先显式接管".into());
            } else if first.mode == DeployMode::External
                && files::tree_hash(Path::new(&first.path)).ok().as_ref()
                    != first.last_applied_hash.as_ref()
            {
                step.view.conflict = Some("外部安装自导入后已变化，请重新导入".into());
            } else if observed == "conflict" {
                step.view.conflict = Some("部署路径已被外部修改；不会覆盖或删除".into());
            } else if files::exists(Path::new(&physical))
                && first.last_applied_hash.is_none()
                && first.mode != DeployMode::Symlink
            {
                step.view.conflict = Some("目标已有未托管内容，请先导入并接管".into());
            }
            for d in &mut record.deployments {
                if affected.contains(&d.target_id) {
                    d.desired_enabled = enabled;
                    if adopt {
                        d.mode = chosen.clone();
                    }
                }
            }
            plan.steps.push(step);
        }
        Ok(())
    }
}
fn expand_targets(targets: &[AgentTarget], ids: &[String]) -> ExtResult<Vec<String>> {
    let mut result = BTreeSet::new();
    for id in ids {
        let t = targets
            .iter()
            .find(|t| &t.id == id)
            .ok_or_else(|| err("invalid_request", "未知应用 ID"))?;
        result.insert(id.clone());
        result.extend(t.shared_with.iter().cloned());
    }
    Ok(result.into_iter().collect())
}
