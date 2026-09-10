# dbox 任务索引

本目录把 [开发计划](../development-plan.md) 拆成可独立实现、测试和验收的任务。每个任务只对应一个文件；实现完成时在该文件中更新状态并记录测试证据。

Skill/MCP 的最新选型、代码对照及阶段依赖见 [Skill 与 MCP 管理接入计划](../skills-mcp-integration-plan.md)。该计划替代原先的 Skill CLI 薄适配要求；T21/T22 和 MCP 管理现已实现，使用方式见 [管理说明](../skills-mcp-management.md)。

## 执行原则

1. **Rust 后端优先。** T01–T16 完成前，不开始正式图形界面 T17–T19。
2. **测试属于任务本身。** 每个 Rust 任务必须同时提交对应单元/fixture/集成测试，不创建“以后统一补测试”的尾部任务。
3. **不操作真实全局包。** 自动化测试只能使用临时目录、FakeProvider 和 fake binaries，禁止调用真实 `brew upgrade`、`npm install -g`、`rustup update` 或 `npx skills`。
4. **一项一验收。** 依赖任务未完成时，不通过临时硬编码绕过它。
5. **后端门禁。** T16 是前端开工条件；Rust 格式化、Clippy、测试和集成场景全部通过后才解锁 UI。
6. **Provider 优先于白名单。** npm/brew 的全部全局安装项均由 Provider 自动生成 Tool；catalog 只做特殊增强。
7. **Skill 管理遵循最新能力对照。** 参考 cc-switch 实现原生内容与部署管理，已有安装须显式接管；不与外部管理器双写状态，外部 lock 仅用于兼容导入读取。Skill/MCP 元数据直接使用 JSON 文件，按少量本地资源设计，不使用数据库。下载、解压和解析继续优先使用开源基础库。
8. **基础设施不自研。** 命令执行、进程组、取消、PATH 修复和 executable 查找优先采用维护中的开源库；dbox 只保留领域编排和薄适配。自定义替代必须先用 ADR 证明现有库确实不满足。
9. **前后端契约单一来源。** Rust API DTO 生成 TypeScript command/event bindings，禁止手工维护镜像 interface；生成不能替代 wire-format golden test。
10. **前端基础设施不自研。** 路由、全局状态、测试 runner、虚拟列表和通用节流使用任务指定的维护库；dbox 只实现产品状态与交互规则。
11. **依赖治理属于门禁。** 新依赖固定版本/feature 并记录许可证、MSRV 和平台兼容性；T16 执行许可证与 RustSec 检查。

## 状态值

- `Pending`：依赖满足后可开始。
- `In Progress`：正在实现，同一时间尽量只保留一个核心任务。
- `Blocked`：依赖或已记录问题阻塞。
- `Completed`：交付物、测试和完成标准全部满足，并已记录验证证据。
- `Future`：MVP 后候选任务。

## Rust 后端主线

