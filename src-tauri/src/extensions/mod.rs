pub mod files;
mod mcp_ops;
pub mod model;
mod operation;
mod skill_ops;
use crate::{
    agents::{AgentRegistry, AgentTarget},
    api::{ApiErrorCode, ApiErrorDto, ApiEventEmitter},
    mcp,
    persistence::PersistenceStore,
    skills::{self, SkillFetcher},
};
pub use model::*;
use operation::PreparedPlan;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tokio_util::sync::CancellationToken;
pub type ExtResult<T> = Result<T, ApiErrorDto>;
pub fn err(code: &str, message: impl Into<String>) -> ApiErrorDto {
    ApiErrorDto::new(
        match code {
            "invalid_request" | "unsupported" => ApiErrorCode::InvalidRequest,
            "not_found" => ApiErrorCode::NotFound,
            "conflict" => ApiErrorCode::Conflict,
            "invalid_plan" => ApiErrorCode::InvalidPlan,
            "unavailable" | "corrupt" => ApiErrorCode::Unavailable,
            _ => ApiErrorCode::Internal,
        },
        message,
        matches!(code, "conflict" | "unavailable" | "failed"),
    )
}
#[derive(Clone)]
pub(crate) struct StoredMcpCandidate {
    pub view: McpCandidate,
    pub config: McpConfig,
    pub extra: Value,
    pub entry_hash: String,
    pub file_revision: String,
}
#[derive(Default)]
pub(crate) struct Cache {
    pub update_bases: BTreeMap<String, String>,
    pub skills: BTreeMap<String, SkillCandidate>,
    pub mcp: BTreeMap<String, StoredMcpCandidate>,
    pub plans: BTreeMap<String, PreparedPlan>,
    pub results: BTreeMap<String, ExtensionResult>,
}
pub struct ExtensionService {
    pub(crate) store: Arc<PersistenceStore>,
    pub(crate) agents: AgentRegistry,
    pub(crate) cache: Mutex<Cache>,
    pub(crate) serial: Mutex<()>,
    pub(crate) emitter: Arc<dyn ApiEventEmitter>,
    pub(crate) active: Mutex<BTreeMap<String, CancellationToken>>,
    pub(crate) fetcher: SkillFetcher,
}
impl ExtensionService {
    pub fn new(
        store: Arc<PersistenceStore>,
        agents: AgentRegistry,
        emitter: Arc<dyn ApiEventEmitter>,
    ) -> Self {
        Self {
            store,
            agents,
            cache: Mutex::new(Cache::default()),
            serial: Mutex::new(()),
            emitter,
            active: Mutex::new(BTreeMap::new()),
            fetcher: SkillFetcher::default(),
        }
    }
    pub fn with_fetcher(mut self, fetcher: SkillFetcher) -> Self {
        self.fetcher = fetcher;
        self
    }
    pub(crate) fn skills_file(&self) -> PathBuf {
        self.store.paths().config_dir.join("skills.json")
    }
    pub(crate) fn mcp_file(&self) -> PathBuf {
        self.store.paths().config_dir.join("mcp.json")
    }
    pub(crate) fn content_dir(&self, id: &str) -> PathBuf {
        self.store
            .paths()
            .data_dir
            .join("skills")
            .join(id)
            .join("content")
    }
    pub(crate) fn staging(&self) -> PathBuf {
        self.store.paths().data_dir.join("extension-staging")
    }
    pub(crate) fn backups_dir(&self) -> PathBuf {
        self.store.paths().data_dir.join("extension-backups")
    }
    pub(crate) fn load_skills(&self) -> ExtResult<(String, SkillsDocument)> {
        let (revision, doc): (_, SkillsDocument) = files::load(&self.skills_file())?;
        let mut ids = std::collections::BTreeSet::new();
        for s in &doc.skills {
            uuid::Uuid::parse_str(&s.id).map_err(|_| err("corrupt", "Skill ID 无效"))?;
            files::safe_name(&s.directory_name)?;
            if !ids.insert(&s.id) {
                return Err(err("corrupt", "Skill ID 重复"));
            }
        }
        Ok((revision, doc))
    }
    pub(crate) fn load_mcp(&self) -> ExtResult<(String, McpDocument)> {
        let (revision, doc): (_, McpDocument) = files::load(&self.mcp_file())?;
        let mut ids = std::collections::BTreeSet::new();
        for s in &doc.servers {
            uuid::Uuid::parse_str(&s.id).map_err(|_| err("corrupt", "服务器 ID 无效"))?;
            if !ids.insert(&s.id) {
                return Err(err("corrupt", "服务器 ID 重复"));
            }
        }
        Ok((revision, doc))
    }
    pub fn targets(&self) -> ExtResult<Vec<AgentTarget>> {
        self.agents.targets(&self.load_skills()?.1.agent_overrides)
    }
    pub fn save_agents(&self, request: SaveAgentsRequest) -> ExtResult<Vec<AgentTarget>> {
        let _guard = self.serial.lock().unwrap();
        let (rev, mut doc) = self.load_skills()?;
        if rev != request.expected_revision {
            return Err(err("conflict", "应用路径设置已过期"));
        }
        if doc.skills.iter().any(|s| !s.deployments.is_empty())
            || self
                .load_mcp()?
                .1
                .servers
                .iter()
                .any(|s| !s.bindings.is_empty())
        {
            return Err(err(
                "conflict",
                "已有部署绑定时不可切换应用路径；请先移除绑定",
            ));
        }
        let targets = self.agents.targets(&request.overrides)?;
        doc.agent_overrides = request.overrides;
        files::save(&self.skills_file(), &rev, &doc)?;
        Ok(targets)
    }
    pub fn list_skills(&self) -> ExtResult<SkillsSnapshot> {
        let (revision, mut doc) = self.load_skills()?;
        for skill in &mut doc.skills {
            for d in &mut skill.deployments {
                d.observed_state = observe_skill(d, &self.content_dir(&skill.id));
            }
        }
        Ok(SkillsSnapshot {
            revision,
            sources: doc.sources,
            skills: doc.skills,
        })
    }
    pub fn save_source(&self, request: SaveSourceRequest) -> ExtResult<SkillsSnapshot> {
        let _guard = self.serial.lock().unwrap();
        skills::validate_source(&request.source)?;
        let (revision, mut doc) = self.load_skills()?;
        if request.expected_revision != revision {
            return Err(err("conflict", "来源列表已更改"));
        }
        let mut source = request.source;
        if source.id.is_empty() {
            source.id = uuid::Uuid::new_v4().to_string();
        }
        if let Some(old) = doc.sources.iter_mut().find(|s| s.id == source.id) {
            *old = source;
        } else {
            doc.sources.push(source);
        }
        files::save(&self.skills_file(), &revision, &doc)?;
        self.list_skills()
    }
    pub fn delete_source(&self, request: DeleteSourceRequest) -> ExtResult<SkillsSnapshot> {
        let _guard = self.serial.lock().unwrap();
        let (revision, mut doc) = self.load_skills()?;
        if request.expected_revision != revision {
            return Err(err("conflict", "来源列表已更改"));
        }
        doc.sources.retain(|s| s.id != request.source_id);
        files::save(&self.skills_file(), &revision, &doc)?;
        self.list_skills()
    }
    pub fn begin_request(&self, id: &str) -> ExtResult<CancellationToken> {
        uuid::Uuid::parse_str(id).map_err(|_| err("invalid_request", "requestId 必须为 UUID"))?;
        let mut active = self.active.lock().unwrap();
        if active.contains_key(id) {
            return Err(err("conflict", "请求已在执行"));
        }
        let token = CancellationToken::new();
        active.insert(id.into(), token.clone());
        Ok(token)
    }
    pub fn cancel(&self, id: &str) -> bool {
        if let Some(token) = self.active.lock().unwrap().get(id) {
            token.cancel();
            true
        } else {
            false
        }
    }
    pub async fn discover(&self, request: DiscoverRequest) -> ExtResult<SkillDiscovery> {
        let token = self.begin_request(&request.request_id)?;
        let result = self.discover_inner(&request.source, &token).await;
        self.active.lock().unwrap().remove(&request.request_id);
        result
    }
    async fn discover_inner(
        &self,
        source: &SkillSource,
        token: &CancellationToken,
    ) -> ExtResult<SkillDiscovery> {
        let staging = self.staging().join(uuid::Uuid::new_v4().to_string());
        let prepared = self.fetcher.prepare(source, &staging, token).await;
        let (root, revision) = match prepared {
            Ok(p) => p,
            Err(e) => {
                let _ = files::remove(&staging);
                return Ok(SkillDiscovery {
                    candidates: vec![],
                    errors: vec![Diagnostic {
                        target_id: source.id.clone(),
                        message: e.message,
                    }],
                    checked_at: chrono::Utc::now().to_rfc3339(),
                });
            }
        };
        let origin = SkillOrigin {
            source: source.clone(),
            relative_path: String::new(),
            resolved_revision: revision,
        };
        let mut discovered = skills::scan(&root, Some(origin));
        // Freeze every candidate, including local directories, so a confirmation installs precisely the previewed bytes.
        let mut cache = self.cache.lock().unwrap();
        if cache.skills.len() + discovered.candidates.len() > 256 {
            return Err(err("unavailable", "发现缓存已满；请重新启动以清理暂存内容"));
        }
        for c in &mut discovered.candidates {
            if token.is_cancelled() {
                return Err(err("cancelled", "发现已取消"));
            }
            let frozen = self.staging().join(&c.id).join(&c.directory_name);
            files::copy_tree(Path::new(&c.path), &frozen)?;
            if files::tree_hash(&frozen)? != c.content_hash {
                return Err(err("conflict", "来源在暂存过程中发生变化"));
            }
            c.path = frozen.display().to_string();
            c.canonical_path = c.path.clone();
            cache.skills.insert(c.id.clone(), c.clone());
        }
        let _ = files::remove(&staging);
        Ok(discovered)
    }
    pub async fn search(&self, request: SearchRequest) -> ExtResult<Vec<SearchSkill>> {
        let token = self.begin_request(&request.request_id)?;
        let result = self.fetcher.search(&request.query, &token).await;
        self.active.lock().unwrap().remove(&request.request_id);
        result
    }
    pub fn scan_skill_imports(&self) -> ExtResult<SkillDiscovery> {
        let mut result = SkillDiscovery {
            candidates: vec![],
            errors: vec![],
            checked_at: chrono::Utc::now().to_rfc3339(),
        };
        let (_, doc) = self.load_skills()?;
        let mut seen: BTreeMap<String, usize> = BTreeMap::new();
        for target in self.targets()? {
            for dir in &target.scan_dirs {
                let root = Path::new(dir);
                if !files::exists(root) {
                    continue;
                }
                let entries = match std::fs::read_dir(root) {
                    Ok(e) => e,
                    Err(_) => {
                        result.errors.push(Diagnostic {
                            target_id: target.id.clone(),
                            message: "Skill 目录不可读".into(),
                        });
                        continue;
                    }
                };
                for entry in entries.take(files::MAX_FILES) {
                    let path = match entry {
                        Ok(e) => e.path(),
                        Err(_) => continue,
                    };
                    if path
                        .file_name()
                        .is_some_and(|n| n.to_string_lossy().starts_with('.'))
                        || !path.join("SKILL.md").exists()
                    {
                        continue;
                    }
                    let physical = files::physical(&path)?.display().to_string();
                    if doc.skills.iter().flat_map(|s| &s.deployments).any(|d| {
                        files::physical(Path::new(&d.path))
                            .ok()
                            .is_some_and(|p| p.display().to_string() == physical)
                    }) {
                        continue;
                    }
                    if let Some(index) = seen.get(&physical) {
                        if !result.candidates[*index].target_ids.contains(&target.id) {
                            result.candidates[*index].target_ids.push(target.id.clone());
                        }
                        continue;
                    }
                    match skills::candidate(&path, None, vec![target.id.clone()]) {
                        Ok(c) => {
                            seen.insert(physical, result.candidates.len());
                            result.candidates.push(c);
                        }
                        Err(e) => result.errors.push(Diagnostic {
                            target_id: target.id.clone(),
                            message: e.message,
                        }),
                    }
                }
            }
        }
        let mut cache = self.cache.lock().unwrap();
        for c in &result.candidates {
            cache.skills.insert(c.id.clone(), c.clone());
        }
        Ok(result)
    }
    pub async fn check_updates(&self, request: CheckUpdatesRequest) -> ExtResult<Vec<SkillUpdate>> {
        let token = self.begin_request(&request.request_id)?;
        let result = self.check_updates_inner(&request.skill_ids, &token).await;
        self.active.lock().unwrap().remove(&request.request_id);
        result
    }
    async fn check_updates_inner(
        &self,
        ids: &[String],
        token: &CancellationToken,
    ) -> ExtResult<Vec<SkillUpdate>> {
        let skills = self.list_skills()?.skills;
        let mut result = vec![];
        let mut sources: BTreeMap<String, SkillDiscovery> = BTreeMap::new();
        for s in skills
            .into_iter()
            .filter(|s| ids.is_empty() || ids.contains(&s.id))
        {
            if token.is_cancelled() {
                return Err(err("cancelled", "检查已取消"));
            }
            let mut row = SkillUpdate {
                skill_id: s.id.clone(),
                status: "source_unknown".into(),
                message: "无法检查远程更新；可重新导入本地来源".into(),
                candidate_id: None,
            };
            if files::tree_hash(&self.content_dir(&s.id)).ok().as_ref() != Some(&s.content_hash) {
                row.status = "local_modified".into();
                row.message = "管理库内容已被修改".into();
            } else if s.deployments.iter().any(|d| d.observed_state == "conflict") {
                row.status = "conflict".into();
                row.message = "应用部署存在外部修改".into();
            } else if let Some(origin) = &s.origin {
                if origin.source.kind == SourceKind::Github {
                    let source_key = format!(
                        "{}@{}",
                        origin.source.uri,
                        origin.source.requested_ref.as_deref().unwrap_or("HEAD")
                    );
                    if !sources.contains_key(&source_key) {
                        sources.insert(
                            source_key.clone(),
                            self.discover_inner(&origin.source, token).await?,
                        );
                    }
                    let discovered = &sources[&source_key];
                    if !discovered.errors.is_empty() {
                        row.status = "unavailable".into();
                        row.message = "来源检查失败，不能判断是否有更新".into();
                    } else if let Some(c) = discovered.candidates.iter().find(|c| {
                        c.origin
                            .as_ref()
                            .is_some_and(|o| o.relative_path == origin.relative_path)
                    }) {
                        row.status = if c.content_hash == s.content_hash {
                            "up_to_date"
                        } else {
                            "update_available"
                        }
                        .into();
                        row.message = if c.content_hash == s.content_hash {
                            "内容已是最新"
                        } else {
                            "发现更新，可预览后安装"
                        }
                        .into();
                        row.candidate_id = Some(c.id.clone());
                        self.cache
                            .lock()
                            .unwrap()
                            .update_bases
                            .insert(format!("{}/{}", s.id, c.id), s.content_hash.clone());
                    } else {
                        row.status = "unavailable".into();
                        row.message = "远端原始路径已消失；未按同名条目替换".into();
                    }
                }
            }
            result.push(row);
        }
        Ok(result)
    }
    pub fn list_mcp(&self) -> ExtResult<McpSnapshot> {
        let (revision, doc) = self.load_mcp()?;
        let targets = self.targets()?;
        let servers = doc
            .servers
            .iter()
            .map(|s| McpServerView {
                id: s.id.clone(),
                name: s.name.clone(),
                description: s.description.clone(),
                transport: s.config.transport.clone(),
                config_json: mcp::redacted_json(&s.config.fields),
                pending_delete: s.pending_delete,
                bindings: s
                    .bindings
                    .iter()
                    .map(|b| {
                        let observed_state = targets
                            .iter()
                            .find(|t| t.id == b.target_id)
                            .and_then(|t| {
                                mcp::read_config(t).ok().map(|(_, doc, _)| {
                                    match mcp::entries(&t.id, &doc).and_then(|o| o.get(&b.key)) {
                                        None => "disabled",
                                        Some(v)
                                            if b.last_applied_hash.as_ref()
                                                != Some(&mcp::entry_hash(v)) =>
                                        {
                                            "conflict"
                                        }
                                        Some(v)
                                            if !mcp::effective_enabled(&t.id, &b.key, v, &doc) =>
                                        {
                                            "disabled"
                                        }
                                        _ => "enabled",
                                    }
                                })
                            })
                            .unwrap_or("unavailable");
                        McpBindingView {
                            target_id: b.target_id.clone(),
                            key: b.key.clone(),
                            desired_enabled: b.desired_enabled,
                            observed_state: observed_state.into(),
                            extra_json: mcp::redacted_json(&b.extra),
                        }
                    })
                    .collect(),
            })
            .collect();
        Ok(McpSnapshot { revision, servers })
    }
    pub fn scan_mcp_imports(&self) -> ExtResult<McpScan> {
        let mut scan = McpScan {
            candidates: vec![],
            errors: vec![],
        };
        let (_, managed) = self.load_mcp()?;
        for t in self.targets()? {
            match mcp::read_config(&t) {
                Ok((revision, doc, _)) => {
                    if let Some(entries) = mcp::entries(&t.id, &doc) {
                        for (key, value) in entries {
                            if managed
                                .servers
                                .iter()
                                .flat_map(|s| &s.bindings)
                                .any(|b| b.target_id == t.id && &b.key == key)
                            {
                                continue;
                            }
                            match mcp::decode(&t.id, value) {
                                Ok((config, extra)) => {
                                    let view = McpCandidate {
                                        id: uuid::Uuid::new_v4().to_string(),
                                        target_id: t.id.clone(),
                                        key: key.clone(),
                                        transport: config.transport.clone(),
                                        config_json: mcp::redacted_json(&config.fields),
                                        enabled: mcp::effective_enabled(&t.id, key, value, &doc),
                                    };
                                    self.cache.lock().unwrap().mcp.insert(
                                        view.id.clone(),
                                        StoredMcpCandidate {
                                            view: view.clone(),
                                            config,
                                            extra,
                                            entry_hash: mcp::entry_hash(value),
                                            file_revision: revision.clone(),
                                        },
                                    );
                                    scan.candidates.push(view);
                                }
                                Err(e) => scan.errors.push(Diagnostic {
                                    target_id: t.id.clone(),
                                    message: e.message,
                                }),
                            }
                        }
                    }
                }
                Err(e) => scan.errors.push(Diagnostic {
                    target_id: t.id,
                    message: e.message,
                }),
            }
        }
        Ok(scan)
    }
}
pub(crate) fn observe_skill(d: &SkillDeployment, content: &Path) -> String {
    let p = Path::new(&d.path);
    if d.mode == DeployMode::External {
        return if p.exists() { "external" } else { "missing" }.into();
    }
    if !files::exists(p) {
        return "disabled".into();
    }
    if d.mode == DeployMode::Symlink {
        return if std::fs::read_link(p).ok().is_some_and(|l| l == content) {
            "enabled"
        } else {
            "conflict"
        }
        .into();
    }
    if files::tree_hash(p).ok().as_ref() == d.last_applied_hash.as_ref() {
        "enabled"
    } else {
        "conflict"
    }
    .into()
}

#[cfg(test)]
mod tests;
