//! Side-by-side macOS binary releases from ggml-org/llama.cpp.
use std::cmp::Reverse;
use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use specta::Type;
use tokio::io::AsyncWriteExt;

const RELEASES: &str = "https://api.github.com/repos/ggml-org/llama.cpp/releases";
const PAGE_SIZE: u32 = 20;
const MAX_METADATA: usize = 8 * 1024 * 1024;
const MAX_ARCHIVE: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LlamaInventory {
    pub root: String,
    pub versions: Vec<LlamaInstalledVersion>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LlamaInstalledVersion {
    pub tag: String,
    pub directory: String,
    pub server_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LlamaReleasePage {
    pub releases: Vec<LlamaRelease>,
    pub has_more: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LlamaRelease {
    pub tag: String,
    pub published_at: String,
    pub asset_name: Option<String>,
}

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
    published_at: Option<String>,
    draft: bool,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    size: u64,
    digest: Option<String>,
}

fn build_number(tag: &str) -> Option<u32> {
    let digits = tag.strip_prefix('b')?;
    if digits.len() > 10 || !digits.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

fn validate_tag(tag: &str) -> Result<(), String> {
    build_number(tag)
        .map(|_| ())
        .ok_or_else(|| "版本号应为 b 加构建编号，例如 b12644".into())
}

fn mac_arch() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Some("arm64"),
        ("macos", "x86_64") => Some("x64"),
        _ => None,
    }
}

impl GithubRelease {
    fn asset(&self, arch: &str) -> Option<&Asset> {
        let stem = format!("llama-{}-bin-macos-{arch}", self.tag_name);
        ["tar.gz", "zip"].into_iter().find_map(|extension| {
            let expected = format!("{stem}.{extension}");
            self.assets.iter().find(|asset| asset.name == expected)
        })
    }
}

fn client() -> Result<Client, String> {
    Client::builder()
        .user_agent(concat!("DBox/", env!("CARGO_PKG_VERSION")))
        .https_only(true)
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| e.to_string())
}

async fn metadata<T: serde::de::DeserializeOwned>(
    client: &Client,
    url: String,
) -> Result<(T, bool), String> {
    let mut response = client
        .get(url)
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| format!("获取 GitHub 发布信息失败：{e}"))?;
    let has_more = response
        .headers()
        .get(reqwest::header::LINK)
        .and_then(|link| link.to_str().ok())
        .is_some_and(|link| link.contains("rel=\"next\""));
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
        if bytes.len() + chunk.len() > MAX_METADATA {
            return Err("GitHub 发布信息过大".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let value =
        serde_json::from_slice(&bytes).map_err(|e| format!("GitHub 发布信息格式无效：{e}"))?;
    Ok((value, has_more))
}

pub async fn releases(page: u32) -> Result<LlamaReleasePage, String> {
    if page == 0 {
        return Err("页码必须大于零".into());
    }
    let (rows, has_more) = metadata::<Vec<GithubRelease>>(
        &client()?,
        format!("{RELEASES}?per_page={PAGE_SIZE}&page={page}"),
    )
    .await?;
    let releases = rows
        .into_iter()
        .filter(|row| !row.draft && build_number(&row.tag_name).is_some())
        .map(|row| LlamaRelease {
            asset_name: mac_arch()
                .and_then(|arch| row.asset(arch))
                .map(|a| a.name.clone()),
            tag: row.tag_name,
            published_at: row.published_at.unwrap_or_default(),
        })
        .collect();
    Ok(LlamaReleasePage { releases, has_more })
}

// Only real version directories and real executable files qualify. The unversioned
// binaries that Llama.app may keep alongside them belong to Llama.app.
fn real_dir(path: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => Ok(true),
        Ok(_) => Err(io::Error::other(format!(
            "预期为独立目录：{}",
            path.display()
        ))),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

fn server_in(dir: &Path) -> io::Result<Option<PathBuf>> {
    for parent in [dir.to_path_buf(), dir.join("bin")] {
        if !fs::symlink_metadata(&parent).is_ok_and(|meta| meta.is_dir()) {
            continue;
        }
        let path = parent.join("llama-server");
        match fs::symlink_metadata(&path) {
            Ok(meta) if meta.is_file() => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if meta.permissions().mode() & 0o111 == 0 {
                        continue;
                    }
                }
                return Ok(Some(path));
            }
            Err(e) if e.kind() != ErrorKind::NotFound => return Err(e),
            _ => {}
        }
    }
    Ok(None)
}