| ID | 任务 | 依赖 | 主要验收 |
| --- | --- | --- | --- |
| T01 | [项目重命名与 Rust 测试基线](01-project-baseline.md) | 无 | fmt/clippy/test/build 基线通过 |
| T02 | [核心领域模型](02-domain-model.md) | T01 | 来源限定 ID、多 executable、serde 测试 |
| T03 | [Provider 接口与注册表](03-provider-registry.md) | T02 | FakeProvider 与 capability 测试 |
| T04 | [版本比较与状态汇总](04-version-and-status.md) | T02 | SemVer/unknown/partial 纯单测 |
| T05 | [可选 Manifest 与 Catalog 合并](05-manifest-catalog.md) | T02、T04 | 无 manifest 仍可管理，覆盖合并测试 |
| T06 | [设置、缓存与运行记录持久化](06-persistence.md) | T01、T02 | 原子写入库、临时目录、revision 测试 |
| T07 | [基于开源库的命令规格与 UpdatePlan](07-command-plan.md) | T02、T04 | argv 隔离、hash、过期计划测试 |
| T08 | [基于开源库的命令执行适配层](08-command-executor.md) | T06、T07 | fake process 集成测试 |
| T09 | [基于开源库的 GUI 环境与命令解析](09-environment-resolver.md) | T03、T07 | fake PATH/候选路径测试 |
| T10 | [npm Global Provider](10-npm-global-provider.md) | T03、T04、T07–T09 | 全部 global package fixture |
| T11 | [Homebrew Provider](11-homebrew-provider.md) | T03、T04、T07–T09 | 全部 formula/cask fixture |
| T12 | [Catalog 增强与 Pi 示例](12-catalog-enrichment-pi.md) | T05、T10、T11 | 通用 Tool + 多组件增强测试 |
| T13 | [刷新、检查与更新应用服务](13-application-service.md) | T03–T12 | 纯 Rust 完整用例测试 |
| T14 | [批量队列、运行历史与恢复](14-run-queue-history.md) | T06、T08、T13 | 20 项、部分失败、中断恢复测试 |
| T15 | [Tauri 后端 API 与事件边界](15-tauri-backend-api.md) | T09、T13、T14 | 生成 bindings + DTO/command/event 契约测试 |
| T16 | [Rust 后端集成验收门](16-backend-acceptance-gate.md) | T01–T15 | 后端、依赖治理与端到端 fixture 门禁通过 |

## 图形界面与发布

以下任务只有在 T16 标记 `Completed` 后才能开始：

| ID | 任务 | 依赖 | 主要验收 |
| --- | --- | --- | --- |
| T17 | [前端壳层与类型化 API](17-frontend-foundation.md) | T16 | 标准路由/store/test + 生成 bindings 对接 |
| T18 | [工具列表、详情与刷新界面](18-tools-ui.md) | T17 | 全量 Tool 状态展示 |
| T19 | [更新确认、运行记录与设置界面](19-update-runs-settings-ui.md) | T14、T17、T18 | UpdatePlan 驱动完整 UI 流程 |
| T20 | [MVP 打包、文档与发布验收](20-mvp-release.md) | T16、T18、T19 | macOS 包和验收报告 |

## 扩展管理与后续能力

| ID | 任务 | 依赖 | 说明 |
| --- | --- | --- | --- |
| T21 | [原生 Skill 管理后端](21-skills-cli-backend.md) | T08、T09、T15、T16、接入计划 P0/P1 | 已完成：内容库、来源、部署、导入、更新、备份恢复 |
| T22 | [Skill 管理界面](22-skills-ui.md) | T17、T21 | 已完成：已安装/发现、应用启停、导入、更新与恢复 |
| T23 | [mise Provider](23-mise-provider.md) | Backend Gate | 多版本与全局 scope |
| T24 | [rustup Provider](24-rustup-provider.md) | Backend Gate | toolchain/component/target 层级 |
| T25 | [Cargo Install Provider](25-cargo-install-provider.md) | Backend Gate | 用户级 binary crates |
| T26 | [Direct/Self-update Provider](26-direct-provider.md) | Backend Gate | Deno/Bun/Flutter 等明确来源 |

| T27 | [MCP 管理与共享运行基础](27-mcp-management.md) | T15、T17、接入计划 P0/P1 | 已完成：三应用配置、导入、启停、备份恢复、脱敏与运行记录 |

## 推荐执行顺序

主线顺序：

```text
T01 → T02 → T03/T04/T06 → T05/T07 → T08/T09
    → T10/T11 → T12 → T13 → T14 → T15 → T16
    → T17 → T18 → T19 → T20
```

斜杠只表示依赖允许并行，不要求并行开发。T21/T22/T27 已按本次集成需求完成；其余扩展按对应任务状态推进。

Skill/MCP 共享基础、MCP 后端/界面和集成验收先按 [接入计划 P0–P7](../skills-mcp-integration-plan.md#9-分阶段任务与验收) 执行；实际交付记录见 T21、T22、T27 和 ADR 0004。

## 每项完成时的记录格式

在任务文件末尾追加：

```text
## 验证记录

- 完成日期：YYYY-MM-DD
- 关键文件：
- 执行命令：
- 测试结果：
- 已知限制：
```
