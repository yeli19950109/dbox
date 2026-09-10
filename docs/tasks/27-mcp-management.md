# T27：MCP 管理与共享运行基础

- 状态：Completed
- 日期：2026-09-10
- 依赖：T15、T17、Skill/MCP 接入计划 P0/P1

三应用 MCP 配置适配、JSON 与应用扫描导入、创建/编辑、分应用启停、删除、手动同步、冲突保护、备份恢复和逐目标结果已交付。界面包括 McpView、McpEditor、Pinia store 与共享 OperationPanel/AgentPaths。敏感值保留在私有配置中，普通 DTO/日志/预览脱敏；编辑使用 keep/set/remove。

RunSubject 支持原生扩展操作，state/API schema 升到 2，旧工具历史仍可读取；不使用虚构工具 ID。Tauri/HTTP 共用服务和生成 DTO，事件连接在确认前建立。窗口重获焦点、事件重连以及写操作完成后重新读取权威列表。

测试覆盖 JSON/TOML 损坏隔离、导入不回写、HTTP/SSE 映射、全局禁用规则、同名身份、未知字段和嵌套 TOML 注释保留、配置文件 symlink、文件指纹冲突、部分成功、密钥保留/替换/移除、备份恢复和 HTTP 历史合同。浏览器中验证仅选 Codex 时只写隔离 Codex 文件，未创建其他应用 MCP 文件。

更多说明见 [管理使用说明](../skills-mcp-management.md) 与 [ADR 0004](../adr/0004-native-extensions.md)。
