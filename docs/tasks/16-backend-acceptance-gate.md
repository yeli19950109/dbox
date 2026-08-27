# T16：Rust 后端集成验收门

- 状态：Completed
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

## 完成标准

所有强制门禁命令通过，九类场景有自动化证据，`npm audit` 已审阅并记录决策，没有 P0/P1 后端缺陷。只有本任务标记 Completed 后，才允许开始 T17–T19 的正式 UI 对接。

## 非目标

- 不以手工点击界面代替后端自动化测试。
- 不要求真实执行系统包升级。

## 验收报告

- 完成日期：2026-08-27
- 后端 DTO schema revision：1
- 强制门禁：`cargo fmt --manifest-path src-tauri/Cargo.toml --check`、`cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`、`cargo test --manifest-path src-tauri/Cargo.toml`、`cargo deny --manifest-path src-tauri/Cargo.toml check`、`cargo audit --file src-tauri/Cargo.lock`、`npm run bindings:check`、`npm run build`、`npm audit` 全部通过。
- Rust 测试结果：共 108 个测试通过、0 失败。`npm audit` 报告 0 个漏洞。`cargo-deny` 的 advisory/license/bans/source 四项通过，Tauri 依赖图中的重复版本按 `warn` 策略保留；`cargo audit` 未发现阻塞漏洞并以成功状态结束。
- 场景 1：`npm_global_provider` fixture 覆盖全部 global top-level 项、未知包、scoped package、多 bin、缺失 bin、多 prefix、outdated/cache/offline 和精确单包计划。
- 场景 2：`homebrew_provider` fixture 覆盖 formula/cask、同名身份隔离、未知字段、多版本、pinned、outdated、权限/远端失败和精确单项计划。
- 场景 3：`catalog_pi_enrichment` 覆盖 Pi core/extensions 增强、普通未知 npm/brew Tool 保留、用户 override 与坏 manifest 隔离。
- 场景 4：`backend_acceptance` 通过 typed API 完成 snapshot → refresh → preview → confirm → execute → post-check → history/log，并验证状态与事件。
- 场景 5：`executor`、`run_queue_history` 和 `backend_acceptance` 覆盖命令失败、超时、running/queued 取消、进程树清理、批量 partial、失败后继续、retry 关联和重启 interrupted 恢复。为避免全套并行执行时受调度延迟影响，PID fixture 的超时握手窗口已放宽并重新全量验证。
- 场景 6：`environment` fixture 覆盖 GUI PATH 修复失败、用户 override、缓存、PATH 候选、Provider 路径优先级和 executable fingerprint revision，全程只使用临时 fake binary。
- 场景 7：`application_service` 覆盖 settings revision、environment/program fingerprint 和 version snapshot 改变后拒绝旧计划。
- 场景 8：所有 Provider/执行器/队列/验收链路均注入临时目录 fake executable；测试未调用真实 npm/brew/npx，也未修改真实全局安装。
- 场景 9：依赖选择、许可证、MSRV、Git revision、替代方案和生成契约记录在 ADR 0001/0002；`deny.toml` 固定审计策略。RustSec 维护状态例外的影响、到期日和 T20 跟踪动作记录在 ADR 0003；审查未发现自研原子替换、shell quoting、hash、PID 树/process group、取消原语、PATH 搜索、shell profile 解析或 TypeScript 镜像 DTO。
- 尚存风险：完整 lockfile 审计仍显示 Linux-only GTK3/GLib 上游告警；Linux 不在当前 MVP target graph，macOS/Windows 的 `cargo-deny` 图不包含它们。`paste` 与五个 `unic-*` 仅有停止维护公告，已按 ADR 0003 接受到 2026-11-27，并要求 T20 复核。当前主机完成 macOS 运行时验收，Windows 实机打包/运行验证留给 T20；这些风险不阻塞 T17–T19。
- 缺陷结论：未发现 P0/P1 后端缺陷，允许开始正式 UI 对接。
