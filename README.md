# DBox

DBox is a local manager for discovering, inspecting, and updating command-line tools.

Built-in providers: npm global, Homebrew, mise, Pi Extensions, and GitHub Releases. Command-line providers are discovered from PATH;
an executable path override can be configured in Settings.

The mise provider lists every installed version and marks globally enabled versions.
Updates follow the selected global version request and preserve older installations.
Inactive versions, linked installations, and special requests such as `path:` remain
read-only. Custom target versions and project configuration updates are not supported.
See [mise provider details](docs/tasks/23-mise-provider.md).

Pi Extensions discovers the personal npm packages declared in
`~/.pi/agent/settings.json` (or `PI_CODING_AGENT_DIR`) and installed under
`npm/node_modules` in that agent directory. Each package appears as a separate tool
with its own installed version, available version and update action. Checks use the
configured npm version range or dist-tag; exact versions stay pinned. Updating one
package runs `pi update --extension <source> --no-approve`, preserving Pi's package
management and excluding project settings. Pi Core keeps its own update strategies.
Git/local packages, project packages, legacy npm-global installations and custom Pi
`npmCommand` wrappers are outside this provider's scope; lookup failures remain visible
instead of being reported as up to date. Legacy global packages remain visible through
the npm-global provider.

GitHub Releases checks explicitly configured macOS applications without launching or
updating them. Each `providers.d/*.toml` file declares an application path, expected
bundle identifier, and public GitHub repository. dbox reads the application's
`CFBundleShortVersionString` and compares it with GitHub's latest stable release.
Missing or invalid versions and network failures are reported; these applications
never receive a Provider update action or enter one-click updates.

For DashBeam, copy [the example configuration](examples/providers/dashbeam.toml) to
`~/Library/Application Support/com.yel.dbox/providers.d/dashbeam.toml`, then refresh
the **GitHub Releases** source. Adjust `app_path` if installed elsewhere. This is a
Provider configuration file, separate from the Catalog editor's `tools.d` manifests.
Checks use the public GitHub API without a token; API rate limits are shown as check
failures. No update commands, downloads of release assets, or application changes are
performed.

## Codex usage

The **用量** page reads Codex session logs under `$CODEX_HOME` (default `~/.codex`) and
shows exact per-request token usage by model and by day. Model names are normalized
(`openai/GPT-5.4-2026-03-05` → `gpt-5.4`), sub-agent replays of parent history and
duplicate archived copies are not counted twice, and compressed `*.jsonl.zst` sessions
are read too. Counted requests are kept in `usage/codex-requests.jsonl` under dbox's data
directory, so only new or changed session files are parsed and usage survives deleted
session logs. Nothing is written to the Codex directory. Costs are not computed. See
[ADR 0005](docs/adr/0005-codex-usage-statistics.md).

## Validation

```sh
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
npm run build
```

## Application version

Use the root `package.json` version as the source of truth. After editing it, run:

```sh
npm run version:sync
npm run version:check
```

The sync command updates `package-lock.json` (both root version fields),
`src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml`, and the application entry in
`src-tauri/Cargo.lock`. Dependency versions are preserved. The check command exits
with a nonzero status if any version is out of sync, without modifying files.

Tauri has no application version bump command in its [official CLI](https://v2.tauri.app/reference/cli/).
Its [version config](https://v2.tauri.app/reference/config/#version) can reference
`package.json`, but that does not synchronize the Cargo manifest or lockfiles.

## Local macOS package

Build a DMG for the current Mac architecture with Tauri's bundler:

```sh
npm run tauri -- build --bundles dmg --ci --config '{"bundle":{"macOS":{"signingIdentity":"-"}}}'
```

The DMG is written to `src-tauri/target/release/bundle/dmg/`. This local build uses
ad-hoc signing; it has no Developer ID signature or Apple notarization. Public
distribution requires separate Apple signing and notarization credentials.

Development binaries are excluded from default builds. Use `npm run bindings:generate`
to enable the `bindings` feature for the TypeScript exporter, or `npm run dev:http`
to enable the `dev-http` feature for the browser development bridge.

## Skill 与 MCP 管理

支持 Claude Code、Codex、Gemini CLI 的用户级 Skill 安装/导入/启停/更新/备份恢复，以及 MCP 配置导入、编辑、分应用启停和同步。所有写操作先预览确认，结果进入统一运行记录。

使用方式和范围见 [管理说明](docs/skills-mcp-management.md)，实现决策见 [ADR 0004](docs/adr/0004-native-extensions.md)。