fn version(tag: &str, dir: &Path, server: &Path) -> LlamaInstalledVersion {
    LlamaInstalledVersion {
        tag: tag.to_owned(),
        directory: dir.display().to_string(),
        server_path: server.display().to_string(),
    }
}

fn inventory_at(root: &Path) -> io::Result<LlamaInventory> {
    let mut versions = Vec::new();
    if real_dir(root)? {
        for entry in fs::read_dir(root)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(tag) = name.to_str().and_then(|name| name.strip_prefix("llama-")) else {
                continue;
            };
            if build_number(tag).is_none() || !entry.file_type()?.is_dir() {
                continue;
            }
            let directory = entry.path();
            if let Some(server) = server_in(&directory)? {
                versions.push(version(tag, &directory, &server));
            }
        }
        versions.sort_by_key(|item| Reverse(build_number(&item.tag)));
    }
    Ok(LlamaInventory {
        root: root.display().to_string(),
        versions,
    })
}

async fn filesystem<T: Send + 'static>(
    operation: impl FnOnce() -> io::Result<T> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

pub async fn inventory(root: &Path) -> Result<LlamaInventory, String> {
    let root = root.to_owned();
    filesystem(move || inventory_at(&root)).await
}

fn existing(root: &Path, tag: &str) -> io::Result<Option<LlamaInstalledVersion>> {
    let dir = root.join(format!("llama-{tag}"));
    if !real_dir(&dir)? {
        return Ok(None);
    }
    let server = server_in(&dir)?.ok_or_else(|| {
        io::Error::other(format!(
            "目录 {} 中缺少可执行的 llama-server，请检查目录",
            dir.display()
        ))
    })?;
    Ok(Some(version(tag, &dir, &server)))
}

async fn download(client: &Client, tag: &str, asset: &Asset, archive: &Path) -> Result<(), String> {
    if asset.size == 0 || asset.size > MAX_ARCHIVE {
        return Err("发布包大小超出范围".into());
    }
    let url = format!(
        "https://github.com/ggml-org/llama.cpp/releases/download/{tag}/{}",
        asset.name
    );
    let mut response = client
        .get(url)
        .timeout(Duration::from_secs(600))
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| format!("下载 {tag} 失败：{e}"))?;
    let mut file = tokio::fs::File::create_new(archive)
        .await
        .map_err(|e| e.to_string())?;
    let mut digest = Sha256::new();
    let mut size = 0;
    while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
        size += chunk.len() as u64;
        if size > asset.size {
            return Err("下载内容大于发布包声明的大小".into());
        }
        digest.update(&chunk);
        file.write_all(&chunk).await.map_err(|e| e.to_string())?;
    }
    file.flush().await.map_err(|e| e.to_string())?;
    if size != asset.size {
        return Err("发布包下载不完整".into());
    }
    if let Some(expected) = &asset.digest {
        if expected != &format!("sha256:{:x}", digest.finalize()) {
            return Err("发布包 SHA-256 校验失败".into());
        }
    }
    Ok(())
}

// macOS ships bsdtar with support for both current tar.gz and earlier ZIP
// releases. Its default traversal and symlink protections remain enabled.
fn unpack(archive: &Path, destination: &Path, tag: &str) -> io::Result<PathBuf> {
    fs::create_dir(destination)?;
    let output = Command::new("/usr/bin/tar")
        .args(["--no-same-owner", "--no-same-permissions", "-xf"])
        .arg(archive)
        .arg("-C")
        .arg(destination)
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "解压失败：{}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    for dir in [
        destination.join(format!("llama-{tag}")),
        destination.to_owned(),
        destination.join("build"),
    ] {
        if !fs::symlink_metadata(&dir).is_ok_and(|meta| meta.is_dir()) || server_in(&dir)?.is_none()
        {
            continue;
        }
        let canonical = dir.canonicalize()?;
        for entry in walkdir::WalkDir::new(&dir) {
            let entry = entry?;
            let kind = entry.file_type();
            if kind.is_symlink() {
                let path = entry.path();
                if fs::read_link(path)?.is_absolute()
                    || !path.canonicalize()?.starts_with(&canonical)
                {
                    return Err(io::Error::other("发布包中存在指向版本目录外的链接"));
                }
            } else if !kind.is_file() && !kind.is_dir() {
                return Err(io::Error::other("发布包中存在不支持的文件类型"));
            }
        }
        return Ok(dir);
    }
    Err(io::Error::other("发布包中缺少可执行的 llama-server"))
}

