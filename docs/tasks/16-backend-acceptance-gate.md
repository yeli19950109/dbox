# T16：Rust 后端集成验收门

- 状态：Pending
- 阶段：Rust Gate
- 依赖：T01–T15
- 阻塞：T17–T22

## 目标

在开始正式图形界面前，以可重复的测试证明 dbox 基本功能已经在 Rust 端闭环。

## 必须覆盖的端到端场景

1. Fake npm 扫描全部 global package，包括未知包、scoped package 和多 executable。
2. Fake brew 扫描 formula/cask，包括 unknown catalog 项和 outdated 状态。
3. catalog 增强 Pi core/extensions，但不影响普通 Tool。
4. snapshot → refresh → preview → confirm → execute → post-check → history 完整链路。
5. 命令失败、超时、取消、部分批量失败、应用中断恢复。
6. GUI PATH 缺失时通过用户路径/候选路径解析 fake executable。
7. 配置 revision 或 program 路径改变后旧计划被拒绝。
8. 测试期间不调用真实 npm/brew/npx，不修改任何真实全局安装。
9. T06–T09、T15 的 ADR、许可证/MSRV 记录完整；代码审查确认没有自研原子替换、shell quoting、hash、PID 树、process group、取消原语、PATH 搜索、shell profile 解析或 TypeScript 镜像 DTO。

## 依赖治理门禁

- `cargo deny check` 使用仓库固定配置检查许可证、来源、禁用依赖、重复依赖策略和 advisory policy。
- `cargo audit` 检查 RustSec；任何例外必须写明 advisory、影响分析、到期日和跟踪任务，不能只在命令行忽略。
- Git dependency 必须固定 revision，并在 ADR 记录许可证、更新流程和不可替代原因。
- 复核直接依赖只启用必要 Cargo features；新增依赖必须有许可证、维护状态、MSRV 和 macOS/Windows 兼容记录。
- `npm audit` 在 MVP 阶段生成报告但不作为无例外强门禁；T20 前依据可修复性记录阻塞策略，避免无上游修复的 advisory 永久阻塞。

## 门禁命令

```text
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
cargo deny --manifest-path src-tauri/Cargo.toml check
cargo audit --file src-tauri/Cargo.lock
npm run build
npm audit
```

## 验收报告

在本文件完成时追加：

- 实际运行的命令和结果；
- 测试模块/场景清单；
- 尚未覆盖的风险及是否阻塞 UI；
- 后端 DTO schema revision。

## 完成标准

所有强制门禁命令通过，九类场景有自动化证据，`npm audit` 已审阅并记录决策，没有 P0/P1 后端缺陷。只有本任务标记 Completed 后，才允许开始 T17–T19 的正式 UI 对接。

## 非目标

- 不以手工点击界面代替后端自动化测试。
- 不要求真实执行系统包升级。
