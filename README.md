# dbox

dbox is a local manager for discovering, inspecting, and updating command-line tools.

Built-in providers: npm global, Homebrew, and mise. Providers are discovered from PATH;
an executable path override can be configured in Settings.

The mise provider lists every installed version and marks globally enabled versions.
Updates follow the selected global version request and preserve older installations.
Inactive versions, linked installations, and special requests such as `path:` remain
read-only. Custom target versions and project configuration updates are not supported.
See [mise provider details](docs/tasks/23-mise-provider.md).

## Validation

```sh
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
npm run build
```

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
