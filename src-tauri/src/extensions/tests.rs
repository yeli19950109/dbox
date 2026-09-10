use super::*;
use crate::{
    agents::AgentRegistry,
    api::MemoryApiEventEmitter,
    persistence::{AppPaths, PersistenceStore},
};
use serde_json::json;
use std::{
    fs,
    io::{Cursor, Write},
    path::PathBuf,
    sync::Arc,
};
use tempfile::TempDir;

struct Fixture {
    _temp: TempDir,
    root: PathBuf,
    service: Arc<ExtensionService>,
    events: Arc<MemoryApiEventEmitter>,
}
impl Fixture {
    fn new() -> Self {
        let temp = TempDir::new().unwrap();
        let root = temp.path().to_path_buf();
        let store = Arc::new(PersistenceStore::new(AppPaths::new(
            root.join("config"),
            root.join("data"),
            root.join("logs"),
        )));
        let events = Arc::new(MemoryApiEventEmitter::default());
        let service = Arc::new(ExtensionService::new(
            store,
            AgentRegistry::isolated(root.join("home")),
            events.clone(),
        ));
        Self {
            _temp: temp,
            root,
            service,
            events,
        }
    }
    fn skill(&self, relative: &str, text: &str) -> PathBuf {
        let path = self.root.join(relative);
        fs::create_dir_all(&path).unwrap();
        fs::write(
            path.join("SKILL.md"),
            format!(
                "---\nname: demo\ndescription: Test skill\nunknown: preserve-me\n---\n{text}\n"
            ),
        )
        .unwrap();
        path
    }
    async fn discover(&self, path: &Path) -> SkillCandidate {
        self.service
            .discover(DiscoverRequest {
                request_id: uuid::Uuid::new_v4().to_string(),
                source: SkillSource {
                    id: "test".into(),
                    kind: SourceKind::Local,
                    uri: path.display().to_string(),
                    requested_ref: None,
                    enabled: true,
                },
            })
            .await
            .unwrap()
            .candidates
            .remove(0)
    }
    async fn execute(&self, plan: ExtensionPlan) -> ExtensionResult {
        let started = self
            .service
            .confirm(ConfirmExtensionRequest {
                plan_id: plan.plan_id,
                plan_hash: plan.plan_hash,
            })
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                let result = self.service.result(&started.run_id).unwrap();
                if result.status != "running" {
                    return result;
                }
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap()
    }
    async fn skill_op(&self, req: SkillOperationRequest) -> ExtensionResult {
        self.execute(self.service.preview_skill(req).unwrap()).await
    }
    async fn mcp_op(&self, req: McpOperationRequest) -> ExtensionResult {
        self.execute(self.service.preview_mcp(req).unwrap()).await
    }
}
fn skill_req(
    action: SkillAction,
    ids: Vec<String>,
    candidates: Vec<String>,
    targets: &[&str],
    enabled: bool,
) -> SkillOperationRequest {
    SkillOperationRequest {
        action,
        skill_ids: ids,
        candidate_ids: candidates,
        target_ids: targets.iter().map(|s| (*s).into()).collect(),
        enabled,
        mode: DeployMode::Auto,
        backup_id: None,
    }
}
fn mcp_req(
    action: McpAction,
    ids: Vec<String>,
    targets: &[&str],
    enabled: bool,
) -> McpOperationRequest {
    McpOperationRequest {
        action,
        server_ids: ids,
        candidate_ids: vec![],
        target_ids: targets.iter().map(|s| (*s).into()).collect(),
        enabled,
        edit: None,
        import_json: None,
        backup_id: None,
    }
}
fn editor(transport: McpTransport, config: Value, targets: &[&str]) -> McpEdit {
    McpEdit {
        id: None,
        name: "demo".into(),
        description: "safe description".into(),
        transport,
        config_json: config.to_string(),
        secrets: BTreeMap::new(),
        bindings: targets
            .iter()
            .map(|t| McpBindingView {
                target_id: (*t).into(),
                key: "demo".into(),
                desired_enabled: true,
                observed_state: "disabled".into(),
                extra_json: "{}".into(),
            })
            .collect(),
    }
}
#[test]
fn targets_and_queries_do_not_initialize_agents_or_extension_documents() {
    let f = Fixture::new();
    let targets = f.service.targets().unwrap();
    assert_eq!(targets.len(), 3);
    assert_eq!(targets[1].shared_with, vec!["gemini"]);
    assert!(f.service.list_skills().unwrap().skills.is_empty());
    assert!(f.service.list_mcp().unwrap().servers.is_empty());
    assert!(!f.root.join("home").exists());
    assert!(!f.root.join("config/skills.json").exists());
}
#[test]
fn corrupt_json_is_not_overwritten_and_source_revisions_conflict() {
    let f = Fixture::new();
    fs::create_dir_all(f.root.join("config")).unwrap();
    fs::write(f.service.skills_file(), b"{broken").unwrap();
    assert!(f.service.list_skills().is_err());
    assert_eq!(fs::read(f.service.skills_file()).unwrap(), b"{broken");
    fs::remove_file(f.service.skills_file()).unwrap();
    let source = SkillSource {
        id: "a".into(),
        kind: SourceKind::Github,
        uri: "owner/repo".into(),
        requested_ref: Some("main".into()),
        enabled: true,
    };
    f.service
        .save_source(SaveSourceRequest {
            expected_revision: "missing".into(),
            source: source.clone(),
        })
        .unwrap();
    assert_eq!(
        f.service
            .save_source(SaveSourceRequest {
                expected_revision: "missing".into(),
                source
            })
            .unwrap_err()
            .code,
        ApiErrorCode::Conflict
    );
}
#[tokio::test]
async fn full_skill_import_adopt_disable_offline_enable_uninstall_restore() {
    let f = Fixture::new();
    let external = f.skill("home/.claude/skills/demo", "Original body");
    let original = files::tree_hash(&external).unwrap();
    let imports = f.service.scan_skill_imports().unwrap();
    assert_eq!(imports.candidates.len(), 1);
    let imported = f
        .skill_op(skill_req(
            SkillAction::Import,
            vec![],
            vec![imports.candidates[0].id.clone()],
            &[],
            true,
        ))
        .await;
    assert_eq!(imported.status, "succeeded");
    assert_eq!(files::tree_hash(&external).unwrap(), original);
    assert!(!fs::symlink_metadata(&external)
        .unwrap()
        .file_type()
        .is_symlink());
    let id = f.service.list_skills().unwrap().skills[0].id.clone();
    let untouched = f
        .skill_op(skill_req(
            SkillAction::Toggle,
            vec![id.clone()],
            vec![],
            &["claude"],
            false,
        ))
        .await;
    assert_eq!(untouched.status, "failed");
    assert!(external.exists());
    assert_eq!(
        f.skill_op(skill_req(
            SkillAction::Adopt,
            vec![id.clone()],
            vec![],
            &["claude"],
            true
        ))
        .await
        .status,
        "succeeded"
    );
    assert!(fs::symlink_metadata(&external)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(
        f.skill_op(skill_req(
            SkillAction::Toggle,
            vec![id.clone()],
            vec![],
            &["claude"],
            false
        ))
        .await
        .status,
        "succeeded"
    );
    assert!(!files::exists(&external));
    assert!(f.service.content_dir(&id).join("SKILL.md").exists());
    assert_eq!(
        f.skill_op(skill_req(
            SkillAction::Toggle,
            vec![id.clone()],
            vec![],
            &["claude"],
            true
        ))
        .await
        .status,
        "succeeded"
    );
    let uninstalled = f
        .skill_op(skill_req(
            SkillAction::Uninstall,
            vec![id.clone()],
            vec![],
            &[],
            true,
        ))
        .await;
    assert_eq!(uninstalled.status, "succeeded");
    assert!(f.service.list_skills().unwrap().skills.is_empty());
    assert!(!external.exists());
    let mut restore = skill_req(SkillAction::Restore, vec![], vec![], &[], true);
    restore.backup_id = uninstalled.backup_id;
    assert_eq!(f.skill_op(restore).await.status, "succeeded");
    assert_eq!(files::tree_hash(&external).unwrap(), original);
    assert_eq!(f.service.list_skills().unwrap().skills.len(), 1);
}
#[tokio::test]
async fn shared_targets_install_once_and_disable_together() {
    let f = Fixture::new();
    let source = f.skill("source/demo", "body");
    let c = f.discover(&source).await;
    let plan = f
        .service
        .preview_skill(skill_req(
            SkillAction::Install,
            vec![],
            vec![c.id],
            &["codex"],
            true,
        ))
        .unwrap();
    assert_eq!(plan.steps.len(), 2);
    assert!(plan.warnings.iter().any(|w| w.contains("关联")));
    assert_eq!(f.execute(plan).await.status, "succeeded");
    let skill = f.service.list_skills().unwrap().skills.remove(0);
    assert_eq!(skill.deployments.len(), 2);
    assert_eq!(
        f.skill_op(skill_req(
            SkillAction::Toggle,
            vec![skill.id],
            vec![],
            &["gemini"],
            false
        ))
        .await
        .status,
        "succeeded"
    );
    assert!(f.service.list_skills().unwrap().skills[0]
        .deployments
        .iter()
        .all(|d| !d.desired_enabled && d.observed_state == "disabled"));
}
#[tokio::test]
async fn same_names_are_separate_and_foreign_path_is_not_overwritten() {
    let f = Fixture::new();
    let a = f.skill("a/demo", "a");
    let b = f.skill("b/demo", "b");
    let a = f.discover(&a).await;
    let b = f.discover(&b).await;
    assert_eq!(
        f.skill_op(skill_req(
            SkillAction::Install,
            vec![],
            vec![a.id],
            &["claude"],
            true
        ))
        .await
        .status,
        "succeeded"
    );
    let second = f
        .skill_op(skill_req(
            SkillAction::Install,
            vec![],
            vec![b.id],
            &["claude"],
            true,
        ))
        .await;
    assert_eq!(second.status, "partial");
    assert_eq!(f.service.list_skills().unwrap().skills.len(), 2);
}
#[tokio::test]
async fn copy_local_modification_blocks_disable_and_preserves_library() {
    let f = Fixture::new();
    let source = f.skill("source/demo", "body");
    let c = f.discover(&source).await;
    let mut req = skill_req(SkillAction::Install, vec![], vec![c.id], &["claude"], true);
    req.mode = DeployMode::Copy;
    assert_eq!(f.skill_op(req).await.status, "succeeded");
    let s = f.service.list_skills().unwrap().skills.remove(0);
    fs::write(Path::new(&s.deployments[0].path).join("local.txt"), "edit").unwrap();
    let result = f
        .skill_op(skill_req(
            SkillAction::Uninstall,
            vec![s.id.clone()],
            vec![],
            &[],
            true,
        ))
        .await;
    assert_eq!(result.status, "failed");
    assert!(f.service.content_dir(&s.id).exists());
    assert!(Path::new(&s.deployments[0].path).join("local.txt").exists());
}
#[tokio::test]
async fn stale_confirmation_rejects_changed_document_and_changed_candidate() {
    let f = Fixture::new();
    let source = f.skill("source/demo", "body");
    let c = f.discover(&source).await;
    let p = f
        .service
        .preview_skill(skill_req(
            SkillAction::Install,
            vec![],
            vec![c.id],
            &[],
            true,
        ))
        .unwrap();
    fs::create_dir_all(f.root.join("config")).unwrap();
    fs::write(f.service.skills_file(), "{} ").unwrap();
    assert_eq!(
        f.service
            .confirm(ConfirmExtensionRequest {
                plan_id: p.plan_id,
                plan_hash: p.plan_hash
            })
            .unwrap_err()
            .code,
        ApiErrorCode::Conflict
    );
}
#[tokio::test]
async fn local_skill_check_does_not_touch_library_or_agents() {
    let f = Fixture::new();
    let source = f.skill("source/demo", "body");
    let c = f.discover(&source).await;
    f.skill_op(skill_req(
        SkillAction::Install,
        vec![],
        vec![c.id],
        &["claude"],
        true,
    ))
    .await;
    let before = f.service.list_skills().unwrap();
    let state = fs::read(f.service.skills_file()).unwrap();
    let library = files::tree_hash(&f.service.content_dir(&before.skills[0].id)).unwrap();
    let updates = f
        .service
        .check_updates(CheckUpdatesRequest {
            request_id: uuid::Uuid::new_v4().to_string(),
            skill_ids: vec![],
        })
        .await
        .unwrap();
    assert_eq!(updates[0].status, "source_unknown");
    assert_eq!(state, fs::read(f.service.skills_file()).unwrap());
    assert_eq!(
        library,
        files::tree_hash(&f.service.content_dir(&before.skills[0].id)).unwrap()
    );
}
fn archive(names: &[(&str, &str)]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(vec![]));
    for (name, content) in names {
        writer
            .start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(content.as_bytes()).unwrap();
    }
    writer.finish().unwrap().into_inner()
}
#[test]
fn zip_rejects_traversal_duplicates_case_aliases_and_special_files() {
    let f = Fixture::new();
    for names in [
        vec![("../escape", "x")],
        vec![("/absolute", "x")],
        vec![("C:/escape", "x")],
        vec![("one/a", "x"), ("one/A", "y")],
    ] {
        let dir = f.root.join(uuid::Uuid::new_v4().to_string());
        assert!(skills::extract_zip(&archive(&names), &dir, &CancellationToken::new()).is_err());
    }
    let mut duplicate = archive(&[("same", "x"), ("diff", "y")]);
    for i in 0..duplicate.len().saturating_sub(3) {
        if &duplicate[i..i + 4] == b"diff" {
            duplicate[i..i + 4].copy_from_slice(b"same");
        }
    }
    assert!(skills::extract_zip(
        &duplicate,
        &f.root.join("duplicate"),
        &CancellationToken::new()
    )
    .is_err());
    let mut writer = zip::ZipWriter::new(Cursor::new(vec![]));
    writer
        .add_symlink("link", "/outside", zip::write::SimpleFileOptions::default())
        .unwrap();
    let bytes = writer.finish().unwrap().into_inner();
    assert!(skills::extract_zip(&bytes, &f.root.join("links"), &CancellationToken::new()).is_err());
}
#[tokio::test]
async fn zip_discovers_multiple_skills_and_preserves_raw_frontmatter() {
    let f = Fixture::new();
    let path = f.root.join("skills.zip");
    fs::write(
        &path,
        archive(&[
            (
                "a/SKILL.md",
                "---\nname: same\ndescription: alpha\ncustom: keep\n---\nbody",
            ),
            (
                "b/SKILL.md",
                "---\nname: same\ndescription: beta\n---\nbody",
            ),
        ]),
    )
    .unwrap();
    let result = f
        .service
        .discover(DiscoverRequest {
            request_id: uuid::Uuid::new_v4().to_string(),
            source: SkillSource {
                id: "zip".into(),
                kind: SourceKind::Zip,
                uri: path.display().to_string(),
                requested_ref: None,
                enabled: true,
            },
        })
        .await
        .unwrap();
    assert_eq!(result.candidates.len(), 2);
    assert!(result.candidates[0].content.contains("custom: keep"));
    assert_ne!(result.candidates[0].id, result.candidates[1].id);
}
#[tokio::test]
async fn mcp_import_is_read_only_and_corrupt_agent_is_isolated() {
    let f = Fixture::new();
    fs::create_dir_all(f.root.join("home/.gemini")).unwrap();
    fs::create_dir_all(f.root.join("home/.codex")).unwrap();
    let claude = f.root.join("home/.claude.json");
    let bytes = br#"{"projects":{"untouched":true},"mcpServers":{"demo":{"command":"npx","args":["private-secret"],"env":{"TOKEN":"secret-env"}}}}"#;
    fs::write(&claude, bytes).unwrap();
    fs::write(f.root.join("home/.codex/config.toml"), "invalid [toml").unwrap();
    let scan = f.service.scan_mcp_imports().unwrap();
    assert_eq!(scan.candidates.len(), 1);
    assert_eq!(scan.errors.len(), 1);
    assert!(!serde_json::to_string(&scan).unwrap().contains("secret-env"));
    let mut req = mcp_req(McpAction::Import, vec![], &[], true);
    req.candidate_ids = vec![scan.candidates[0].id.clone()];
    assert_eq!(f.mcp_op(req).await.status, "succeeded");
    assert_eq!(fs::read(&claude).unwrap(), bytes);
}
#[tokio::test]
async fn mcp_mapping_preserves_unmanaged_fields_comments_and_redacts_secrets() {
    let f = Fixture::new();
    fs::create_dir_all(f.root.join("home/.codex")).unwrap();
    fs::write(f.root.join("home/.codex/config.toml"), "# keep top comment\nmodel = 'local-model' # keep inline\n[mcp_servers.unmanaged]\ncommand = 'untouched'\n").unwrap();
    let mut req = mcp_req(McpAction::Upsert, vec![], &[], true);
    req.edit = Some(editor(
        McpTransport::Http,
        json!({"url":"https://example.test/mcp?token=private-url", "headers":{"Authorization":"private-header"}}),
        &["claude", "codex", "gemini"],
    ));
    let preview = f.service.preview_mcp(req).unwrap();
    let serialized = serde_json::to_string(&preview).unwrap();
    assert!(!serialized.contains("private-url"));
    assert!(!serialized.contains("private-header"));
    assert_eq!(f.execute(preview).await.status, "succeeded");
    let codex = fs::read_to_string(f.root.join("home/.codex/config.toml")).unwrap();
    assert!(codex.contains("# keep top comment"));
    assert!(codex.contains("# keep inline"));
    assert!(codex.contains("unmanaged"));
    assert!(codex.contains("http_headers"));
    assert!(!codex.contains("type ="));
    let gemini: Value =
        serde_json::from_slice(&fs::read(f.root.join("home/.gemini/settings.json")).unwrap())
            .unwrap();
    assert!(gemini["mcpServers"]["demo"]["httpUrl"].is_string());
    let list = f.service.list_mcp().unwrap();
    let serialized = serde_json::to_string(&list).unwrap();
    assert!(!serialized.contains("private-url"));
    assert!(!serialized.contains("private-header"));
    let events = format!("{:?}", f.events.events());
    assert!(!events.contains("private-header"));
}
#[tokio::test]
async fn mcp_edit_keep_set_remove_and_disabled_binding_are_correct() {
    let f = Fixture::new();
    let mut req = mcp_req(McpAction::Upsert, vec![], &[], true);
    req.edit = Some(editor(
        McpTransport::Stdio,
        json!({"command":"npx", "env":{"KEEP":"alpha", "SET":"beta", "REMOVE":"gamma"}}),
        &["codex"],
    ));
    assert_eq!(f.mcp_op(req).await.status, "succeeded");
    let server = f.service.list_mcp().unwrap().servers.remove(0);
    let mut edit = editor(McpTransport::Stdio, json!({}), &[]);
    edit.id = Some(server.id.clone());
    edit.config_json = server.config_json;
    edit.bindings = server.bindings;
    edit.secrets.insert("/env/KEEP".into(), SecretEdit::Keep);
    edit.secrets
        .insert("/env/SET".into(), SecretEdit::Set("changed".into()));
    edit.secrets
        .insert("/env/REMOVE".into(), SecretEdit::Remove);
    let mut req = mcp_req(McpAction::Upsert, vec![], &[], true);
    req.edit = Some(edit);
    assert_eq!(f.mcp_op(req).await.status, "succeeded");
    let (_, doc) = f.service.load_mcp().unwrap();
    assert_eq!(
        doc.servers[0].config.fields["env"],
        json!({"KEEP":"alpha", "SET":"changed"})
    );
    assert_eq!(
        f.mcp_op(mcp_req(
            McpAction::Toggle,
            vec![server.id],
            &["codex"],
            false
        ))
        .await
        .status,
        "succeeded"
    );
    assert_eq!(
        f.service.list_mcp().unwrap().servers[0].bindings[0].observed_state,
        "disabled"
    );
}
#[tokio::test]
async fn mcp_external_change_after_preview_is_partial_and_restoration_protects_new_edits() {
    let f = Fixture::new();
    let mut req = mcp_req(McpAction::Upsert, vec![], &[], true);
    req.edit = Some(editor(
        McpTransport::Stdio,
        json!({"command":"npx"}),
        &["claude", "gemini"],
    ));
    let p = f.service.preview_mcp(req).unwrap();
    fs::create_dir_all(f.root.join("home")).unwrap();
    fs::write(f.root.join("home/.claude.json"), r#"{"external":"keep"}"#).unwrap();
    let result = f.execute(p).await;
    assert_eq!(result.status, "partial");
    assert_eq!(
        fs::read_to_string(f.root.join("home/.claude.json")).unwrap(),
        r#"{"external":"keep"}"#
    );
    fs::write(
        f.root.join("home/.gemini/settings.json"),
        "{\"changed\":true}",
    )
    .unwrap();
    let mut restore = mcp_req(McpAction::Restore, vec![], &[], true);
    restore.backup_id = result.backup_id;
    assert!(f.service.preview_mcp(restore).is_err());
}
#[tokio::test]
async fn mcp_sse_unsupported_only_for_codex_and_multiple_names_keep_identity() {
    let f = Fixture::new();
    let mut req = mcp_req(McpAction::Upsert, vec![], &[], true);
    req.edit = Some(editor(
        McpTransport::Sse,
        json!({"url":"https://example.test/sse"}),
        &["codex", "gemini"],
    ));
    assert_eq!(f.mcp_op(req).await.status, "partial");
    let mut import = mcp_req(McpAction::Import, vec![], &[], true);
    import.import_json = Some(r#"{"demo":{"command":"node"}}"#.into());
    assert_eq!(f.mcp_op(import).await.status, "succeeded");
    assert_eq!(f.service.list_mcp().unwrap().servers.len(), 2);
}
#[tokio::test]
async fn cancelled_operation_skips_files_and_recovery_marks_unfinished_runs() {
    let f = Fixture::new();
    let source = f.skill("source/demo", "body");
    let c = f.discover(&source).await;
    let p = f
        .service
        .preview_skill(skill_req(
            SkillAction::Install,
            vec![],
            vec![c.id],
            &["claude"],
            true,
        ))
        .unwrap();
    let guard = f.service.serial.lock().unwrap();
    let started = f
        .service
        .confirm(ConfirmExtensionRequest {
            plan_id: p.plan_id,
            plan_hash: p.plan_hash,
        })
        .unwrap();
    assert!(f.service.cancel(&started.run_id));
    drop(guard);
    loop {
        let r = f.service.result(&started.run_id).unwrap();
        if r.status != "running" {
            assert_eq!(r.status, "cancelled");
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    assert!(!f.root.join("home/.claude/skills/demo").exists());
    f.service
        .store
        .update_cached_state(|state| {
            state.runs[0].status = crate::domain::RunStatus::Running;
        })
        .unwrap();
    f.service.recover().unwrap();
    assert_eq!(
        f.service.store.load_state().unwrap().value.runs[0].status,
        crate::domain::RunStatus::Interrupted
    );
}

#[cfg(feature = "dev-http")]
#[tokio::test]
async fn remote_discovery_pins_commit_check_is_read_only_and_update_keeps_origin() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let version = Arc::new(AtomicUsize::new(1));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let phase = version.clone();
    let requested = Arc::new(Mutex::new(vec![]));
    let requests = requested.clone();
    let server = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = [0u8; 4096];
            let len = socket.read(&mut bytes).await.unwrap();
            let line = String::from_utf8_lossy(&bytes[..len]);
            let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
            requests.lock().unwrap().push(path.clone());
            let v = phase.load(Ordering::SeqCst);
            let (status, body) = if v == 3 {
                ("500 Internal Server Error", b"unavailable".to_vec())
            } else if path.starts_with("/repos/") {
                (
                    "200 OK",
                    format!(
                        "{{\"sha\":\"{}\"}}",
                        if v == 1 {
                            "1".repeat(40)
                        } else {
                            "2".repeat(40)
                        }
                    )
                    .into_bytes(),
                )
            } else {
                let body = if path.contains(&"1".repeat(40)) {
                    "version-one"
                } else {
                    "version-two"
                };
                (
                    "200 OK",
                    archive(&[(
                        "repo/skills/demo/SKILL.md",
                        &format!("---\nname: demo\ndescription: Remote\n---\n{body}"),
                    )]),
                )
            };
            let header = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            socket.write_all(header.as_bytes()).await.unwrap();
            socket.write_all(&body).await.unwrap();
        }
    });
    let mut f = Fixture::new();
    let mut fetcher = SkillFetcher::default();
    fetcher.github_api = format!("http://{address}");
    fetcher.github_archive = format!("http://{address}");
    Arc::get_mut(&mut f.service).unwrap().fetcher = fetcher;
    let source = SkillSource {
        id: "remote".into(),
        kind: SourceKind::Github,
        uri: "owner/repo".into(),
        requested_ref: Some("main".into()),
        enabled: true,
    };
    let discovery = f
        .service
        .discover(DiscoverRequest {
            request_id: uuid::Uuid::new_v4().to_string(),
            source,
        })
        .await
        .unwrap();
    assert!(discovery.errors.is_empty());
    let c = &discovery.candidates[0];
    assert_eq!(c.origin.as_ref().unwrap().relative_path, "skills/demo");
    let p = f
        .service
        .preview_skill(skill_req(
            SkillAction::Install,
            vec![],
            vec![c.id.clone()],
            &["claude", "codex"],
            true,
        ))
        .unwrap();
    version.store(2, Ordering::SeqCst);
    assert_eq!(f.execute(p).await.status, "succeeded");
    let s = f.service.list_skills().unwrap().skills.remove(0);
    let content = f.service.content_dir(&s.id);
    assert!(fs::read_to_string(content.join("SKILL.md"))
        .unwrap()
        .contains("version-one"));
    let before = fs::read(f.service.skills_file()).unwrap();
    let updates = f
        .service
        .check_updates(CheckUpdatesRequest {
            request_id: uuid::Uuid::new_v4().to_string(),
            skill_ids: vec![s.id.clone()],
        })
        .await
        .unwrap();
    assert_eq!(updates[0].status, "update_available");
    assert_eq!(fs::read(f.service.skills_file()).unwrap(), before);
    assert!(fs::read_to_string(content.join("SKILL.md"))
        .unwrap()
        .contains("version-one"));
    assert_eq!(
        f.skill_op(skill_req(
            SkillAction::Update,
            vec![s.id.clone()],
            vec![updates[0].candidate_id.clone().unwrap()],
            &[],
            true
        ))
        .await
        .status,
        "succeeded"
    );
    assert!(fs::read_to_string(content.join("SKILL.md"))
        .unwrap()
        .contains("version-two"));
    let current = f.service.list_skills().unwrap().skills.remove(0);
    assert_eq!(current.deployments.len(), 3);
    assert_eq!(
        current.origin.unwrap().resolved_revision,
        Some("2".repeat(40))
    );
    version.store(3, Ordering::SeqCst);
    let failed = f
        .service
        .check_updates(CheckUpdatesRequest {
            request_id: uuid::Uuid::new_v4().to_string(),
            skill_ids: vec![s.id],
        })
        .await
        .unwrap();
    assert_eq!(failed[0].status, "unavailable");
    assert!(requested
        .lock()
        .unwrap()
        .iter()
        .any(|path| path == "/repos/owner/repo/commits/main"));
    server.abort();
}
#[tokio::test]
async fn mcp_delete_restore_and_delete_backup_actually_free_storage() {
    let f = Fixture::new();
    let mut req = mcp_req(McpAction::Upsert, vec![], &[], true);
    req.edit = Some(editor(
        McpTransport::Stdio,
        json!({"command":"npx"}),
        &["claude"],
    ));
    f.mcp_op(req).await;
    let s = f.service.list_mcp().unwrap().servers.remove(0);
    let removed = f
        .mcp_op(mcp_req(McpAction::Delete, vec![s.id], &[], true))
        .await;
    assert_eq!(removed.status, "succeeded");
    assert!(f.service.list_mcp().unwrap().servers.is_empty());
    let mut restore = mcp_req(McpAction::Restore, vec![], &[], true);
    restore.backup_id = removed.backup_id.clone();
    assert_eq!(f.mcp_op(restore).await.status, "succeeded");
    assert_eq!(f.service.list_mcp().unwrap().servers.len(), 1);
    let count = f.service.list_backups().unwrap().len();
    let p = f
        .service
        .restore_plan(&removed.backup_id.unwrap(), ResourceKind::Mcp, true)
        .unwrap();
    assert_eq!(f.execute(p).await.status, "succeeded");
    assert_eq!(f.service.list_backups().unwrap().len(), count - 1);
}
#[tokio::test]
async fn gemini_global_exclusion_is_observed_and_not_silently_overridden() {
    let f = Fixture::new();
    fs::create_dir_all(f.root.join("home/.gemini")).unwrap();
    let path = f.root.join("home/.gemini/settings.json");
    fs::write(
        &path,
        r#"{"mcp":{"excluded":["demo"]},"mcpServers":{"demo":{"command":"node"}}}"#,
    )
    .unwrap();
    let before = fs::read(&path).unwrap();
    let scan = f.service.scan_mcp_imports().unwrap();
    assert!(!scan.candidates[0].enabled);
    let mut req = mcp_req(McpAction::Import, vec![], &[], true);
    req.candidate_ids = vec![scan.candidates[0].id.clone()];
    f.mcp_op(req).await;
    let s = f.service.list_mcp().unwrap().servers.remove(0);
    assert_eq!(s.bindings[0].observed_state, "disabled");
    let result = f
        .mcp_op(mcp_req(McpAction::Toggle, vec![s.id], &["gemini"], true))
        .await;
    assert_eq!(result.status, "failed");
    assert_eq!(fs::read(&path).unwrap(), before);
}
#[test]
fn nested_sensitive_keys_are_redacted_even_when_named_command() {
    let value = json!({"command":"npx", "env":{"command":"secret-env"}, "headers":{"command":"secret-header"}, "args":["secret-arg"], "url":"https://example.test/private"});
    let output = mcp::redacted_json(&value);
    assert!(output.contains("npx"));
    assert!(!output.contains("secret-"));
    assert!(!output.contains("example.test"));
}
#[cfg(unix)]
#[tokio::test]
async fn file_permissions_protect_mcp_records_and_backups() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    let mut req = mcp_req(McpAction::Upsert, vec![], &[], true);
    req.edit = Some(editor(
        McpTransport::Stdio,
        json!({"command":"node","env":{"TOKEN":"hidden"}}),
        &["claude"],
    ));
    let result = f.mcp_op(req).await;
    for path in [
        f.service.mcp_file(),
        f.root.join("home/.claude.json"),
        f.service
            .backups_dir()
            .join(result.backup_id.unwrap())
            .join("manifest.json"),
    ] {
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

#[tokio::test]
async fn library_only_mcp_does_not_initialize_unselected_agents() {
    let f = Fixture::new();
    let mut edit = editor(
        McpTransport::Stdio,
        json!({"command":"node"}),
        &["claude", "codex", "gemini"],
    );
    for b in &mut edit.bindings {
        b.desired_enabled = false;
    }
    let mut req = mcp_req(McpAction::Upsert, vec![], &[], true);
    req.edit = Some(edit);
    let plan = f.service.preview_mcp(req).unwrap();
    assert!(plan.steps.is_empty());
    assert_eq!(f.execute(plan).await.status, "succeeded");
    assert!(!f.root.join("home").exists());
}
#[cfg(unix)]
#[tokio::test]
async fn mcp_file_symlink_is_preserved_and_restore_uses_original_file_contents() {
    let f = Fixture::new();
    fs::create_dir_all(f.root.join("home")).unwrap();
    let original = f.root.join("shared.json");
    let alias = f.root.join("home/.claude.json");
    fs::write(&original, "{\"keep\":true}").unwrap();
    std::os::unix::fs::symlink(&original, &alias).unwrap();
    let mut req = mcp_req(McpAction::Upsert, vec![], &[], true);
    req.edit = Some(editor(
        McpTransport::Stdio,
        json!({"command":"node"}),
        &["claude"],
    ));
    let result = f.mcp_op(req).await;
    assert_eq!(result.status, "succeeded");
    assert!(fs::symlink_metadata(&alias)
        .unwrap()
        .file_type()
        .is_symlink());
    assert!(fs::read_to_string(&original)
        .unwrap()
        .contains("mcpServers"));
    let mut restore = mcp_req(McpAction::Restore, vec![], &[], true);
    restore.backup_id = result.backup_id;
    assert_eq!(f.mcp_op(restore).await.status, "succeeded");
    assert_eq!(fs::read_to_string(&original).unwrap(), "{\"keep\":true}");
    assert!(fs::symlink_metadata(alias)
        .unwrap()
        .file_type()
        .is_symlink());
}
#[test]
fn toml_nested_comments_and_unknown_fields_survive_a_core_edit() {
    let before = b"# top\n[mcp_servers.demo]\ncommand = 'node' # keep command\nargs = ['old']\n[mcp_servers.demo.env] # keep env header\n# keep token comment\nTOKEN = 'secret' # keep token suffix\n";
    let doc = mcp::parse_document("codex", before).unwrap();
    let mut value = doc["mcp_servers"]["demo"].clone();
    value["args"] = json!(["new"]);
    let bytes = mcp::render(
        "codex",
        before,
        &BTreeMap::from([("demo".into(), Some(value))]),
    )
    .unwrap();
    let after = String::from_utf8(bytes).unwrap();
    for text in [
        "# top",
        "# keep command",
        "# keep env header",
        "# keep token comment",
        "# keep token suffix",
    ] {
        assert!(after.contains(text), "missing {text}: {after}");
    }
}
#[test]
fn legacy_tool_runs_migrate_to_subject_without_fabricating_extension_tool_ids() {
    let f = Fixture::new();
    let loaded = f.service.store.load_state().unwrap();
    let mut value = serde_json::to_value(loaded.value).unwrap();
    value["runs"] = json!([{"id":"old-run", "tool_id":"npm:old", "component_ids":["core"], "status":"succeeded", "created_at":"2026-01-01T00:00:00Z", "started_at":null, "finished_at":null}]);
    fs::write(
        f.service.store.paths().state_file(),
        serde_json::to_vec(&json!({"schema_version":1,"state":value})).unwrap(),
    )
    .unwrap();
    let migrated = f.service.store.load_state().unwrap();
    assert_eq!(
        migrated.value.runs[0].subject,
        crate::domain::RunSubject::ToolUpdate
    );
    assert_eq!(
        migrated.value.runs[0].tool_id.as_ref().unwrap().as_str(),
        "npm:old"
    );
    f.service.store.update_cached_state(|_| {}).unwrap();
    let written: Value =
        serde_json::from_slice(&fs::read(f.service.store.paths().state_file()).unwrap()).unwrap();
    assert_eq!(written["schema_version"], 2);
}

#[tokio::test]
async fn restore_recognizes_interruption_between_file_and_record_commits() {
    let f = Fixture::new();
    let mut request = mcp_req(McpAction::Upsert, vec![], &[], true);
    request.edit = Some(editor(
        McpTransport::Stdio,
        json!({"command":"node"}),
        &["claude"],
    ));
    let result = f.mcp_op(request).await;
    let path = f
        .service
        .backups_dir()
        .join(result.backup_id.as_ref().unwrap())
        .join("manifest.json");
    let mut manifest: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    // The file replacement landed, but the management record's applied hash has
    // not been committed yet. This is one of the two recorded JSON revisions.
    let mut previous = manifest["plan"]["mcp"].clone();
    previous["servers"][0]["bindings"][0]["lastAppliedHash"] = Value::Null;
    manifest["previous_mcp"] = previous.clone();
    manifest["view"]["status"] = json!("running");
    manifest["result"]["status"] = json!("running");
    fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    fs::write(f.service.mcp_file(), serde_json::to_vec(&previous).unwrap()).unwrap();
    let mut restore = mcp_req(McpAction::Restore, vec![], &[], true);
    restore.backup_id = result.backup_id;
    assert_eq!(f.mcp_op(restore).await.status, "succeeded");
    assert!(f.service.list_mcp().unwrap().servers.is_empty());
    assert!(!f.root.join("home/.claude.json").exists());
}

#[tokio::test]
async fn backup_budget_is_checked_before_management_or_agent_writes() {
    let f = Fixture::new();
    let mut request = mcp_req(McpAction::Upsert, vec![], &[], true);
    request.edit = Some(editor(
        McpTransport::Stdio,
        json!({"command":"node"}),
        &["claude"],
    ));
    let result = f.mcp_op(request).await;
    let backup = f.service.backups_dir().join(result.backup_id.unwrap());
    let used: u64 = f.service.list_backups().unwrap()[0]
        .size_bytes
        .parse()
        .unwrap();
    // Sparse fixture accounts for logical quota without allocating a GiB of data.
    fs::File::create(backup.join("quota-fixture"))
        .unwrap()
        .set_len(1024 * 1024 * 1024 - used - 1024)
        .unwrap();
    let before = fs::read(f.service.mcp_file()).unwrap();
    let app = fs::read(f.root.join("home/.claude.json")).unwrap();
    let mut edit = editor(McpTransport::Stdio, json!({"command":"node"}), &["claude"]);
    edit.name = "second".into();
    edit.bindings[0].key = "second".into();
    edit.description = "description".repeat(400);
    let mut request = mcp_req(McpAction::Upsert, vec![], &[], true);
    request.edit = Some(edit);
    assert_eq!(f.mcp_op(request).await.status, "failed");
    assert_eq!(fs::read(f.service.mcp_file()).unwrap(), before);
    assert_eq!(fs::read(f.root.join("home/.claude.json")).unwrap(), app);
}
