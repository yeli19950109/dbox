use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dbox_lib::domain::{ComponentId, StrategyId};
use dbox_lib::providers::{GitHubReleasesProvider, ProviderUpdateRequest, ToolProvider};
use dbox_lib::version::ComponentStatus;
use serde_json::json;
use tempfile::TempDir;

struct Fixture {
    temporary: TempDir,
    config_dir: PathBuf,
    app: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temporary = TempDir::new().unwrap();
        let config_dir = temporary.path().join("providers.d");
        let app = temporary.path().join("Example.app");
        fs::create_dir_all(&config_dir).unwrap();
        let fixture = Self {
            temporary,
            config_dir,
            app,
        };
        fixture.config("example", "Example", "owner/repo");
        fixture.plist(Some("1.0.0"), "com.example.app", false);
        fixture
    }

    fn config(&self, id: &str, name: &str, repository: &str) {
        let value = json!({
            "schema_version": 1,
            "id": id,
            "display_name": name,
            "app_path": self.app.to_str().unwrap(),
            "bundle_id": "com.example.app",
            "repository": repository,
        });
        fs::write(
            self.config_dir.join("example.toml"),
            toml::to_string(&value).unwrap(),
        )
        .unwrap();
    }

    fn plist(&self, version: Option<&str>, bundle_id: &str, binary: bool) {
        fs::create_dir_all(self.app.join("Contents")).unwrap();
        let mut values = plist::Dictionary::new();
        values.insert(
            "CFBundleIdentifier".into(),
            plist::Value::String(bundle_id.into()),
        );
        if let Some(version) = version {
            values.insert(
                "CFBundleShortVersionString".into(),
                plist::Value::String(version.into()),
            );
        }
        let value = plist::Value::Dictionary(values);
        let path = self.app.join("Contents/Info.plist");
        if binary {
            value.to_file_binary(path).unwrap()
        } else {
            value.to_file_xml(path).unwrap()
        }
    }

    fn provider(&self, server: &Server) -> GitHubReleasesProvider {
        self.provider_timeout(server, Duration::from_secs(3))
    }

    fn provider_timeout(&self, server: &Server, timeout: Duration) -> GitHubReleasesProvider {
        GitHubReleasesProvider::with_test_endpoint(self.config_dir.clone(), &server.url, timeout)
            .unwrap()
    }

    fn options(&self, options: serde_json::Value) {
        let path = self.config_dir.join("example.toml");
        let mut config: toml::Value = toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        for (key, value) in options.as_object().unwrap() {
            config
                .as_table_mut()
                .unwrap()
                .insert(key.clone(), toml::Value::try_from(value).unwrap());
        }
        fs::write(path, toml::to_string(&config).unwrap()).unwrap();
    }

    #[cfg(unix)]
    fn command(&self, body: &str, stream: &str, pattern: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let executable = self.temporary.path().join("bin with spaces/llama");
        fs::create_dir_all(executable.parent().unwrap()).unwrap();
        fs::write(
            &executable,
            format!("#!/bin/sh\n[ \"$#\" = 1 ] && [ \"$1\" = --version ] || exit 80\n{body}\n"),
        )
        .unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
        let config = json!({
            "schema_version": 1,
            "id": "llama-cpp",
            "display_name": "llama.cpp",
            "repository": "ggml-org/llama.cpp",
            "command": {
                "executable": executable,
                "stream": stream,
                "pattern": pattern,
            },
        });
        fs::write(
            self.config_dir.join("example.toml"),
            toml::to_string(&config).unwrap(),
        )
        .unwrap();
        executable
    }
}

struct Server {
    url: String,
    requests: Arc<Mutex<Vec<String>>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Server {
    fn new(responses: Vec<(u16, &'static str, String)>, delay: Duration) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = requests.clone();
        let thread = std::thread::spawn(move || {
            for (status, extra_headers, body) in responses {
                let deadline = std::time::Instant::now() + Duration::from_secs(5);
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            if std::time::Instant::now() >= deadline {
                                return;
                            }
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(error) => panic!("{error}"),
                    }
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut bytes = Vec::new();
                let mut chunk = [0; 1024];
                while !bytes.windows(4).any(|part| part == b"\r\n\r\n") {
                    let count = stream.read(&mut chunk).unwrap();
                    if count == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&chunk[..count]);
                }
                captured
                    .lock()
                    .unwrap()
                    .push(String::from_utf8(bytes).unwrap());
                std::thread::sleep(delay);
                let response = format!("HTTP/1.1 {status} Response\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{extra_headers}\r\n{body}", body.len());
                let _ = stream.write_all(response.as_bytes());
            }
        });
        Self {
            url,
            requests,
            thread: Some(thread),
        }
    }

