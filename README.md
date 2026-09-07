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
