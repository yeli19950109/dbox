use crate::extensions::{err, files, model::*, ExtResult};
use std::{
    collections::BTreeSet,
    fs,
    io::{Cursor, Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};
use tokio_util::sync::CancellationToken;
use walkdir::WalkDir;

pub fn validate_source(source: &SkillSource) -> ExtResult<()> {
    match source.kind {
        SourceKind::Github => {
            repository(&source.uri)?;
        }
        _ => {
            files::physical(Path::new(&source.uri))?;
        }
    }
    Ok(())
}
pub fn repository(uri: &str) -> ExtResult<String> {
    let value = uri
        .trim()
        .trim_end_matches('/')
        .strip_prefix("https://github.com/")
        .unwrap_or(uri.trim())
        .trim_end_matches(".git");
    let parts: Vec<_> = value.split('/').collect();
    if parts.len() != 2
        || parts.iter().any(|part| {
            part.is_empty()
                || !part
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
                || *part == "."
                || *part == ".."
        })
    {
        return Err(err(
            "invalid_request",
            "GitHub 来源必须为 owner/repository 或公开仓库 URL",
        ));
    }
    Ok(value.to_owned())
}
pub fn metadata(path: &Path) -> ExtResult<(String, String, String)> {
    let bytes = files::read(&path.join("SKILL.md"))?;
    if bytes.len() > 1024 * 1024 {
        return Err(err("invalid_request", "SKILL.md 超过 1 MiB"));
    }
    let raw =
        String::from_utf8(bytes).map_err(|_| err("invalid_request", "SKILL.md 必须为 UTF-8"))?;
    let normalized = raw.trim_start_matches('\u{feff}').replace("\r\n", "\n");
    let rest = normalized
        .strip_prefix("---\n")
        .ok_or_else(|| err("invalid_request", "SKILL.md 缺少 YAML frontmatter"))?;
    let end = rest
        .find("\n---")
        .ok_or_else(|| err("invalid_request", "SKILL.md frontmatter 未闭合"))?;
    let meta: serde_yaml_ng::Value = serde_yaml_ng::from_str(&rest[..end])
        .map_err(|_| err("invalid_request", "SKILL.md frontmatter 无效"))?;
    let name = meta
        .get("name")
        .and_then(|v| v.as_str())
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| err("invalid_request", "Skill 缺少 name"))?;
    let description = meta
        .get("description")
        .and_then(|v| v.as_str())
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| err("invalid_request", "Skill 缺少 description"))?;
    if name.len() > 128 || description.len() > 8192 {
        return Err(err("invalid_request", "Skill 名称或描述过长"));
    }
    Ok((name.into(), description.into(), raw))
}
pub fn candidate(
    path: &Path,
    origin: Option<SkillOrigin>,
    target_ids: Vec<String>,
) -> ExtResult<SkillCandidate> {
    let (name, description, content) = metadata(path)?;
    let directory_name = path
        .file_name()
        .and_then(|v| v.to_str())
        .ok_or_else(|| err("invalid_request", "Skill 目录名无效"))?
        .to_string();
    files::safe_name(&directory_name)?;
    let canonical = path
        .canonicalize()
        .map_err(|_| err("unavailable", "Skill 来源目录不可用"))?;
    let link_target = fs::read_link(path).ok().map(|p| p.display().to_string());
    Ok(SkillCandidate {
        id: uuid::Uuid::new_v4().to_string(),
        name,
        description,
        directory_name,
        path: path.display().to_string(),
        canonical_path: canonical.display().to_string(),
        link_target,
        content_hash: files::tree_hash(path)?,
        content,
        target_ids,
        origin,
    })
}
pub fn scan(root: &Path, origin: Option<SkillOrigin>) -> SkillDiscovery {
    let mut result = SkillDiscovery {
        candidates: vec![],
        errors: vec![],
        checked_at: chrono::Utc::now().to_rfc3339(),
    };
    if !root.is_dir() {
        result.errors.push(Diagnostic {
            target_id: root.display().to_string(),
            message: "来源目录不可用".into(),
        });
        return result;
    }
    let mut count = 0;
    for entry in WalkDir::new(root)
        .follow_links(false)
        .max_depth(16)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|e| {
            e.depth() == 0
                || !matches!(
                    e.file_name().to_str(),
                    Some(".git" | "node_modules" | "target" | "dist" | ".venv")
                )
        })
    {
        count += 1;
        if count > files::MAX_FILES {
            result.errors.push(Diagnostic {
                target_id: "source".into(),
                message: "扫描条目超过限制".into(),
            });
            break;
        }
        match entry {
            Ok(e) if e.file_name() == "SKILL.md" && e.file_type().is_file() => {
                let dir = e.path().parent().unwrap();
                let mut source = origin.clone();
                if let Some(o) = source.as_mut() {
                    o.relative_path = dir
                        .strip_prefix(root)
                        .unwrap_or(Path::new(""))
                        .to_string_lossy()
                        .replace('\\', "/");
                }
                match candidate(dir, source, vec![]) {
                    Ok(c) => result.candidates.push(c),
                    Err(e) => result.errors.push(Diagnostic {
                        target_id: dir.display().to_string(),
                        message: e.message,
                    }),
                }
            }
            Err(_) => result.errors.push(Diagnostic {
                target_id: "source".into(),
                message: "部分目录无法读取".into(),
            }),
            _ => {}
        }
    }
    result
}

