use crate::extensions::{
    err, files,
    model::{AgentOverride, McpTransport},
    ExtResult,
};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::{collections::BTreeMap, path::PathBuf};
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentTarget {
    pub id: String,
    pub name: String,
    pub scope: String,
    pub skills_dir: String,
    pub mcp_file: String,
    pub path_source: String,
    pub available: bool,
    pub transports: Vec<McpTransport>,
    pub shared_with: Vec<String>,
    pub scan_dirs: Vec<String>,
}
#[derive(Clone)]
pub struct AgentRegistry {
    home: PathBuf,
    env: BTreeMap<String, String>,
}
impl AgentRegistry {
    pub fn isolated(home: PathBuf) -> Self {
        Self {
            home,
            env: BTreeMap::new(),
        }
    }
    pub fn production() -> Self {
        Self {
            home: dirs::home_dir().unwrap_or_else(|| PathBuf::from("/unavailable")),
            env: std::env::vars()
                .filter(|(k, _)| {
                    ["CLAUDE_CONFIG_DIR", "CODEX_HOME", "GEMINI_CLI_HOME"].contains(&k.as_str())
                })
                .collect(),
        }
    }
    pub fn targets(
        &self,
        overrides: &BTreeMap<String, AgentOverride>,
    ) -> ExtResult<Vec<AgentTarget>> {
        let mut result = vec![];
        for (id, name, folder, env_key) in [
            ("claude", "Claude Code", ".claude", "CLAUDE_CONFIG_DIR"),
            ("codex", "Codex", ".codex", "CODEX_HOME"),
            ("gemini", "Gemini CLI", ".gemini", "GEMINI_CLI_HOME"),
        ] {
            let root = self
                .env
                .get(env_key)
                .map(PathBuf::from)
                .unwrap_or_else(|| self.home.join(folder));
            let config = if id == "claude" {
                if self.env.contains_key(env_key) {
                    root.join(".claude.json")
                } else {
                    self.home.join(".claude.json")
                }
            } else if id == "codex" {
                root.join("config.toml")
            } else {
                root.join("settings.json")
            };
            // Codex and Gemini share the current interoperable user location. Legacy locations remain read-only scan roots.
            let skill_dir = if id == "claude" {
                root.join("skills")
            } else {
                self.home.join(".agents/skills")
            };
            let custom = overrides.get(id);
            let skill_dir = custom
                .and_then(|o| o.skills_dir.as_ref())
                .map(PathBuf::from)
                .unwrap_or(skill_dir);
            let config = custom
                .and_then(|o| o.mcp_file.as_ref())
                .map(PathBuf::from)
                .unwrap_or(config);
            files::physical(&skill_dir)?;
            files::physical(&config)?;
            let mut scan_dirs = vec![skill_dir.display().to_string()];
            if custom.and_then(|o| o.skills_dir.as_ref()).is_none() && id != "claude" {
                scan_dirs.push(root.join("skills").display().to_string());
            }
            result.push(AgentTarget {
                id: id.into(),
                name: name.into(),
                scope: "user".into(),
                skills_dir: skill_dir.display().to_string(),
                mcp_file: config.display().to_string(),
                path_source: if custom.is_some() {
                    "override"
                } else if self.env.contains_key(env_key) {
                    "environment"
                } else {
                    "default"
                }
                .into(),
                available: root.is_dir(),
                transports: if id == "codex" {
                    vec![McpTransport::Stdio, McpTransport::Http]
                } else {
                    vec![McpTransport::Stdio, McpTransport::Http, McpTransport::Sse]
                },
                shared_with: vec![],
                scan_dirs,
            });
        }
        for i in 0..result.len() {
            for j in 0..result.len() {
                if i != j
                    && resolved_directory(&result[i].skills_dir)?
                        == resolved_directory(&result[j].skills_dir)?
                {
                    let id = result[j].id.clone();
                    result[i].shared_with.push(id);
                }
            }
        }
        for key in overrides.keys() {
            if !result.iter().any(|t| &t.id == key) {
                return Err(err("invalid_request", "未知应用 ID"));
            }
        }
        Ok(result)
    }
}

fn resolved_directory(path: &str) -> ExtResult<PathBuf> {
    let p = PathBuf::from(path);
    if p.is_dir() {
        p.canonicalize()
            .map_err(|_| err("unavailable", "Skill 目录无法解析"))
    } else {
        files::physical(&p)
    }
}