// Take the filesystem lock only for the final check and rename. A failed
// download is invisible to scans, and independent versions can download concurrently.
fn lock_root(root: &Path) -> io::Result<fs::File> {
    let lock = fs::File::open(root)?;
    lock.lock()?;
    Ok(lock)
}

fn publish(root: &Path, tag: &str, unpacked: &Path) -> io::Result<LlamaInstalledVersion> {
    let _lock = lock_root(root)?;
    if let Some(installed) = existing(root, tag)? {
        return Ok(installed);
    }
    let dir = root.join(format!("llama-{tag}"));
    fs::rename(unpacked, &dir)?;
    existing(root, tag)?.ok_or_else(|| io::Error::other("安装目录已被移除"))
}

pub async fn install(root: &Path, tag: &str) -> Result<LlamaInstalledVersion, String> {
    validate_tag(tag)?;
    let arch = mac_arch().ok_or("目前支持下载 macOS Apple Silicon 和 Intel 版本")?;
    if real_dir(root).map_err(|e| e.to_string())? {
        if let Some(installed) = existing(root, tag).map_err(|e| e.to_string())? {
            return Ok(installed);
        }
    }
    let client = client()?;
    let (release, _) = metadata::<GithubRelease>(&client, format!("{RELEASES}/tags/{tag}")).await?;
    if release.draft || release.tag_name != tag {
        return Err("未找到指定的公开版本".into());
    }
    let asset = release
        .asset(arch)
        .ok_or_else(|| format!("{tag} 没有适用于 macOS {arch} 的发布包"))?;
    fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let staging = tempfile::Builder::new()
        .prefix(".dbox-download-")
        .tempdir_in(root)
        .map_err(|e| e.to_string())?;
    let archive = staging.path().join("archive");
    download(&client, tag, asset, &archive).await?;
    let root = root.to_owned();
    let tag = tag.to_owned();
    filesystem(move || {
        let dir = unpack(&archive, &staging.path().join("unpacked"), &tag)?;
        publish(&root, &tag, &dir)
    })
    .await
}

fn remove_at(root: &Path, tag: &str) -> io::Result<()> {
    if !real_dir(root)? {
        return Ok(());
    }
    let _lock = lock_root(root)?;
    if existing(root, tag)?.is_some() {
        let retired = tempfile::Builder::new()
            .prefix(".dbox-remove-")
            .tempdir_in(root)?;
        fs::rename(
            root.join(format!("llama-{tag}")),
            retired.path().join("version"),
        )?;
        drop(_lock);
        retired.close()?;
    }
    Ok(())
}

pub async fn remove(root: &Path, tag: &str) -> Result<(), String> {
    validate_tag(tag)?;
    let root = root.to_owned();
    let tag = tag.to_owned();
    filesystem(move || remove_at(&root, &tag)).await
}

#[cfg(all(unix, test))]
mod tests {
    use super::*;
    use std::os::unix::fs::{symlink, PermissionsExt};

