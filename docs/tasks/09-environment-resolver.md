# T09：基于开源库的 GUI 环境与命令解析

- 状态：Pending
- 阶段：Rust Provider 基础
- 依赖：T03、T07
- 阻塞：T10、T11、T15、T21、T23–T26

## 目标

通过 Tauri 官方开源环境修复库和成熟的 executable 查找 crate 解决 GUI PATH 问题。dbox 只负责用户覆盖优先级、缓存与诊断 DTO，不自行解析 shell profile、重写 `which` 或维护平台目录规则。

## 开源库优先方案

| 能力 | 优先实现 |
| --- | --- |
| GUI PATH 修复 | Tauri 官方 [`fix-path-env-rs`](https://github.com/tauri-apps/fix-path-env-rs)，固定 tag/revision |
| executable 定位和全部候选 | [`which`](https://docs.rs/which/latest/which/) 的 `which_in`/`which_in_all` |
| 应用 config/data/log 目录 | Tauri `PathResolver`；非 Tauri 场景才评估 `etcetera` |
| Homebrew prefix | 调用已定位的 `brew --prefix`，由 Homebrew Provider 解析 |

新增依赖前记录许可证、维护状态、MSRV 和失败行为。`fix-path-env-rs` 只在应用启动的受控入口调用；测试通过 adapter/fake 隔离，不能读取开发机真实 shell profile。

## 交付内容

- 为 `fix-path-env-rs` 和 `which` 定义极薄 adapter，便于测试替换，不复制其逻辑。
- 启动早期调用 `fix_path_env::fix()`，之后使用 `which` 在修复后的 PATH 中解析全部候选。
- 选择顺序：用户绝对路径、已验证缓存、`which` 返回候选、Provider 自身报告路径；不手写 shell profile 读取器。
- 真正命令仍以解析后的绝对 program + argv 直接执行。
- 记录命令来源、验证时间和不可用原因。
- 提供环境诊断 DTO：Node/npm/npx/brew 路径、版本、PATH 条目和冲突项。
- 路径变化或 executable 不再存在时使相关 CommandPlan 失效。

## 单元测试

- 使用注入 PATH 和临时 fake binaries 验证优先级。
- `which` 返回同名多个 executable 时保持确定顺序并显示冲突。
- 缓存路径失效后回退重新解析。
- `fix-path-env-rs` adapter 失败不阻断用户绝对路径和现有应用 PATH。
- 测试不读取真实 shell profile。

## 完成标准

在完全隔离的临时环境中可以通过 fake adapter 稳定解析 npm、brew、npx 并生成诊断快照；全部测试通过。代码审查确认没有手写 PATH 分割、executable 权限判断、shell profile 解析或平台配置目录实现。

## 非目标

- 不修改用户 shell 配置。
- 不自行实现 `which`、shell environment loader 或跨平台 app-dir 库。
