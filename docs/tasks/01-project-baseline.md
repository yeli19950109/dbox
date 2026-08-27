# T01：项目重命名与 Rust 测试基线

- 状态：Pending
- 阶段：Rust 基础
- 依赖：无
- 阻塞：T02–T26

## 目标

把当前 `tauri-test` 脚手架整理为可持续开发的 dbox 工程，并建立所有后续 Rust 任务共同使用的测试门禁。

## 交付内容

- 将产品名、窗口标题、bundle identifier、npm package、Rust package/lib 从 `tauri-test` 重命名为 dbox。
- 删除 `greet` command 和模板专用 Rust 代码；Vue 欢迎页可保留最小占位，但本任务不做正式 UI。
- 建立 `domain`、`providers`、`executor`、`persistence` 等 Rust 模块空骨架。
- 固定本地/CI 校验命令：
  - `cargo fmt --manifest-path src-tauri/Cargo.toml --check`
  - `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`
  - `cargo test --manifest-path src-tauri/Cargo.toml`
- 恢复合理 CSP，只保留当前必需的 Tauri capability。
- 在 Rust crate 中加入一个最小 smoke test，证明测试链路工作。

## 测试要求

- `cargo test` 至少运行一个真实测试，不接受“0 tests”作为基线完成。
- 搜索仓库，除迁移说明外不再出现 `tauri-test`、`greet` 或模板欢迎文案。
- `npm run build` 仍可通过，确保重命名没有破坏 Tauri/Vite 构建连接。

## 完成标准

三条 Rust 校验命令和 `npm run build` 全部通过；应用以 dbox 身份启动；没有开始实现正式图形界面。

## 非目标

- 不实现领域模型、Provider 或更新功能。
- 不进行视觉设计。