    fn releases(tags: &[&str]) -> Self {
        Self::new(
            tags.iter()
                .map(|tag| {
                    (
                        200,
                        "",
                        json!({"tag_name": tag, "draft": false, "prerelease": false}).to_string(),
                    )
                })
                .collect(),
            Duration::ZERO,
        )
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}

#[tokio::test]
async fn detects_updates_and_current_or_newer_versions_without_running_or_modifying_app() {
    let fixture = Fixture::new();
    let server = Server::releases(&["v1.1.0", "1.0.0", "v0.9.0"]);
    let provider = fixture.provider(&server);
    assert!(provider.probe().await.unwrap().available);
    assert!(provider.capabilities().check_updates);
    assert!(!provider.capabilities().update);
    assert!(!provider.capabilities().executables);
    let before = fs::read(fixture.app.join("Contents/Info.plist")).unwrap();
    let mut progress = Vec::new();
    let apps = provider
        .scan_with_progress(&mut |event| progress.push(event))
        .await
        .unwrap();
    assert_eq!(apps.len(), 1);
    assert_eq!(apps[0].package.name, "Example");
    assert_eq!(apps[0].installed_version.as_ref().unwrap().raw(), "1.0.0");
    assert!(apps[0].executables.is_empty());
    assert!(progress
        .iter()
        .any(|event| event.package_name.as_deref() == Some("Example")));
    for expected in [
        ComponentStatus::UpdateAvailable,
        ComponentStatus::UpToDate,
        ComponentStatus::UpToDate,
    ] {
        let mut progress = Vec::new();
        let checks = provider
            .check_updates_with_progress(&apps, &mut |event| progress.push(event))
            .await
            .unwrap();
        assert_eq!(checks[0].status, expected);
        assert_eq!(progress.last().unwrap().completed, Some(1));
        assert_eq!(progress.last().unwrap().total, Some(1));
        assert_eq!(
            progress.last().unwrap().package_name.as_deref(),
            Some("Example")
        );
    }
    let error = provider
        .plan_update(ProviderUpdateRequest {
            installation_id: apps[0].id.clone(),
            component_id: ComponentId::new("core").unwrap(),
            strategy_id: StrategyId::new("provider-default").unwrap(),
            target_version: None,
        })
        .await
        .unwrap_err();
    assert!(error.summary.contains("only checks versions"));
    assert_eq!(
        before,
        fs::read(fixture.app.join("Contents/Info.plist")).unwrap()
    );
    assert_eq!(
        fs::read_dir(fixture.app.join("Contents")).unwrap().count(),
        1
    );
    assert_eq!(server.requests.lock().unwrap().len(), 3);
    for request in server.requests.lock().unwrap().iter() {
        assert!(request.starts_with("GET /repos/owner/repo/releases/latest HTTP/1.1\r\n"));
        assert!(request.to_ascii_lowercase().contains("user-agent: dbox/"));
    }
}

#[tokio::test]
async fn supports_binary_plist_and_stable_ids_independent_of_display_name() {
    let fixture = Fixture::new();
    fixture.plist(Some("1.2.3"), "com.example.app", true);
    let provider = GitHubReleasesProvider::new(fixture.config_dir.clone());
    let original = provider.scan().await.unwrap();
    fixture.config("example", "Renamed", "owner/repo");
    let renamed = provider.scan().await.unwrap();
    assert_eq!(original[0].id, renamed[0].id);
    assert_eq!(
        renamed[0].installed_version.as_ref().unwrap().raw(),
        "1.2.3"
    );
    fixture.config("different", "Renamed", "owner/repo");
    assert_ne!(renamed[0].id, provider.scan().await.unwrap()[0].id);
}

#[tokio::test]
async fn python_prereleases_preserve_display_strings_and_compare_numeric_suffixes() {
    let fixture = Fixture::new();
    fixture.options(json!({"version_format": "python_prerelease", "allow_prerelease": true}));
    fixture.plist(Some("0.7.1rc2"), "com.example.app", false);
    let server = Server::releases(&["v0.7.1rc2", "v0.7.1rc10", "v0.7.1", "v0.7.0"]);
    let provider = fixture.provider(&server);
    let apps = provider.scan().await.unwrap();
    assert_eq!(
        apps[0].installed_version.as_ref().unwrap().raw(),
        "0.7.1rc2"
    );
    for (expected_version, expected_status) in [
        ("0.7.1rc2", ComponentStatus::UpToDate),
        ("0.7.1rc10", ComponentStatus::UpdateAvailable),
        ("0.7.1", ComponentStatus::UpdateAvailable),
        ("0.7.0", ComponentStatus::UpToDate),
    ] {
        let checks = provider.check_updates(&apps).await.unwrap();
        assert_eq!(
            checks[0].latest_version.as_ref().unwrap().raw(),
            expected_version
        );
        assert_eq!(checks[0].status, expected_status);
    }
}

#[tokio::test]
async fn python_format_and_remote_prereleases_require_explicit_options() {
    let fixture = Fixture::new();
    fixture.plist(Some("0.7.1rc1"), "com.example.app", false);
    let provider = GitHubReleasesProvider::new(fixture.config_dir.clone());
    let apps = provider.scan().await.unwrap();
    assert_failure(
        &provider.check_updates(&apps).await.unwrap()[0].status,
        "app_version_invalid",
    );

    fixture.options(json!({"version_format": "python_prerelease"}));
    let server = Server::releases(&["v0.7.1rc1"]);
    let provider = fixture.provider(&server);
    let apps = provider.scan().await.unwrap();
    assert_failure(
        &provider.check_updates(&apps).await.unwrap()[0].status,
        "github_release_unstable",
    );

    fixture.options(json!({"allow_prerelease": true}));
    let server = Server::new(
        vec![(
            200,
            "",
            json!({"tag_name":"v0.7.1rc1","draft":true,"prerelease":true}).to_string(),
        )],
        Duration::ZERO,
    );
    let provider = fixture.provider(&server);
    let apps = provider.scan().await.unwrap();
    assert_failure(
        &provider.check_updates(&apps).await.unwrap()[0].status,
        "github_release_unstable",
    );
}

#[cfg(unix)]
#[tokio::test]
async fn explicitly_configured_cli_checks_version_with_a_single_fixed_argument() {
    let fixture = Fixture::new();
    for stream in ["stdout", "stderr"] {
        let redirect = if stream == "stderr" { ">&2" } else { "" };
        let executable = fixture.command(
            &format!("printf 'version: 0.7.0-dev (build 11706, commit abc123)\\n' {redirect}"),
            stream,
            r"(?m)^version: ([0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?) ",
        );
        let before = fs::read(&executable).unwrap();
        let server = Server::releases(&["v0.7.0"]);
        let provider = fixture.provider(&server);
        let apps = provider.scan().await.unwrap();
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].install_path.as_ref(), Some(&executable));
        assert_eq!(
            apps[0].installed_version.as_ref().unwrap().raw(),
            "0.7.0-dev"
        );
        assert_eq!(
            provider.check_updates(&apps).await.unwrap()[0].status,
            ComponentStatus::UpdateAvailable
        );
        assert!(!provider.capabilities().update);
        assert_eq!(fs::read(executable).unwrap(), before);
        assert!(server.requests.lock().unwrap()[0]
            .starts_with("GET /repos/ggml-org/llama.cpp/releases/latest "));
    }
}

