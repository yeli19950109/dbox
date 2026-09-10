# T21：原生 Skill 管理后端

- 状态：Completed
- 阶段：原生扩展管理
- 依赖：T08、T09、T15、T16，以及接入计划 P0/P1
- 阻塞：T22

## 目标

参考 `reference-only/cc-switch` 的 Skill 服务实现原生管理：统一内容库、来源发现、安装、按应用启停、已有安装导入/接管、独立更新检查、更新、卸载与备份恢复。

2026-09-10 已按用户要求比较 `skills` CLI 的实际能力，取消原有 `npx skills` 唯一后端约束。当前文件名保留以兼容已有链接；原薄适配任务内容由本文替代。选型证据、边界及详细行为见 [Skill 与 MCP 管理接入计划](../skills-mcp-integration-plan.md)，对应 P2/P3。

## 交付内容

- 首版提供 Claude Code、Codex、Gemini CLI 的用户级管理，能力与路径由 AgentRegistry 提供。
- dbox 私有内容库、SkillSource、SkillRecord、SkillDeployment 和备份元数据；保留来源相对路径及 resolved revision。
- 来源、Skill 及嵌套部署信息直接保存为 skills.json，使用现有 JSON/原子写入能力；不引入数据库、ORM、索引或通用存储框架。内容只保留当前目录和有限备份，备份 manifest.json 记录恢复信息。
- 支持公开 GitHub 来源、本地目录、本地 ZIP；仓库来源增删/启停、仓库发现和独立 skills.sh 搜索适配。
- 只读扫描外部 Skill；显式导入快照、来源冲突选择和部署接管，不自动替换原目录。
- 支持 Auto/Symlink/Copy，记录实际方式；全部应用停用后仍保留本地内容。
- 检查只更新 dbox 检查缓存，不改变已安装内容；无来源、远端失败、本地修改与无更新分别返回。
- 更新、卸载、恢复、删除备份均生成操作计划；受影响路径、指纹、备份和逐项结果可核对。
- 外部来源未知时明确标记 unknown；首版不解析或改写 CLI / cc-switch 的状态和 lock。
- Rust DTO 生成 bindings；同时提供 Tauri/HTTP 接口、进度、取消与 Run 历史。

## 测试要求

- TempDir 模拟三应用根目录和管理库，覆盖导入→接管→停用→离线启用→卸载→恢复。
- 同名不同来源、断链、路径别名/重叠、共享加载目录、外部修改不被静默覆盖。
- mock HTTP 覆盖来源不可达、远端路径变化、ZIP 越界/超限、多 Skill 选择及更新部分失败。
- 检查更新前后 Agent 目录和已安装内容保持一致；未知不能报告为 up-to-date。
- 过期计划、并发变更、取消和中断恢复都有 fixture；旧工具更新门禁继续通过。
- 自动化不读写真实用户 Skill，不调用真实安装命令。
- 少量记录验证 JSON 读写、外部手工修改冲突和坏文件保护，不以数千条资源为设计或验收目标。

## 完成标准

接入计划 P2/P3 的管理流程和异常场景通过 Rust 测试及生成契约验证；所有权、来源、期望启用与实际部署均可追踪；不依赖 Node/npx 才能完成基本管理。

## 完成记录（2026-09-10）

已实现 `agents/`、`skills/`、`extensions/`，接入 Tauri/HTTP、RunSubject、备份清单与原子 JSON 存储。临时目录测试覆盖完整导入/接管/停用/恢复流程、共享路径、复制副本本地修改、ZIP 安全与多 Skill、固定 GitHub commit、只读检查、取消/中断、同名身份和旧历史迁移。新增 HTTP 测试校验原生运行与历史 DTO 一致。

依赖及平台边界见 [ADR 0004](../adr/0004-native-extensions.md)，实际使用和限制见 [管理说明](../skills-mcp-management.md)。
