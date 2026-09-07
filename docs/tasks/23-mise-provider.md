# T23：mise Provider

- 状态：Completed
- 阶段：后续 Provider
- 依赖：T03、T04、T07–T09、T13、T16
- 阻塞：无

## 目标

通过 mise 的公开 CLI 管理全局 scope 工具及多版本信息，不把 mise 简化为 npm/brew 单版本包。

## 交付内容

- probe mise 与 capability/version。
- 扫描全局安装和激活版本，保留 backend、版本、scope 与安装路径。
- 表达多版本共存和当前激活版本。
- 生成精确更新 CommandPlan，不影响未选择工具或项目级配置。
- 同一产品由 mise/npm/brew 安装时分开显示。

## 测试要求

- fake mise 覆盖多版本、未激活、全局/项目混合、插件缺失和离线。
- 测试不得修改真实 mise 配置。

## 完成标准

fixture scan/check/plan 通过，且不修改通用领域/API。

## 实现说明

- 内置注册 `mise`，沿用 GUI PATH 解析和 `executableOverrides.mise`，设置页自动展示 Provider。
- 通过 `mise --cd <home> version`、`upgrade --help` 探测版本及 `--no-prune` 能力。
- 合并 `ls --installed --json` 与 `ls --global --installed --json`，通过 `tool --json` 获取完整 backend 坐标。
- 每个安装版本独立展示。全局配置项使用 `Global` scope，其他共享安装使用 `User` scope；前端标记“全局启用”或“未全局启用”。项目或环境激活不视为全局启用。
- 全局安装 ID 根据工具、backend、数据根、配置来源及请求范围生成，不随解析版本变化；非全局安装 ID 包含具体版本。更新后旧版本继续保留，post-check 能找到原全局配置项。
- 使用 `latest <backend>@<request>` 检查配置范围内的更新；限制查询并发、缓存成功结果，离线、超时、插件缺失和无效输出显示检查失败。
- 更新计划为 `mise --cd <home> upgrade --no-prune --yes -- <tool>@<request>`。计划生成前重新核对全局/当前配置、版本路径和 backend，拒绝配置覆盖或过期选择。仅生成计划，执行仍经过现有预览/确认流程。
- 无领域/API/TypeScript bindings 修改，无新增依赖。

CLI 语义参考：[ls](https://mise.jdx.dev/cli/ls.html)、[tool](https://mise.jdx.dev/cli/tool.html)、[latest](https://mise.jdx.dev/cli/latest.html)、[upgrade](https://mise.jdx.dev/cli/upgrade.html)。

## 验证记录

- 完成日期：2026-09-07。
- 关键文件：`src-tauri/src/providers/mise.rs`、`src-tauri/src/api/service.rs`、`src-tauri/tests/mise_provider.rs`、`src-tauri/tests/fixtures/fake-mise.sh`、`src/components/ToolCard.vue`、`src/views/ToolsView.vue`。
- Rust 全量回归（含 `dev-http`）通过；最终 mise fixture 10 项通过。覆盖多版本、多全局范围、未激活、项目/环境来源、跨 Provider/数据根身份、更新后身份、配置变更、缺失插件、离线、缓存、超时、无效 JSON/路径及生产注册。
- `cargo fmt --manifest-path src-tauri/Cargo.toml --check`、`cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --features dev-http -- -D warnings`、`npm test`（38 项）、`npm run build` 通过。
- 本机 mise 2026.9.1 只读扫描识别 22 个安装版本、7 个全局启用项，7 项更新计划预览成功；未执行真实工具更新。真实远端版本查询受当前网络/缓存写入权限限制，失败状态正确返回。
- 已知限制：未全局启用的版本、链接安装、`path:`/`ref:` 等特殊请求仅展示；暂不支持自定义目标版本、跨版本范围升级、项目配置管理及 executable 枚举。缺少 `upgrade --no-prune` 的 mise 可扫描，更新保持只读。不同非 SemVer 版本不猜测更新顺序。