#[derive(Clone)]
pub struct SkillFetcher {
    client: reqwest::Client,
    pub github_api: String,
    pub github_archive: String,
    pub search_api: String,
}
impl Default for SkillFetcher {
    fn default() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(60))
                .user_agent("dbox/0.1")
                .redirect(reqwest::redirect::Policy::limited(3))
                .build()
                .expect("valid HTTP client"),
            github_api: "https://api.github.com".into(),
            github_archive: "https://codeload.github.com".into(),
            search_api: "https://skills.sh/api/search".into(),
        }
    }
}
impl SkillFetcher {
    async fn download(&self, url: reqwest::Url, cancel: &CancellationToken) -> ExtResult<Vec<u8>> {
        let mut response = tokio::select! { _ = cancel.cancelled() => return Err(err("cancelled", "请求已取消")), r = self.client.get(url).send() => r.map_err(|_| err("unavailable", "网络请求失败"))? };
        if !response.status().is_success() {
            return Err(err(
                "unavailable",
                format!("来源返回 HTTP {}", response.status().as_u16()),
            ));
        }
        if response
            .content_length()
            .is_some_and(|n| n > files::MAX_BYTES)
        {
            return Err(err("invalid_request", "下载超过大小限制"));
        }
        let mut bytes = vec![];
        loop {
            let chunk = tokio::select! { _ = cancel.cancelled() => return Err(err("cancelled", "请求已取消")), r = response.chunk() => r.map_err(|_| err("unavailable", "下载中断"))? };
            let Some(chunk) = chunk else { break };
            if bytes.len() + chunk.len() > files::MAX_BYTES as usize {
                return Err(err("invalid_request", "下载超过大小限制"));
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }
    pub async fn prepare(
        &self,
        source: &SkillSource,
        staging: &Path,
        cancel: &CancellationToken,
    ) -> ExtResult<(PathBuf, Option<String>)> {
        validate_source(source)?;
        files::private_dir(staging)?;
        match source.kind {
            SourceKind::Local => {
                let root = Path::new(&source.uri)
                    .canonicalize()
                    .map_err(|_| err("unavailable", "本地来源不存在"))?;
                Ok((root, None))
            }
            SourceKind::Zip => {
                let bytes = files::read(Path::new(&source.uri))?;
                extract_zip(&bytes, staging, cancel)?;
                Ok((staging.to_owned(), None))
            }
            SourceKind::Github => {
                let repo = repository(&source.uri)?;
                let mut url =
                    reqwest::Url::parse(&format!("{}/repos/{repo}/commits/", self.github_api))
                        .map_err(|_| err("invalid_request", "来源 URL 无效"))?;
                url.path_segments_mut()
                    .map_err(|_| err("invalid_request", "来源 URL 无效"))?
                    .pop_if_empty()
                    .push(
                        source
                            .requested_ref
                            .as_deref()
                            .filter(|v| !v.is_empty())
                            .unwrap_or("HEAD"),
                    );
                let bytes = self.download(url, cancel).await?;
                let value: serde_json::Value = serde_json::from_slice(&bytes)
                    .map_err(|_| err("unavailable", "GitHub 响应格式无效"))?;
                let commit = value["sha"]
                    .as_str()
                    .filter(|s| s.len() == 40 && s.chars().all(|c| c.is_ascii_hexdigit()))
                    .ok_or_else(|| err("unavailable", "GitHub 未返回固定 commit"))?
                    .to_owned();
                let url =
                    reqwest::Url::parse(&format!("{}/{repo}/zip/{commit}", self.github_archive))
                        .map_err(|_| err("invalid_request", "来源 URL 无效"))?;
                let bytes = self.download(url, cancel).await?;
                extract_zip(&bytes, staging, cancel)?;
                let entries = fs::read_dir(staging)
                    .map_err(|_| err("unavailable", "暂存目录不可读"))?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|_| err("unavailable", "暂存目录不可读"))?;
                if entries.len() != 1 || !entries[0].path().is_dir() {
                    return Err(err("invalid_request", "GitHub 归档根目录无效"));
                }
                Ok((entries[0].path(), Some(commit)))
            }
        }
    }
    pub async fn search(
        &self,
        query: &str,
        cancel: &CancellationToken,
    ) -> ExtResult<Vec<SearchSkill>> {
        let mut url = reqwest::Url::parse(&self.search_api)
            .map_err(|_| err("invalid_request", "搜索 URL 无效"))?;
        url.query_pairs_mut()
            .append_pair("q", query)
            .append_pair("limit", "30");
        let bytes = self.download(url, cancel).await?;
        let value: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|_| err("unavailable", "搜索结果格式无效"))?;
        let list = value["skills"]
            .as_array()
            .ok_or_else(|| err("unavailable", "搜索结果格式无效"))?;
        Ok(list
            .iter()
            .filter_map(|v| {
                let repo = v["source"].as_str()?;
                repository(repo).ok()?;
                Some(SearchSkill {
                    name: v["name"].as_str()?.into(),
                    repository: repo.into(),
                    skill_id: v["id"].as_str().unwrap_or_default().into(),
                })
            })
            .collect())
    }
}
pub fn extract_zip(bytes: &[u8], destination: &Path, cancel: &CancellationToken) -> ExtResult<()> {
    // Bound central-directory allocation before the archive reader builds its index.
    let trailer = (bytes.len().saturating_sub(65_557)..bytes.len().saturating_sub(21))
        .rev()
        .find(|&i| {
            bytes.get(i..i + 4) == Some(b"PK\x05\x06")
                && i + 22 + u16::from_le_bytes([bytes[i + 20], bytes[i + 21]]) as usize
                    == bytes.len()
        })
        .ok_or_else(|| err("invalid_request", "ZIP 尾记录无效"))?;
    if u16::from_le_bytes([bytes[trailer + 10], bytes[trailer + 11]]) as usize > files::MAX_FILES {
        return Err(err("invalid_request", "ZIP 条目超过限制或使用 ZIP64"));
    }
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|_| err("invalid_request", "ZIP 格式无效"))?;
    // zip 2.x indexes entries by name and collapses exact duplicates. Compare the
    // validated classic EOCD count against that index before extracting anything.
    // ZIP64 is unnecessary within our 128 MiB / 10k entry limits and is rejected.
    let eocd = bytes
        .len()
        .checked_sub(22 + zip.comment().len())
        .ok_or_else(|| err("invalid_request", "ZIP 尾记录无效"))?;
    if zip.zip64_comment().is_some() || bytes.get(eocd..eocd + 4) != Some(b"PK\x05\x06") {
        return Err(err("invalid_request", "暂不支持 ZIP64 或非标准 ZIP 尾记录"));
    }
    let count = u16::from_le_bytes([bytes[eocd + 10], bytes[eocd + 11]]) as usize;
    if count != zip.len() || count == u16::MAX as usize {
        return Err(err("invalid_request", "ZIP 包含重复条目或不支持的条目数量"));
    }
    if zip.len() > files::MAX_FILES {
        return Err(err("invalid_request", "ZIP 条目超过限制"));
    }
    let mut paths = BTreeSet::new();
    let mut total: u64 = 0;
    for i in 0..zip.len() {
        if cancel.is_cancelled() {
            return Err(err("cancelled", "解压已取消"));
        }
        let file = zip
            .by_index(i)
            .map_err(|_| err("invalid_request", "ZIP 条目损坏"))?;
        let name = file.name().to_string();
        let path = Path::new(&name);
        files::safe_relative(path)?;
        if path.components().count() > 24
            || !paths.insert(name.trim_end_matches('/').to_lowercase())
        {
            return Err(err(
                "invalid_request",
                "ZIP 存在重复路径、大小写冲突或过深目录",
            ));
        }
        let mode = file.unix_mode().unwrap_or(0);
        let kind = mode & 0o170000;
        if !matches!(kind, 0 | 0o100000 | 0o040000) {
            return Err(err("invalid_request", "ZIP 不允许链接或特殊文件"));
        }
        total = total
            .checked_add(file.size())
            .ok_or_else(|| err("invalid_request", "ZIP 大小溢出"))?;
        if total > files::MAX_BYTES || file.size() > files::MAX_FILE {
            return Err(err("invalid_request", "ZIP 解压大小超过限制"));
        }
        let out = destination.join(path);
        if file.is_dir() {
            files::private_dir(&out)?;
            continue;
        }
        files::private_dir(out.parent().unwrap())?;
        let mut data = vec![];
        file.take(files::MAX_FILE + 1)
            .read_to_end(&mut data)
            .map_err(|_| err("invalid_request", "ZIP 解压校验失败"))?;
        if data.len() as u64 > files::MAX_FILE {
            return Err(err("invalid_request", "ZIP 文件超过限制"));
        }
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&out)
            .map_err(|_| err("invalid_request", "ZIP 路径冲突"))?;
        output
            .write_all(&data)
            .map_err(|_| err("failed", "无法写入解压文件"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            output
                .set_permissions(fs::Permissions::from_mode(0o600 | (mode & 0o111)))
                .map_err(|_| err("failed", "无法保留文件权限"))?;
        }
    }
    Ok(())
}