    fn create_server(dir: &Path) {
        fs::create_dir_all(dir).unwrap();
        let server = dir.join("llama-server");
        fs::write(&server, b"release fixture").unwrap();
        fs::set_permissions(server, fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn archive(parent: &Path, name: &str, target: &Path) {
        assert!(Command::new("/usr/bin/tar")
            .arg("-czf")
            .arg(target)
            .arg("-C")
            .arg(parent)
            .arg(name)
            .status()
            .unwrap()
            .success());
    }

    #[tokio::test]
    async fn scans_existing_executables_and_removes_only_requested_build() {
        let home = tempfile::tempdir().unwrap();
        let root = home.path().join(".llama-app");
        assert!(inventory(&root).await.unwrap().versions.is_empty());
        assert!(!root.exists());
        create_server(&root);
        create_server(&root.join("llama-b9/bin"));
        create_server(&root.join("llama-b12644"));
        create_server(&root.join(".dbox-download-interrupted"));
        create_server(&root.join("models"));
        symlink(root.join("models"), root.join("llama-b8")).unwrap();

        let listed = inventory(&root).await.unwrap();
        let tags: Vec<_> = listed.versions.iter().map(|v| v.tag.as_str()).collect();
        assert_eq!(tags, ["b12644", "b9"]);
        assert_eq!(
            listed.versions[1].server_path,
            root.join("llama-b9/bin/llama-server").display().to_string()
        );
        assert!(remove(&root, "b8").await.is_err());
        assert!(remove(&root, "b9/../models").await.is_err());
        remove(&root, "b9").await.unwrap();
        remove(&root, "b9").await.unwrap();
        assert!(root.join("models/llama-server").exists());
        assert!(root.join("llama-server").exists());
        assert!(root.join("llama-b12644/llama-server").exists());
        assert!(!root.join("llama-b9").exists());
    }

    #[test]
    fn publishes_complete_build_with_library_links_and_preserves_other_content() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("installs");
        create_server(&root.join("llama-b2"));
        let source = temp.path().join("llama-b3");
        create_server(&source);
        fs::write(source.join("libggml.1.dylib"), "library").unwrap();
        symlink("libggml.1.dylib", source.join("libggml.dylib")).unwrap();
        let tar = temp.path().join("release.tgz");
        archive(temp.path(), "llama-b3", &tar);
        let extracted = unpack(&tar, &temp.path().join("staging"), "b3").unwrap();
        assert_eq!(inventory_at(&root).unwrap().versions.len(), 1);
        let installed = publish(&root, "b3", &extracted).unwrap();
        assert_eq!(
            fs::read(Path::new(&installed.directory).join("libggml.dylib")).unwrap(),
            b"library"
        );
        let duplicate = unpack(&tar, &temp.path().join("second"), "b3").unwrap();
        assert_eq!(
            publish(&root, "b3", &duplicate).unwrap().server_path,
            installed.server_path
        );
        assert!(duplicate.exists());
        fs::create_dir(root.join("llama-b4")).unwrap();
        assert!(publish(&root, "b4", &duplicate).is_err());
        assert!(root.join("llama-b4").read_dir().unwrap().next().is_none());
        assert_eq!(inventory_at(&root).unwrap().versions.len(), 2);
    }

    #[test]
    fn rejects_packages_with_external_library_links() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("llama-b2");
        create_server(&source);
        symlink("/usr/lib", source.join("lib")).unwrap();
        let tar = temp.path().join("release.tgz");
        archive(temp.path(), "llama-b2", &tar);
        assert!(unpack(&tar, &temp.path().join("out"), "b2").is_err());
    }

    #[test]
    fn selects_matching_macos_asset_for_tag_and_architecture() {
        let release: GithubRelease = serde_json::from_value(serde_json::json!({
            "tag_name": "b12644", "draft": false, "prerelease": true, "published_at": "2026-11-23",
            "assets": [
                { "name": "llama-b12643-bin-macos-arm64.tar.gz", "size": 2 },
                { "name": "llama-b12644-bin-macos-arm64-kleidiai.tar.gz", "size": 2 },
                { "name": "llama-b12644-bin-macos-x64.zip", "size": 2 },
                { "name": "llama-b12644-bin-macos-arm64.tar.gz", "size": 2 }
            ]
        }))
        .unwrap();
        assert_eq!(
            release.asset("arm64").unwrap().name,
            "llama-b12644-bin-macos-arm64.tar.gz"
        );
        assert_eq!(
            release.asset("x64").unwrap().name,
            "llama-b12644-bin-macos-x64.zip"
        );
        assert!(release.asset("other").is_none());
    }
}
