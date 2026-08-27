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
9. T07–T09 的 ADR、许可证/MSRV 记录完整；代码审查确认没有自研 shell quoting、hash、PID 树、process group、取消原语、PATH 搜索或 shell profile 解析。

## 门禁命令

```text
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
npm run build
```

## 验收报告

在本文件完成时追加：

- 实际运行的命令和结果；
- 测试模块/场景清单；
- 尚未覆盖的风险及是否阻塞 UI；
- 后端 DTO schema revision。

## 完成标准

所有门禁命令通过，八类场景有自动化证据，没有 P0/P1 后端缺陷。只有本任务标记 Completed 后，才允许开始 T17–T19 的正式 UI 对接。

## 非目标

- 不以手工点击界面代替后端自动化测试。
- 不要求真实执行系统包升级。
