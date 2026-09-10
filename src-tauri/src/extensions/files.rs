//! Bounded, non-executing filesystem operations. Never follows a nested source symlink.
use super::{err, ExtResult};
use serde::{de::DeserializeOwned, Serialize};
use std::{
    collections::BTreeSet,
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};
use walkdir::WalkDir;
pub const MAX_FILES: usize = 10_000;
pub const MAX_BYTES: u64 = 128 * 1024 * 1024;
pub const MAX_FILE: u64 = 16 * 1024 * 1024;

pub fn hash(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}
pub fn exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}
pub fn read(path: &Path) -> ExtResult<Vec<u8>> {
    let mut f =
        fs::File::open(path).map_err(|_| err("unavailable", "无法读取文件；请检查路径和权限"))?;
    let mut bytes = vec![];
    Read::by_ref(&mut f)
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| err("unavailable", "文件读取失败"))?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(err("invalid_request", "文件超过 128 MiB 限制"));
    }
    Ok(bytes)
}
pub fn revision(path: &Path) -> ExtResult<String> {
    if !exists(path) {
        return Ok("missing".into());
    }
    Ok(hash(&read(path)?))
}
pub fn load<T: DeserializeOwned + Default>(path: &Path) -> ExtResult<(String, T)> {
    if !exists(path) {
        return Ok(("missing".into(), T::default()));
    }
    let bytes = read(path)?;
    let value: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|_| err("corrupt", "管理文件不是有效 JSON；原文件已保留"))?;
    if value.get("schemaVersion").and_then(|v| v.as_u64()) != Some(1) {
        return Err(err("corrupt", "不支持此管理文件版本"));
    }
    let doc = serde_json::from_value(value)
        .map_err(|_| err("corrupt", "管理文件结构损坏；原文件已保留"))?;
    Ok((hash(&bytes), doc))
}
pub fn save<T: Serialize>(path: &Path, expected: &str, doc: &T) -> ExtResult<String> {
    if revision(path)? != expected {
        return Err(err("conflict", "文件已被外部修改，请刷新后重试"));
    }
    let bytes = serde_json::to_vec_pretty(doc).map_err(|_| err("failed", "无法序列化管理记录"))?;
    if hash(&bytes) != expected {
        atomic_write(path, &bytes)?;
    }
    Ok(hash(&bytes))
}
pub fn private_dir(path: &Path) -> ExtResult<()> {
    fs::create_dir_all(path).map_err(|_| err("unavailable", "无法创建目录"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|_| err("failed", "无法设置目录权限"))?;
    }
    Ok(())
}
pub fn atomic_write(path: &Path, bytes: &[u8]) -> ExtResult<()> {
    let parent = path
        .parent()
        .ok_or_else(|| err("invalid_request", "文件缺少父目录"))?;
    if !parent.exists() {
        private_dir(parent)?;
    }
    let mut f = atomic_write_file::AtomicWriteFile::open(path)
        .map_err(|_| err("failed", "无法准备原子写入"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        f.as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|_| err("failed", "无法设置文件权限"))?;
    }
    f.write_all(bytes)
        .map_err(|_| err("failed", "文件写入失败"))?;
    f.commit().map_err(|_| err("failed", "文件替换失败"))?;
    if read(path)? != bytes {
        return Err(err("conflict", "写入后文件又发生变化"));
    }
    Ok(())
}
pub fn safe_name(name: &str) -> ExtResult<()> {
    if name.is_empty()
        || name.len() > 128
        || name.starts_with('.')
        || !name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        return Err(err(
            "invalid_request",
            "目录名/配置 key 只能使用字母、数字、短横线和下划线（最多 128 字符）",
        ));
    }
    Ok(())
}
pub fn safe_relative(path: &Path) -> ExtResult<()> {
    if path.is_absolute()
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
        || path.to_string_lossy().contains(['\\', ':'])
    {
        return Err(err("invalid_request", "拒绝不安全的相对路径"));
    }
    Ok(())
}
// Resolve ancestor aliases, leaving the final entry intact so removing a link never removes its target.
pub fn physical(path: &Path) -> ExtResult<PathBuf> {
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(err("invalid_request", "需要不含 .. 的绝对路径"));
    }
    let parent = path
        .parent()
        .ok_or_else(|| err("invalid_request", "不能管理文件系统根目录"))?;
    let name = path
        .file_name()
        .ok_or_else(|| err("invalid_request", "路径缺少名称"))?;
    Ok(resolve_parent(parent)?.join(name))
}
fn resolve_parent(path: &Path) -> ExtResult<PathBuf> {
    if exists(path) {
        return path
            .canonicalize()
            .map_err(|_| err("conflict", "父目录存在断链或无权限"));
    }
    let parent = path
        .parent()
        .ok_or_else(|| err("invalid_request", "无有效父目录"))?;
    Ok(resolve_parent(parent)?.join(path.file_name().unwrap()))
}
pub fn tree_hash(path: &Path) -> ExtResult<String> {
    let root = path
        .canonicalize()
        .map_err(|_| err("unavailable", "Skill 目录不存在或链接失效"))?;
    if !root.is_dir() {
        return Err(err("invalid_request", "Skill 来源必须是目录"));
    }
    let mut hasher = blake3::Hasher::new();
    for file in tree_files(&root)? {
        let relative = file.strip_prefix(&root).unwrap().to_string_lossy();
        let bytes = read(&file)?;
        hasher.update(&(relative.len() as u64).to_le_bytes());
        hasher.update(relative.as_bytes());
        hasher.update(&executable(&file).to_le_bytes());
        hasher.update(&(bytes.len() as u64).to_le_bytes());
        hasher.update(&bytes);
    }
    Ok(hasher.finalize().to_hex().to_string())
}
pub fn tree_files(root: &Path) -> ExtResult<Vec<PathBuf>> {
    let mut files = vec![];
    let mut total = 0;
    let mut names = BTreeSet::new();
    let mut count = 0;
    for entry in WalkDir::new(root).follow_links(false).sort_by_file_name() {
        let e = entry.map_err(|_| err("unavailable", "目录遍历失败"))?;
        count += 1;
        if count > MAX_FILES || e.depth() > 24 {
            return Err(err("invalid_request", "目录文件数或深度超过限制"));
        }
        if e.depth() == 0 {
            continue;
        }
        let rel = e
            .path()
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .to_lowercase();
        if !names.insert(rel) {
            return Err(err("conflict", "目录存在大小写冲突"));
        }
        if e.file_type().is_dir() {
            continue;
        }
        if !e.file_type().is_file() {
            return Err(err(
                "invalid_request",
                "Skill 内容中不支持符号链接或特殊文件",
            ));
        }
        let len = e
            .metadata()
            .map_err(|_| err("unavailable", "无法读取文件属性"))?
            .len();
        total += len;
        if len > MAX_FILE || total > MAX_BYTES {
            return Err(err("invalid_request", "Skill 大小超过限制"));
        }
        files.push(e.into_path());
    }
    Ok(files)
}
fn executable(path: &Path) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path)
            .map(|m| m.permissions().mode() & 0o111)
            .unwrap_or(0)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        0
    }
}
pub fn copy_tree(source: &Path, destination: &Path) -> ExtResult<()> {
    let root = source
        .canonicalize()
        .map_err(|_| err("unavailable", "来源目录不可用"))?;
    let dest = physical(destination)?;
    if dest.starts_with(&root) || root.starts_with(&dest) {
        return Err(err("conflict", "拒绝复制到自身或重叠目录"));
    }
    let files = tree_files(&root)?;
    private_dir(destination)?;
    for file in files {
        let out = destination.join(file.strip_prefix(&root).unwrap());
        private_dir(out.parent().unwrap())?;
        fs::copy(&file, &out).map_err(|_| err("failed", "复制 Skill 文件失败"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&out, fs::Permissions::from_mode(0o600 | executable(&file)))
                .map_err(|_| err("failed", "无法保留执行权限"))?;
        }
    }
    Ok(())
}
pub fn fingerprint(path: &Path) -> ExtResult<String> {
    if !exists(path) {
        return Ok("missing".into());
    }
    let meta = fs::symlink_metadata(path).map_err(|_| err("unavailable", "路径不可读"))?;
    if meta.file_type().is_symlink() {
        return Ok(format!(
            "link:{}",
            fs::read_link(path)
                .map_err(|_| err("unavailable", "无法读取链接"))?
                .display()
        ));
    }
    if meta.is_dir() {
        return Ok(format!("dir:{}", tree_hash(path)?));
    }
    if meta.is_file() {
        return Ok(format!("file:{}", hash(&read(path)?)));
    }
    Err(err("invalid_request", "拒绝操作特殊文件"))
}
pub fn remove(path: &Path) -> ExtResult<()> {
    if !exists(path) {
        return Ok(());
    }
    let m = fs::symlink_metadata(path).map_err(|_| err("unavailable", "路径不可读"))?;
    if m.is_dir() && !m.file_type().is_symlink() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
    .map_err(|_| err("failed", "无法移除目标"))
}
pub fn symlink(source: &Path, target: &Path) -> ExtResult<()> {
    #[cfg(unix)]
    let result = std::os::unix::fs::symlink(source, target);
    #[cfg(windows)]
    let result = std::os::windows::fs::symlink_dir(source, target);
    result.map_err(|_| err("failed", "无法创建符号链接"))
}