#[cfg(unix)]
#[tokio::test]
async fn cli_failures_remain_visible_and_do_not_query_github() {
    let fixture = Fixture::new();
    let server = Server::releases(&[]);
    let provider = fixture.provider_timeout(&server, Duration::from_millis(100));
    for (body, pattern, code) in [
        ("exit 1", r"version: (\S+)", "command_failed"),
        ("exec sleep 5", r"version: (\S+)", "command_timeout"),
        (
            "printf 'no version'",
            r"version: (\S+)",
            "command_version_missing",
        ),
        (
            "printf 'version: nonsense'",
            r"version: (\S+)",
            "command_version_invalid",
        ),
        (
            "printf 'version: 1.0.0\\nversion: 2.0.0'",
            r"version: (\S+)",
            "command_version_ambiguous",
        ),
        (
            "head -c 1048577 /dev/zero",
            r"version: (\S+)",
            "command_failed",
        ),
    ] {
        fixture.command(body, "stdout", pattern);
        let apps = provider.scan().await.unwrap();
        assert_eq!(apps.len(), 1);
        assert!(apps[0].installed_version.is_none());
        assert_failure(
            &provider.check_updates(&apps).await.unwrap()[0].status,
            code,
        );
    }
    let executable = fixture.command("printf 'version: 1.0.0'", "stdout", r"version: (\S+)");
    fs::remove_file(executable).unwrap();
    assert!(provider.scan().await.unwrap().is_empty());
    assert!(server.requests.lock().unwrap().is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn cli_configs_reject_ambiguous_sources_relative_paths_and_extra_arguments() {
    let fixture = Fixture::new();
    let provider = GitHubReleasesProvider::new(fixture.config_dir.clone());
    for pattern in ["[", "version", "(one)(two)"] {
        fixture.command("exit 0", "stdout", pattern);
        assert!(provider
            .scan()
            .await
            .unwrap_err()
            .summary
            .contains("pattern"));
    }
    fixture.command("exit 0", "stdout", "(version)");
    fixture
        .options(json!({"app_path": "/Applications/Example.app", "bundle_id": "com.example.app"}));
    assert!(provider
        .scan()
        .await
        .unwrap_err()
        .summary
        .contains("exclusively"));
    fixture.command("exit 0", "stdout", "(version)");
    fixture.options(json!({"command": {"executable": "llama", "pattern": "(version)"}}));
    assert!(provider
        .scan()
        .await
        .unwrap_err()
        .summary
        .contains("absolute"));
    fixture.options(json!({"command": {"executable": "/bin/sh", "pattern": "(version)", "args": ["-c", "exit 0"]}}));
    assert!(provider.scan().await.is_err());
}

#[tokio::test]
async fn absent_app_or_config_directory_does_not_create_installations() {
    let fixture = Fixture::new();
    fs::remove_dir_all(&fixture.app).unwrap();
    assert!(GitHubReleasesProvider::new(fixture.config_dir.clone())
        .scan()
        .await
        .unwrap()
        .is_empty());
    assert!(
        GitHubReleasesProvider::new(fixture.temporary.path().join("absent"))
            .scan()
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn invalid_local_metadata_never_claims_up_to_date_or_queries_github() {
    let fixture = Fixture::new();
    let provider = GitHubReleasesProvider::new(fixture.config_dir.clone());
    for (version, bundle, code) in [
        (Some("1.0.0"), "com.wrong.app", "app_bundle_mismatch"),
        (
            Some("build 1.0.0"),
            "com.example.app",
            "app_version_invalid",
        ),
        (Some("1.0"), "com.example.app", "app_version_invalid"),
        (None, "com.example.app", "app_version_missing"),
    ] {
        fixture.plist(version, bundle, false);
        let apps = provider.scan().await.unwrap();
        assert_eq!(apps.len(), 1);
        assert!(apps[0].installed_version.is_none());
        assert_failure(
            &provider.check_updates(&apps).await.unwrap()[0].status,
            code,
        );
    }
    fs::write(fixture.app.join("Contents/Info.plist"), b"invalid plist").unwrap();
    let apps = provider.scan().await.unwrap();
    assert_failure(
        &provider.check_updates(&apps).await.unwrap()[0].status,
        "app_metadata_invalid",
    );
}

#[tokio::test]
async fn invalid_and_duplicate_configurations_are_visible_provider_errors() {
    let fixture = Fixture::new();
    let provider = GitHubReleasesProvider::new(fixture.config_dir.clone());
    for repository in [
        "https://github.com/owner/repo",
        "../repo",
        "owner/repo?token=secret",
        "owner/repo/extra",
    ] {
        fixture.config("example", "Example", repository);
        assert!(provider
            .probe()
            .await
            .unwrap_err()
            .summary
            .contains("repository"));
    }
    fixture.config("example", "Example", "owner/repo");
    fs::copy(
        fixture.config_dir.join("example.toml"),
        fixture.config_dir.join("duplicate.toml"),
    )
    .unwrap();
    assert!(provider
        .scan()
        .await
        .unwrap_err()
        .summary
        .contains("Duplicate"));
    fs::remove_file(fixture.config_dir.join("duplicate.toml")).unwrap();
    fs::write(fixture.config_dir.join("example.toml"), "bad = \"secret\"").unwrap();
    let error = provider.scan().await.unwrap_err();
    assert!(error.summary.contains("example.toml"));
    assert!(!error.summary.contains("secret"));
}

#[tokio::test]
async fn github_errors_and_nonstable_tags_never_claim_a_successful_check() {
    let fixture = Fixture::new();
    let cases = vec![
        (429, "", "{}".into(), "github_rate_limited"),
        (
            403,
            "X-RateLimit-Remaining: 0\r\n",
            "{}".into(),
            "github_rate_limited",
        ),
        (404, "", "{}".into(), "github_release_missing"),
        (500, "", "{}".into(), "github_http_failed"),
        (200, "", "invalid json".into(), "github_metadata_invalid"),
        (
            200,
            "",
            json!({"tag_name":"v2.0.0","draft":true,"prerelease":false}).to_string(),
            "github_release_unstable",
        ),
        (
            200,
            "",
            json!({"tag_name":"v2.0.0","draft":false,"prerelease":true}).to_string(),
            "github_release_unstable",
        ),
        (
            200,
            "",
            json!({"tag_name":"v2.0.0-beta.1","draft":false,"prerelease":false}).to_string(),
            "github_release_unstable",
        ),
        (
            200,
            "",
            json!({"tag_name":"release-2.0.0","draft":false,"prerelease":false}).to_string(),
            "github_version_invalid",
        ),
    ];
    let server = Server::new(
        cases
            .iter()
            .map(|(status, headers, body, _)| (*status, *headers, body.clone()))
            .collect(),
        Duration::ZERO,
    );
    let provider = fixture.provider(&server);
    let apps = provider.scan().await.unwrap();
    for (_, _, _, code) in cases {
        let checks = provider.check_updates(&apps).await.unwrap();
        assert_failure(&checks[0].status, code);
        assert!(checks[0].latest_version.is_none());
    }
}

#[tokio::test]
async fn timeout_and_connection_failure_are_retryable_failures() {
    let fixture = Fixture::new();
    let server = Server::new(vec![(200, "", "{}".into())], Duration::from_millis(200));
    let provider = fixture.provider_timeout(&server, Duration::from_millis(50));
    let apps = provider.scan().await.unwrap();
    assert_failure(
        &provider.check_updates(&apps).await.unwrap()[0].status,
        "github_timeout",
    );
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/", listener.local_addr().unwrap());
    drop(listener);
    let provider = GitHubReleasesProvider::with_test_endpoint(
        fixture.config_dir.clone(),
        &endpoint,
        Duration::from_secs(1),
    )
    .unwrap();
    let apps = provider.scan().await.unwrap();
    assert_failure(
        &provider.check_updates(&apps).await.unwrap()[0].status,
        "github_network_failed",
    );
}

#[test]
fn fixture_endpoint_cannot_be_used_for_remote_sources() {
    for endpoint in [
        "http://github.com/",
        "https://127.0.0.1/",
        "http://127.0.0.1/path",
        "http://user@127.0.0.1/",
    ] {
        assert!(GitHubReleasesProvider::with_test_endpoint(
            PathBuf::new(),
            endpoint,
            Duration::from_secs(1)
        )
        .is_err());
    }
}

fn assert_failure(status: &ComponentStatus, code: &str) {
    let ComponentStatus::CheckFailed { reason } = status else {
        panic!("expected check failure, got {status:?}");
    };
    assert_eq!(reason.code, code);
}
