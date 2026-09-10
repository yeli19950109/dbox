# dbox Skill 与 MCP 管理接入计划

- 日期：2026-09-10
- 状态：Implemented（用户级首版）；原始比较与设计保留，实际实现、验证与边界见第 12 节。
- 参考：本地 `reference-only/cc-switch`，其 `package.json` 标记版本为 `3.20.2`。
- CLI 对照：本地 `reference-only/skills`，其 `package.json` 标记版本为 `1.5.25`。最终结论和下文链接均以本地源码为准；没有执行安装、更新、删除命令，也未验证本机安装版本。
- 本文更新 [原开发计划第 15 节](development-plan.md#15-skill-与-mcp-管理集成)、T21、T22 的技术方向。用户已明确允许根据能力缺口放弃原先的 `npx skills` 强制约束。

## 1. 选型结论

**采用 Rust 原生 Skill 管理，参考 cc-switch 的统一技能库、应用分发、已有安装导入和备份恢复；MCP 同样采用 Rust 服务与应用配置适配器。Vue 前端沿用 dbox 的路由、Pinia、生成类型和运行记录体系。**

**存储按个人本地少量资源设计，直接使用 JSON 文件，不使用 SQLite 等数据库。** 首版不建设索引、查询层或通用存储引擎；保留原子写入、外部修改检测和简单备份，满足日常管理即可。

`skills` CLI 能承担基础安装器的角色，支持的来源和 Agent 范围也更广。但是，要达到本计划定义的桌面管理体验，至少需要补齐四类核心能力：保留技能库的停用/恢复、独立更新检查、可核对来源冲突的安装接管、卸载前备份与恢复。仓库来源管理和写操作的结构化结果还需要额外实现。继续以 CLI 为唯一后端，会让这些功能依赖第二套文件和状态逻辑，难以维持“薄适配”。

因此不安排 `npx skills` 后端作为主线，也不设计两个后端同时管理同一批目录。CLI 仍可作为互操作对照；其已有安装通过显式导入接入 dbox。网络下载、归档解析、配置编辑等基础设施继续优先使用开源库，原生管理主要负责领域规则与操作编排。

首个完整版本聚焦 **Claude Code、Codex、Gemini CLI 的用户级管理**。项目级 Skill/MCP 和其他 Agent 列入后续阶段，必须在界面中准确标明支持范围，不能把项目级资源当作用户级资源处理。

## 2. `skills` CLI 与 cc-switch 能力对照

这里比较的是“公开 CLI 能否完成同等管理流程”，而不只是命令名称是否存在。原生方案也有开发成本，不把 cc-switch 的全部功能当作现成可直接移植的模块。

| 能力 | `reference-only/skills` 实际情况 | cc-switch 本地实现 | 对 dbox 的判断 |
| --- | --- | --- | --- |
| 安装、指定 Skill、指定 Agent | 支持 `add`、`--skill`、`--agent`、`--yes` | 仓库发现后安装到统一库，再分发给应用 | CLI 已覆盖基础操作，不计为缺口。[CLI 入口](../reference-only/skills/src/cli.ts) |
| 用户级/项目级、多种来源 | 支持 global/project、GitHub/GitLab/Git URL、本地目录等 | 当前 Skill 服务以用户级目录及 GitHub 仓库为主 | CLI 覆盖更广；原生首版主动限制范围，不能宣称完全替代 CLI。[README](../reference-only/skills/README.md) |
| 列表、Agent 筛选、JSON | `list --json` 已存在；输出名称、路径、scope、agents、来源，agents 是显示名称；实现默认 project，`-g` 为 global | InstalledSkill 返回描述、来源、时间、hash 和应用状态 | 可以包装；描述、部署冲突等数据仍需补充。旧计划“可能没有 JSON”的假设应更新。[list.ts](../reference-only/skills/src/list.ts) |
| 发现已有本地 Skill | 列表会扫描公共目录及 Agent 目录，不能说 CLI 完全发现不了手动安装 | `scan_unmanaged` 加显式导入界面 | 缺口在管理接管、来源选择和冲突预览，不在基本扫描。[installer.ts](../reference-only/skills/src/installer.ts) |
| 多 Agent 独立停用 | `remove --agent` 能移除指定 Agent 的安装；不提供独立的 enabled 状态 | `toggle_app` 分发/移除应用副本，统一库保留 | 部分覆盖；不能将“移除后重新安装”直接标为启停。[remove.ts](../reference-only/skills/src/remove.ts) |
| 全部停用后保留技能库 | remove 在判断无其他 Agent 使用时删除 canonical 目录及 lock 条目 | 全部 apps 为 false 时仍可保留统一库和记录 | 核心差异：停用后的离线重新启用需要稳定保留本地内容。[remove.ts](../reference-only/skills/src/remove.ts) |
| 只检查更新，不安装 | `check`、`update`、`upgrade` 分支均调用 `runUpdate`；后者执行实际更新 | `check_updates` 与 `update_skill` 分开；检查可能回填自身 hash 元数据 | 核心缺口；不得沿用旧计划把 `skills check` 作为只读检查。[cli.ts](../reference-only/skills/src/cli.ts)、[update.ts](../reference-only/skills/src/update.ts) |
| 单项/批量更新 | 支持指定名称、多个名称、global/project 更新 | 单项更新与逐项更新全部 | CLI 已覆盖；dbox 仍需管理来源丢失、本地修改、远端匹配失败等状态。[update.ts](../reference-only/skills/src/update.ts) |
| 搜索与仓库来源管理 | `find` 支持关键词及 owner；未发现维护仓库列表的公开命令 | skills.sh 搜索、仓库增删、branch、enabled、按仓库发现 | 搜索已有；持久来源列表可由 dbox 补充，单独这一项不足以否定 CLI。[find.ts](../reference-only/skills/src/find.ts)、[命令表](../reference-only/skills/src/cli.ts) |
| 来源预览/搜索的机器接口 | `add --list` 输出终端文本；find 没有公开 JSON 选项；JSON 列表不能代表全部命令都有 JSON | 发现、搜索、安装分别返回类型化对象 | 是桌面集成成本，不能通过导入 CLI 私有 TS 模块获得稳定契约。[add.ts](../reference-only/skills/src/add.ts)、[find.ts](../reference-only/skills/src/find.ts) |
| ZIP/归档安装 | 已支持远程归档 URL；本地路径走目录发现，未见本地 ZIP 的独立安装流程 | 文件选择后 `install_skills_from_zip` | 部分覆盖；不能写成“CLI 不支持 ZIP”。本地 ZIP 是原生增强项。[source-parser.ts](../reference-only/skills/src/source-parser.ts)、[add.ts](../reference-only/skills/src/add.ts)、[download-source.ts](../reference-only/skills/src/download-source.ts) |
| 卸载备份、恢复 | 未见备份管理命令；`experimental_install` 是按项目 lock 重新安装，不是恢复卸载前的文件内容 | 卸载备份、备份列表、删除、恢复 | 核心缺口，尤其涉及本地编辑和离线 Skill。[install.ts](../reference-only/skills/src/install.ts) |
| 同名不同来源/不同内容 | 已安装列表以 scope 与 name 去重，会合并 Agent 信息 | 未管理列表以目录名聚合，导入时选择首个来源 | 两者都需改善；dbox 必须显示冲突，不能把同名当作同内容。[installer.ts](../reference-only/skills/src/installer.ts) |
| Copy/Symlink | 支持 `--copy` 和默认链接；安装器包含链接失败回退 | Auto/Symlink/Copy | 两者都有，原生管理要记录实际部署方式。[installer.ts](../reference-only/skills/src/installer.ts) |
| 运行时依赖 | 本地 package.json 要求 Node `>=22.20.0` | 主要由 Rust 文件/网络服务执行 Skill 管理 | 原生方案减少管理功能对 Node/npx 的依赖，但自身需承担下载与平台维护成本。[package.json](../reference-only/skills/package.json) |

对照中的否定项限于本次本地快照的公开入口和实现；不代表以后版本不会提供，也不表示 CLI 内部没有可借鉴的算法。`init`、`use`、`experimental_install`、`experimental_sync` 等创建、临时使用、项目锁恢复和 node_modules 同步能力也已存在，但不属于本次桌面管理的核心范围，不作为原生首版必须复制的功能。[命令路由](../reference-only/skills/src/cli.ts)

阅读时发现本地 [AGENTS.md](../reference-only/skills/AGENTS.md) 的部分功能说明与实际路由不完全一致，例如仍分别描述 check/update；本计划以实现为依据。`cli.ts` 第 391–394 行把三种命令都分派到 `runUpdate`，`update.ts` 中实际调用当前 CLI 的 add 流程；没有独立只读 check 入口。README 对 list 默认 scope 的描述也应以 `list.ts::runList` 为准。

### CLI 关键代码位置

| 文件 | 阅读入口与结论 |
| --- | --- |
| [src/cli.ts](../reference-only/skills/src/cli.ts) | `main` 命令路由；check/update 语义；公开操作边界 |
| [src/list.ts](../reference-only/skills/src/list.ts)、[src/installer.ts](../reference-only/skills/src/installer.ts) | `runList` JSON 输出；`listInstalledSkills` 扫描范围及 scope:name 去重；安装与链接回退 |
| [src/remove.ts](../reference-only/skills/src/remove.ts) | `removeCommand` 先移除目标部署，再判断是否删除 canonical/lock |
| [src/update.ts](../reference-only/skills/src/update.ts) | `runUpdate`、`updateGlobalSkills`、`updateProjectSkills`；比较来源后实际重装 |
| [src/add.ts](../reference-only/skills/src/add.ts)、[src/source-parser.ts](../reference-only/skills/src/source-parser.ts)、[src/download-source.ts](../reference-only/skills/src/download-source.ts) | 本地目录与远程归档分别处理，来源预览为终端文本 |
| [src/find.ts](../reference-only/skills/src/find.ts)、[src/install.ts](../reference-only/skills/src/install.ts) | 关键词/owner 搜索；按 lock 重装，不是文件备份恢复 |
| [tests/remove-canonical.test.ts](../reference-only/skills/tests/remove-canonical.test.ts)、[src/list.test.ts](../reference-only/skills/src/list.test.ts)、[tests/direct-download-add.test.ts](../reference-only/skills/tests/direct-download-add.test.ts) | 保留其他 Agent 正在使用的内容、最后一个目标卸载时删除内容、JSON 列表与远程归档行为的测试依据 |

## 3. cc-switch 代码导航与可借鉴部分

### Skill 管理调用链

`UnifiedSkillsPanel / SkillsPage → useSkills → skillsApi → commands/skill.rs → SkillService → 文件目录 + Database DAO`

| 层次 | 文件 | 已确认的职责 |
| --- | --- | --- |
| 已安装管理 | [UnifiedSkillsPanel.tsx](../reference-only/cc-switch/src/components/skills/UnifiedSkillsPanel.tsx) | 列表搜索、应用启停、批量操作、导入、更新、ZIP、卸载与恢复 |
| 发现与来源 | [SkillsPage.tsx](../reference-only/cc-switch/src/components/skills/SkillsPage.tsx)、[RepoManagerPanel.tsx](../reference-only/cc-switch/src/components/skills/RepoManagerPanel.tsx) | 仓库筛选、skills.sh 搜索、仓库添加/删除 |
| API 与状态刷新 | [skills.ts](../reference-only/cc-switch/src/lib/api/skills.ts)、[useSkills.ts](../reference-only/cc-switch/src/hooks/useSkills.ts) | 已安装/发现/导入/备份 DTO；部分写入失败后重新读取权威状态 |
| Rust 入口 | [commands/skill.rs](../reference-only/cc-switch/src-tauri/src/commands/skill.rs) | `get_installed_skills`、`install_skill_unified`、`toggle_skill_app`、`scan_unmanaged_skills` 等 |
| 核心服务 | [services/skill.rs](../reference-only/cc-switch/src-tauri/src/services/skill.rs) | `get_ssot_dir`、`install`、`uninstall`、`check_updates`、`update_skill`、`import_from_apps`、`sync_to_app_dir`、备份恢复 |
| 数据模型与存储 | [app_config.rs](../reference-only/cc-switch/src-tauri/src/app_config.rs)、[dao/skills.rs](../reference-only/cc-switch/src-tauri/src/database/dao/skills.rs) | InstalledSkill、SkillApps、来源、hash、时间及 SQLite 持久化 |
| 行为测试 | [skill_sync.rs](../reference-only/cc-switch/src-tauri/tests/skill_sync.rs)、[UnifiedSkillsPanel.test.tsx](../reference-only/cc-switch/tests/components/UnifiedSkillsPanel.test.tsx) | 导入不反向改源目录、备份恢复、批量部分失败、并发操作限制 |

### MCP 管理调用链

`UnifiedMcpPanel / McpFormModal → useMcp → mcpApi → commands/mcp.rs → McpService → Database + mcp/<app>.rs`

| 层次 | 文件 | 已确认的职责 |
| --- | --- | --- |
| 管理页与编辑 | [UnifiedMcpPanel.tsx](../reference-only/cc-switch/src/components/mcp/UnifiedMcpPanel.tsx)、[McpFormModal.tsx](../reference-only/cc-switch/src/components/mcp/McpFormModal.tsx) | 搜索、添加/编辑/删除、应用开关、批量启停、配置输入与预设 |
| API 与刷新 | [mcp.ts](../reference-only/cc-switch/src/lib/api/mcp.ts)、[useMcp.ts](../reference-only/cc-switch/src/hooks/useMcp.ts) | 统一 MCP API；串行写入；成功和失败都刷新列表 |
| Rust 入口/服务 | [commands/mcp.rs](../reference-only/cc-switch/src-tauri/src/commands/mcp.rs)、[services/mcp.rs](../reference-only/cc-switch/src-tauri/src/services/mcp.rs) | CRUD、toggle、跨应用导入与同步 |
| 配置适配 | [mcp/claude.rs](../reference-only/cc-switch/src-tauri/src/mcp/claude.rs)、[mcp/codex.rs](../reference-only/cc-switch/src-tauri/src/mcp/codex.rs)、[gemini_mcp.rs](../reference-only/cc-switch/src-tauri/src/gemini_mcp.rs) | Claude JSON、Codex TOML 与字段转换、Gemini httpUrl/url 转换 |
| 校验/数据 | [mcp/validation.rs](../reference-only/cc-switch/src-tauri/src/mcp/validation.rs)、[dao/mcp.rs](../reference-only/cc-switch/src-tauri/src/database/dao/mcp.rs) | stdio/http/sse 基础校验、服务器定义和应用状态持久化 |
| 行为测试 | [mcp_commands.rs](../reference-only/cc-switch/src-tauri/tests/mcp_commands.rs)、[UnifiedMcpPanel.test.tsx](../reference-only/cc-switch/tests/components/UnifiedMcpPanel.test.tsx) | 导入不回写、坏配置隔离、未知条目保留、同文件写入互斥、搜索排除敏感字段 |

### 需要改进的参考行为

- cc-switch 部分 MCP 操作先更新数据库、再同步外部文件，失败后可能只完成一部分。dbox 应持久化“期望配置”和逐目标实际结果，不能用一个 boolean 表达成功。
- cc-switch 的部分扫描/更新发现流程记录错误后跳过。dbox 必须返回逐来源错误，把“未检查成功”和“没有更新”区分开。
- 两类资源都存在按名称/目录聚合的参考实现。dbox 的身份、部署路径和内容必须分别建模。
- cc-switch 支持的应用集合并不等于每个应用都支持 MCP；部分分支明确跳过。dbox 通过 capability 表控制可见操作。
- 不迁入 React、React Query、完整 AppState、Provider 切换、旧 API 兼容层或整套数据库迁移。直接复用有价值的 Rust 片段时保留 [MIT 版权声明](../reference-only/cc-switch/LICENSE)，记录来源。

## 4. dbox 现状与接入点

| 现有代码 | 当前情况 | 计划变更 |
| --- | --- | --- |
| [SkillsView.vue](../src/views/SkillsView.vue) | 只有“尚未启用”占位 | 实现已安装、发现、来源、导入与备份界面 |
| [router/index.ts](../src/router/index.ts)、[App.vue](../src/App.vue) | 已有 `/skills`，没有 `/mcp` | 新增 MCP 导航与路由 |
| [executor/plan.rs](../src-tauri/src/executor/plan.rs) | UpdatePlan 强制绑定 Tool/Installation/Component/Strategy；ConfirmedPlan 包装 UpdatePlan | 提取通用确认信息，增加 Skill/MCP 操作计划；保留工具更新适配入口 |
| [executor/runner.rs](../src-tauri/src/executor/runner.rs) | CommandExecutor 执行已确认的外部进程 | 继续负责命令；原生文件操作走独立步骤执行器，共用日志/取消/运行事件 |
| [domain/mod.rs](../src-tauri/src/domain/mod.rs)、[application/run_queue.rs](../src-tauri/src/application/run_queue.rs) | Run 和队列条目都绑定工具；队列使用 CommandExecutor | 增加资源类型与操作类型，给队列增加显式执行分派；不能虚构 ToolId 来包装 Skill/MCP |
| [persistence/mod.rs](../src-tauri/src/persistence/mod.rs) | settings TOML、state JSON、run JSONL；原子写入和 revision 已存在 | 增加 skills.json、mcp.json 的读写和备份清单，复用原子写入；不引入数据库 |
| [api/mod.rs](../src-tauri/src/api/mod.rs)、[dto.rs](../src-tauri/src/api/dto.rs)、[service.rs](../src-tauri/src/api/service.rs) | Rust DTO 经 Specta 生成 TS；ApiService 组装三类工具 Provider | 注入独立 Agent/Skill/MCP 服务，新增生成 DTO/command/event |
| [api/dev_http.rs](../src-tauri/src/api/dev_http.rs)、[httpTransport.ts](../src/api/httpTransport.ts) | 浏览器开发桥显式列举命令和事件 | 所有新增 API 同步提供 HTTP 路由、事件及 mock；浏览器和桌面行为一致 |
| [stores/runs.ts](../src/stores/runs.ts)、[ConfirmView.vue](../src/views/ConfirmView.vue)、[RunsView.vue](../src/views/RunsView.vue) | 确认和历史围绕 UpdatePlan/Tool 展示 | 增加文件变更预览、目标应用与资源名称、逐项结果、按操作重试 |

## 5. 共享基础设计

### 5.1 领域边界

- `AgentRegistry`：应用 ID、显示名、用户级/项目级能力、已解析配置根目录、Skill 路径、MCP 格式与支持的 transport。路径支持用户覆盖，缺失和无权限是显式状态；读取不能创建 Agent 目录。
- `SkillService`：管理 Skill 内容、来源、安装、部署、检查、更新、备份与导入。
- `McpService`：管理服务器定义、应用绑定、读取/导入和配置写入。
- `ExtensionOperationService`：管理预览、确认、互斥、执行、进度、恢复和失败结果；不承担 Skill/MCP 格式规则。
- npm/brew/mise 继续是工具 Provider；Agent、SkillSource、MCP 配置适配器不注册成包管理 Provider。

### 5.2 数据与存储

以少量本地 Skill/MCP 为目标，采用“读取 JSON → 内存修改 → 原子写回整个文件”。复用现有 `serde_json`、文件 revision 和原子写入能力，不引入 SQLite、其他数据库、ORM、DAO 框架或数据库迁移工具。

首版统一选 JSON，不同时维护 JSON/YAML 两套存储。YAML 用于读取 `SKILL.md` 的 frontmatter；Agent 配置仍按它自己的 JSON/TOML 等格式编辑。使用 `AppPaths` 推导路径，不硬编码 `~/.dbox`：

| 对象 | 关键字段/含义 |
| --- | --- |
| `AgentTarget` | agentId、scope、可选 workspaceId、解析路径、capabilities、共享目录标识 |
| `SkillRecord` | 稳定 skillId、名称/描述、来源类型与 URI、仓库内相对路径、requestedRef/resolvedRevision、当前 revision、时间 |
| `SkillDeployment` | skillId、targetId、部署路径、desiredEnabled、observedState、实际 copy/symlink、lastAppliedHash、错误 |
| `SkillSource` | sourceId、GitHub 仓库/本地目录/ZIP、ref、enabled、最近发现结果；停用来源不卸载已装 Skill |
| `McpServer` | 稳定 serverId、显示名、规范化 transport/config、目标特有字段、描述/标签 |
| `McpBinding` | serverId、targetId、该应用实际配置 key、desiredEnabled、observedState、lastAppliedHash |
| `OperationPlan` | planId/hash/expiry、资源与操作类型、目标清单、输入 revision、目标指纹、已准备内容 ID、步骤和冲突 |
| `OperationResult` | runId、整体状态、每个目标的结果/错误/恢复信息；缺失能力和未检查状态单独表达 |

这些是 Rust 领域对象，不是独立的数据表。SkillDeployment 嵌套在对应 SkillRecord 中，McpBinding 嵌套在对应 McpServer 中；来源与 Skill 一起保存在 skills.json。观测状态和检查结果可重新扫描，首版保存在内存，不为它们建立持久化索引。

| 路径 | 内容 |
| --- | --- |
| `config_dir/skills.json` | `schemaVersion`、sources 数组、skills 数组；每个 Skill 内包含自己的部署信息 |
| `config_dir/mcp.json` | `schemaVersion`、servers 数组；每个服务器内包含自己的应用绑定 |
| `data_dir/skills/<skillId>/content/` | 当前 Skill 内容，保留原始 SKILL.md 及附带文件 |
| `data_dir/extension-staging/` | 临时下载和安装内容，用完清理 |
| `data_dir/extension-backups/<runId>/` | 操作前文件/目录备份及 manifest.json，记录目标、指纹、步骤结果和恢复所需信息 |
| 现有 run JSONL | 脱敏进度与运行历史；不另外建立扩展操作数据库或日志系统 |

预览计划只保存在内存，过期或重启后重新生成。备份目录中的 manifest.json 同时承担未完成操作的识别，不另设 journal/WAL 或事件重放机制。只保存当前内容和有数量/大小上限的备份，不建立按 hash 分层的长期版本仓库；尚未处理完的失败操作备份不自动清理。

文件读写采用具体的 load/save 函数即可，新增 JSON 只带一个 schemaVersion，格式变更时写小型兼容转换函数。应用内写入串行，保存前重新读取文件指纹以检测用户手工修改；解析错误保留原文件并提示，不能用空列表覆盖。两个 JSON 文件分别保存，跨应用失败通过逐项结果与备份处理，不构建跨文件事务协调器。

统一技能库必须放在 dbox 私有数据目录，避免把保存但已停用的 Skill 放进 Agent 会自动加载的公共目录。记录按 source identity 与内容定位，部署名另行校验；不能只用 Skill 名称或目录 basename 作为主键。初版固定管理库位置，不做存储位置迁移 UI。

### 5.3 确认、执行与恢复

1. 读取或下载到 dbox 暂存区，生成可核对的变更计划。预览允许产生缓存，不修改 Agent 文件和已安装内容。
2. 计划固定源 revision/contentHash、目标文件或目录指纹、受影响应用、备份策略和步骤；确认时只提交 planId/hash，不接受前端任意路径写入请求。
3. 执行前重新检查资源 revision、目录解析和目标指纹；外部变化时返回冲突并要求重新预览。
4. 扩展写操作首版串行执行，并按规范化后的真实目标路径互斥，覆盖路径别名。读取/网络发现可限流并发。
5. 写前生成备份与 manifest.json，逐目标执行与校验后更新清单中的结果，复用现有 Run 记录进度。跨多个目录/文件不宣称具有一个原子事务。
6. 单文件使用原子替换；Skill 副本在目标同文件系统暂存后替换，先保留旧目录。symlink 切换与副本替换分别实现并测试。
7. 出错保留可恢复记录；回滚前核对文件是否仍为本次写入内容，避免覆盖新产生的外部修改。多目标部分成功返回 `partial`。
8. 取消在下载、解析、步骤间生效；不可分割的替换步骤结束后再停止。启动时根据 Run 和备份清单识别未完成操作，标记 interrupted 并重新扫描，提供重试或从备份恢复，不自动重放步骤。

`Run` 改为携带 `RunSubject`（tool_update / skill / mcp）及操作摘要，旧工具记录迁移成 tool_update。按实际 DTO 变化更新 schema revision 和 golden fixtures。提取公共确认元信息后，保留 UpdatePlan 的工具校验逻辑；通过枚举/执行器分派接入扩展操作，避免把文件写入编码成 shell 命令。

指纹重检与原子替换可检测常见外部修改，但不能阻止不遵循锁协议的外部程序恰好同时写入；应保留备份、写后校验及冲突恢复，不能承诺绝对跨进程事务。

## 6. Skill 管理实现计划

### 6.1 完整首版能力

- 已安装列表：名称、描述、来源、应用部署、更新时间、检查结果；按应用、来源、启用状态搜索/筛选。
- 发现与来源：添加 GitHub 仓库和 ref，启停/删除来源，查看仓库内 Skill；支持本地目录、本地 ZIP；skills.sh 搜索作为独立发现适配器。
- 安装：查看 Skill 元信息及内容预览，选择应用、部署方式，展示路径与冲突，确认后安装；允许只保存到库、暂不启用。
- 启停：按应用启用、停用和批量操作；全部停用也保留统一库，可离线恢复启用。
- 导入：扫描已安装但未被 dbox 管理的 Skill，选择来源、处理同名不同内容、接管后可管理。
- 更新：独立检查、单项更新、更新全部；保留来源 ref、仓库相对路径及部署目标，不把同名远端 Skill 自动视为原始来源。
- 卸载与恢复：卸载前备份；备份列表、恢复、删除备份。停用与卸载在 UI 上分开。

### 6.2 文件与来源规则

**发现与安装。** 使用成熟 YAML/frontmatter 库解析 `SKILL.md`，保留原文和未知 frontmatter 字段。仓库递归发现设置深度与文件数上限，跳过 `.git`、构建产物等；按完整相对路径区分同名条目。GitHub 下载固定 resolved commit，不能确认某个 branch 后再安装另一份最新内容。首次支持公开 GitHub、本地目录和 ZIP，私有仓库认证及任意 Git 主机作为后续来源适配器。

远程获取、解压、扫描只写 dbox 暂存区。校验绝对路径、`..`、符号链接逃逸、压缩炸弹、重复路径以及大小写冲突；保留脚本可执行位，拒绝归档中的设备等特殊文件，不运行 Skill 内的任何脚本。网络失败按来源返回错误，已有发现缓存标明时间。

**已有安装导入。** 先只读扫描目标目录，返回每个实际路径、规范化路径、链接目标、内容 hash 和所属应用。不同目录同名但不同内容分别展示，不选择“第一个匹配”。默认导入为 dbox 管理库中的快照并记录外部部署，保持原目录不变；显式“接管此应用部署”时才生成备份与替换计划。在没有接管前，外部副本只观察，不允许 dbox 停用、更新或删除它。

对于由 CLI、cc-switch 或插件管理器创建的安装，保留 `external` 标记。来源信息可通过用户补充，或独立、版本化的只读元数据导入器取得；外部 lock 格式不认识时标记来源未知，不猜测仓库，不写回 `.skill-lock.json` / `skills-lock.json`。指向 cc-switch 等外部管理库的链接也须显式接管；dbox 不删除原管理库。

**部署与启停。** Auto 优先 symlink，无法支持时回退 copy，并记录实际结果；更新 copy 部署前比对上次 hash，保护用户修改。删除链接只删除链接本身；删除副本必须同时通过 dbox 所有权记录和内容指纹核对。路径别名、父子目录重叠、断链须作为独立情况处理。

如果多个 Agent 会读取同一个实际目录或公共回退目录，UI 必须显示关联影响，并按共享目标一起操作或标为不支持独立启停。不能只改变 JSON 记录中的某个应用开关，就宣称它已单独停用。

**检查与更新。** 检查只比较远程暂存内容、当前管理库内容 hash 和部署状态；结果保存在内存，不修改管理库内容和 Agent 部署。区分 up_to_date / update_available / local_modified / source_unknown / unavailable / conflict。更新先暂存新内容并备份旧内容，再替换当前 content 目录。指向同一 content 的 symlink 部署会一起生效，预览须列出全部关联目标；copy 部署逐个替换并记录 lastAppliedHash。失败的 copy 保留原内容，共享内容替换失败时恢复原目录，多目标未全部成功则报告 partial，不为每个应用建设独立版本仓库。

本地目录、ZIP 或无明确远程来源的导入项显示“无法检查远程更新”，允许显式重新导入，不显示“已是最新”。远端路径消失/移动不靠 basename 自动匹配。

**卸载与恢复。** 卸载计划展示将移除哪些已接管的应用部署以及库记录；先备份内容、来源、部署方式与状态，再执行移除。外部修改导致清理失败时保留原记录并标记待清理，供继续处理。恢复也生成新计划，不覆盖冲突路径；最后一份可恢复内容不能在操作仍未收敛时被自动清理。

## 7. MCP 管理实现计划

### 7.1 完整首版能力与配置映射

提供列表、搜索、手动新增/编辑、JSON 导入、从应用扫描导入、按应用启停、删除、手动同步和逐目标结果。表单支持 stdio 和目标应用允许的 HTTP/SSE，提供少量配置模板及高级字段编辑。首版管理配置，不启动后台 MCP 服务，也不提供连接测试、工具调用、OAuth 登录或服务器安装。

以下是从 cc-switch 读取到的用户级布局，作为适配器 fixture 起点。开发适配器时需核对目标应用版本与配置规范，不能把参考实现视为当前规范的替代品。

| 应用 | 默认用户级文件 | 适配要点 |
| --- | --- | --- |
| Claude Code | `~/.claude.json` 的 `mcpServers` | 与 `~/.claude/settings.json` 区分；保留 projects 等无关字段；自定义根目录规则单独解析 |
| Codex | `~/.codex/config.toml` 的 `mcp_servers` | 用保留格式的 TOML 编辑器；处理 URL 型配置、headers/http_headers 及未知字段，不机械复制 JSON type |
| Gemini CLI | `~/.gemini/settings.json` 的 `mcpServers` | HTTP 的 httpUrl、SSE 的 url 与统一 transport 双向映射；保留其他设置 |

`AgentRegistry` 返回配置路径及来源（默认/环境/用户覆盖）和可支持的操作。没有初始化目录时显示 unavailable；需要创建时单独展示明确的初始化变更，不能读列表时顺带创建文件，也不能跳过写入却返回已启用。

### 7.2 配置模型与写入规则

- 规范化核心字段：stdio 的 command/args/env/cwd；远程连接的 URL 与认证字段。把应用特有扩展留在对应 binding，不能把某应用的未知字段盲目广播到其他应用。
- 按应用判断 transport、认证和字段能否无损转换；不支持的映射返回 unsupported。表单前端校验用于提示，Rust 仍是校验边界。
- command/args 分开保存。校验 command 是路径/可执行文件检查，不通过启动 MCP 进程验证；不把 env、headers、URL 中的凭据写入日志或搜索索引。
- 同一配置文件中的批量操作先合并成一次文件变更，保留全部无关顶层字段与未托管服务器。TOML 保留注释/格式；JSON 保留语义，重排需体现在预览中；不支持的 JSONC 等格式不得强行按 JSON 重写。
- 外部配置中的 enabled 等状态按应用解释，不能仅凭条目存在就推断有效启用。dbox 同时展示期望状态和读取到的状态；配置已写入不等于应用已热加载或服务器可连接。

### 7.3 导入、冲突和同步流程

1. 扫描所有选定应用，只读生成候选；某应用文件损坏不妨碍显示其他应用的结果。
2. 候选使用 targetId + 配置 key 区分；同名同配置可建议合并，不同配置必须选择分别保留、采用哪一份或跳过。每个 binding 允许不同配置 key。
3. 导入只写 dbox 自身记录，不反向同步到应用。记录来源配置 key/hash，作为后续明确管理该条目的依据。
4. 新增、编辑、启停、删除、同步均先生成按文件分组的脱敏 diff、受影响服务器和备份计划，再确认执行。
5. 修改前核对整文件指纹以及托管条目上次写入 hash；检测到外部变化时重新读取并提示冲突，不以 dbox 的旧配置覆盖。
6. 逐文件备份、原子替换、重读校验；返回每个应用 applied/unchanged/conflict/unavailable/failed。对已知冲突默认不执行该项，其余明确授权的目标可继续。
7. 禁用删除目标应用中的托管条目，但保留 dbox 服务器定义。完全删除时保留待清理记录，直到成功移除对应部署或用户明确选择仅忘记记录。
8. 同步失败后仍刷新列表，保留 desired 与 observed 差异；重试只针对未成功目标重新生成计划，不再次覆盖已成功目标。

敏感值在内部配置和应用文件中按需要保存，文件/备份使用当前用户权限；普通列表、日志、事件与 diff 脱敏。编辑 DTO 使用 keep/set/remove 语义，避免把脱敏占位符保存为真实密钥。备份保留可恢复的原文但不进入普通日志。

## 8. 模块、接口与前端安排

以下为方案制定时的目录和接口建议；实际模块组织见第 12 节及 ADR 0004：

```text
src-tauri/src/
  agents/                    # 路径解析、capabilities、共享目录关系
  skills/                    # 模型、发现、来源、部署、更新、备份
  mcp/                       # 模型、校验、导入与各应用适配器
  application/extensions/    # 操作计划、确认、执行分派、恢复
  persistence/extensions.rs  # skills.json、mcp.json 和备份清单的简单读写
  api/                       # Skill/MCP DTO、commands、events、dev HTTP
src/
  views/SkillsView.vue
  views/McpView.vue
  components/skills/         # 列表、来源、安装/导入、备份
  components/mcp/            # 列表、表单、导入、同步结果
  components/operations/     # 文件 diff、部署计划和逐目标结果
  stores/skills.ts
  stores/mcp.ts
```

| 接口组 | 建议 command | 语义 |
| --- | --- | --- |
| 应用 | `list_agent_targets` | 读取能力、配置路径与可用性 |
| Skill 查询 | `list_skills`、`discover_skills`、`search_skills`、`scan_skill_imports`、`check_skill_updates`、`list_skill_backups` | 返回快照/逐来源错误；下载和扫描只写暂存与检查缓存 |
| Skill 来源 | `list_skill_sources`、`save_skill_source`、`delete_skill_source` | revision 保护的 dbox 自身配置；不改变已有部署 |
| Skill 计划 | `preview_skill_operation` | install / import / adopt / toggle / update / uninstall / restore / delete_backup |
| MCP 查询 | `list_mcp_servers`、`scan_mcp_imports`、`validate_mcp_server` | 返回规范化、原始字段映射与逐应用诊断 |
| MCP 计划 | `preview_mcp_operation` | import / upsert / toggle / delete / sync；输入冲突解决选择 |
| 操作执行 | `confirm_extension_operation` | 接收 planId/hash，返回 runId；运行详情通过现有历史/事件读取 |
| 取消与结果 | 扩展 `cancel`、`run_history`、`run_log` | 对工具与扩展统一；取消与重试按执行类型分派 |

新增 skills_changed / mcp_changed / operation_progress 事件携带 revision、sequence、runId 和必要 ID；继续复用 run_state/run_output。网络发现较长时提供 requestId/取消和进度，不能阻塞前端导航。DTO 由 Rust 生成，不手写第二份 TypeScript 契约。

前端先延续 dbox 的基础组件、Pinia 和样式。Skill 页使用“已安装 / 发现”主视图，来源、导入、备份为明确入口；MCP 页显示应用启用状态和同步异常。顶部批量操作必须明确“选中项/当前筛选/全部”的作用范围，不能让搜索过滤后出现数量和实际目标不一致。

本地已安装列表全量读取，在内存中筛选和排序即可，不为它新增分页、搜索索引或虚拟列表。远程目录检索是否分页由来源接口决定。

写操作 pending 时锁住相同资源的冲突操作；成功、失败或部分成功后都重新读取受影响集合。缓存按资源 scope/target/workspace 分开，过期响应不能覆盖新筛选结果。复用确认和 Runs 页面时更新返回路径、文案和类型展示，避免仍显示“更新工具”。

## 9. 分阶段任务与验收

各阶段的测试随阶段完成，不推迟到最后统一补齐。现有工具管理始终保留回归门禁。

| 阶段 | 内容和主要文件 | 依赖 | 完成标准 |
| --- | --- | --- | --- |
| P0：契约与基础 | `agents/`、Skill/MCP 模型、两个 JSON 文件的 load/save；记录 ADR 和依赖选型 | 无 | 少量记录的 JSON 往返、缺失/坏文件、手工修改冲突、原子写入测试；三应用路径 fixture 与支持矩阵；不新增数据库依赖 |
| P1：计划与运行扩展 | plan、run、queue、events、DTO、Confirm/Runs 基础 | P0 | 原工具更新用例不退化；命令/原生步骤分派、过期计划拒绝、取消、partial、恢复、旧历史迁移通过 |
| P2：Skill 本地管理后端（T21 第一部分） | `skills/` 扫描、导入、接管、copy/symlink、启停、卸载/备份恢复 | P0、P1 | 临时目录中完成导入→接管→全部停用→离线启用→卸载→恢复；未接管目录不被改写 |
| P3：Skill 来源与更新（T21 第二部分） | GitHub/local/ZIP、发现、sources、skills.sh 适配、check/update | P2 | mock 网络中完成发现→安装→独立检查→更新；精确来源路径、并发卸载、本地修改和部分失败可追溯 |
| P4：MCP 后端 | `mcp/` 三应用读取/转换/导入/diff/写入与备份 | P0、P1 | 同名冲突处理；坏配置隔离；未知字段/TOML 注释保留；多应用部分成功、取消、恢复通过 |
| P5：Skill UI（T22） | SkillsView、skills store、各管理弹窗、transport | P2、P3 后端验收 | 从 UI 完成 Skill 完整流程；同名来源选择、外部部署、不可更新、批量作用范围与失败刷新都有组件测试 |
| P6：MCP UI | McpView、router/App、mcp store、配置表单、diff 与结果 | P4 后端验收 | 从导入/创建到分应用启停/删除可完成；敏感值 keep/set/remove、unsupported、冲突和重试都有测试 |
| P7：集成发布验收 | Tauri/HTTP 合同、fixtures、构建、使用说明、依赖审计 | P5、P6 | 下述端到端场景及仓库门禁通过；文档明确支持范围和已知限制 |

推荐顺序：`P0 → P1 → P2 → P3 → P4 → P5 → P6 → P7`。P4 技术上只依赖 P0/P1，可根据优先级提前；依赖关系不意味着本次授权并行开发。

### 必须覆盖的验收场景

1. 同名不同仓库/不同内容的 Skill 和 MCP 均不被静默合并；同内容路径别名不会导致自我复制或删除。
2. Skill 全部停用后仍在库中，断网可重新启用；外部 Skill 未接管时无法被 dbox 删除。
3. 只检查更新时所有 Agent 目录和已安装内容保持不变；网络失败显示未检查成功，不显示已是最新。
4. ZIP 路径逃逸、超限、危险链接被拒绝；正常多 Skill ZIP 可逐项选择并记录结果。
5. 本地改动阻止自动覆盖；Skill 内容替换失败可恢复原目录，失败的 copy 保留原内容；共享 symlink 更新影响与预览一致；卸载前完成备份且可恢复。
6. MCP 导入不改写任一应用文件；坏 JSON/TOML 原样保留；其他应用仍可扫描。
7. 同一文件批量修改只提交一次；保留未托管条目、未知字段和 TOML 注释。确认之后外部修改触发冲突。
8. 单目标写入失败或进程中断后，记录和文件状态可重新核对；恢复不会自动重放破坏性步骤。
9. 机密 env/header/URL 值不出现在列表搜索、错误、事件、日志和普通预览；编辑未改动密钥时不会被占位符覆盖。
10. Tauri 与 dev HTTP 返回相同 DTO/错误语义；订阅先于执行，重复事件、断线重连、旧响应不会错报完成。
11. 原 npm/Homebrew/mise 的刷新、预览、确认、取消和历史读取继续通过；旧 state/settings 可升级。
12. skills.json/mcp.json 可直接阅读和备份，少量 fixture 验证完整读写即可；损坏文件不被覆盖，存储实现和依赖中没有数据库、ORM 或额外索引。

自动化只使用 TempDir、注入的 Agent 根目录、mock HTTP 与 fake executable；不读写真实用户 Agent 配置，不调用真实安装/卸载命令。

实现阶段的门禁命令：

```sh
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml --features dev-http
npm run bindings:check
npm test
npm run build
```

依赖与许可证审计按现有 T16/ADR 执行。TOML 编辑、HTTP、ZIP、YAML、目录遍历与文件锁等候选依赖在 P0 锁定版本/feature 并检查 MSRV、平台兼容性；本计划阶段不新增依赖。

## 10. 后续能力与取舍

- 项目级管理：显式选择 workspace；独立保存 scope/target，不自动递归扫描全部项目；项目与用户级同名资源分开。先补齐共用 `.agents/skills` 的关联启停语义，再开放对应开关。
- 更多 Agent：按适配器逐个加入 OpenCode、Pi、Hermes 等；OpenCode 的 JSONC、Pi 的原生加载规则各有测试，不能照搬三应用规则。
- 更多 Skill 来源：私有 Git、GitLab、任意 Git URL、远程归档、锁文件互操作；不得为支持它们重新引入共同管理同一目录的双写后端。
- MCP 连接测试/OAuth/服务器安装：单独规划，避免把配置写入成功与运行健康混为一谈。
- 管理库迁移、云同步、编辑 Skill 正文、市场推荐与定时更新不属于首版；备份恢复、冲突处理和只读检查属于首版，不以“后续优化”延期。

原生方案最大的成本是维护 Agent 配置兼容和文件生命周期。通过固定首版范围、成熟基础库、明确所有权和逐阶段 fixture 测试控制成本。未来若 CLI 提供完整的稳定管理 API，可以重新评估来源/执行适配层；现阶段不以这种可能性阻塞计划。

## 11. 方案制定时的文档变更边界（历史记录）

本次交付此方案，并同步修订旧开发计划、任务索引、T21/T22 描述及开源库审计中的旧 T21 结论，撤销“Skill 只能通过 npx skills 管理”的旧约束。任务状态仍为 Future/Proposed，没有声称功能已完成。上述目录、接口、数据结构、测试和依赖均为待实施项；`src/`、`src-tauri/` 和参考仓库未因本计划被修改。


## 12. 实现交付记录（2026-09-10）

已交付 Claude Code、Codex、Gemini CLI 的用户级 Skill/MCP 管理，详见 [使用说明](skills-mcp-management.md) 和 [ADR 0004](adr/0004-native-extensions.md)。

- 共享基础位于 `agents/` 与 `extensions/`：固定内容计划、revision/路径检查、原生串行执行、取消、备份/恢复、部分完成和中断记录。没有数据库；JSON 文档和 private 内容库由 AppPaths 推导。
- Skill 包含本地/GitHub/ZIP 发现、来源管理、skills.sh 搜索、安装、外部快照导入/接管、共享应用启停、独立检查、更新、卸载及恢复。
- MCP 包含三应用适配、CRUD、JSON/应用导入、按文件合并同步、按目标结果、认证映射限制、keep/set/remove、注释/未知字段保留与备份恢复。
- `RunSubject`、API/state schema 2、旧历史迁移、Tauri/HTTP 生成契约与事件均已接入。工具命令队列保留，原生操作由 ExtensionService 分派到 blocking worker；共用历史存储与运行事件，不伪造 ToolId。
- Skill/MCP 页面复用资源页内 OperationPanel；原工具 ConfirmView 保留。Runs 展示资源名、操作、逐目标结果与日志，失败时可返回资源页重新预览。
- 自动化使用 TempDir、隔离 AgentRegistry、fake executable 和本机 mock HTTP。浏览器验证使用 debug `--fixture-root` 模式，未操作真实用户 Agent 配置。

当前明确限制：仅用户级、公开 GitHub 和本地目录/ZIP；不解析第三方 lock 元数据、不提供 MCP 运行/连接/OAuth；ZIP64/JSONC/内容内部链接明确拒绝；共享 Codex/Gemini 目录一起启停；Windows 尚待实机验证。恢复默认保留外部后续修改；未完成备份不自动清理。以上约束在界面和管理说明中说明。

门禁：fmt、Clippy、默认/`dev-http` Rust 测试、bindings:check、Vitest、前端 build，以及 cargo-deny 的 advisories/licenses/bans/sources。依赖许可原文已随 Tauri resources 打包；旧测试中等待 PID 文件的竞态一并修正。
