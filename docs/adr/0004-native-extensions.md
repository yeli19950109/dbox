# ADR 0004：原生 Skill 与 MCP 管理

- 状态：Accepted
- 日期：2026-09-10
- 范围：Claude Code、Codex、Gemini CLI 用户级 Skill 与 MCP 配置管理
- 验证环境：macOS / Rust 1.97.1；其他桌面平台未进行实机验收

采用 Rust 原生服务和 JSON 管理记录，不依赖 `npx skills`，不引入数据库。`ExtensionService` 负责原生操作的预览、确认、串行执行、备份和恢复；`skills` 负责内容/来源，`mcp` 负责配置适配，`AgentRegistry` 解析应用目录。工具更新仍由原有执行器/队列处理，API 显式分派扩展操作。两类操作共用 Run、JSONL 日志和运行事件；state 写入共用进程内互斥，避免运行记录互相覆盖。

`RunSubject` 区分 tool_update / skill / mcp；扩展记录的 toolId 为 null，不创建虚构工具。state/API schema 升到 2，旧工具记录按默认 subject 兼容读取。计划仅留在内存，固定来源内容、管理记录 revision 和目标指纹，10 分钟过期；确认只接受计划 ID/hash。执行放到 blocking worker，取消在网络请求和原生步骤间生效。

文件替换使用 `atomic-write-file`。目录替换先在目标所在文件系统准备新内容，再保留旧目录、切换、校验。备份 manifest 保存原内容与前后记录，启动时标记中断且不自动重放。跨多个目录/文件的结果可以是 partial，不承诺跨进程事务。MCP 每个配置文件合并为一次替换，未知顶层字段/未托管条目保留，TOML 用递归表更新保留注释。

## 依赖选择

以下为本次 Cargo.lock 锁定的直接依赖；兼容版本线写在 Cargo.toml，具体版本由锁文件控制。

| 用途 | 依赖 / feature | 上游许可证 / 声明 MSRV |
| --- | --- | --- |
| 下载和 skills.sh 查询 | reqwest 0.12.28；关闭默认 feature，rustls-tls/json/stream | MIT OR Apache-2.0 / 1.64.0 |
| YAML frontmatter | serde_yaml_ng 0.10.0 | MIT / 1.64 |
| 保留格式编辑 TOML | toml_edit 0.22.27；serde | MIT OR Apache-2.0 / 1.66 |
| 有界目录遍历 | walkdir 2.5.0 | Unlicense/MIT / 未声明 |
| ZIP 解码/校验 | zip 2.4.2；关闭默认 feature，仅 deflate | MIT / 1.73.0 |
| 用户目录解析 | dirs 6.0.0，由原可选依赖改为运行时依赖 | MIT OR Apache-2.0 / 未声明 |

Rustls 的 ring/webpki/untrusted 使用 ISC，webpki-roots 的根证书数据使用 CDLA-Permissive-2.0；已核对对应许可证原文，将它们加入许可清单，并把文本放入 `src-tauri/resources/licenses` 随应用打包。没有新增 RustSec 忽略项。审计发现原有 chacha20 0.10.1 已撤回，更新到兼容的 0.10.2。原 ADR 0003 的已有例外仍单独保留。

ZIP 解码委托库完成。zip 2.x 会按名称折叠完全重复的条目，因此额外核对经典 EOCD 的条目数量；在解码索引分配前限制数量。首版拒绝 ZIP64、嵌套链接/特殊文件、绝对路径/父级穿越、重复/大小写冲突；限制 10,000 条目、单文件 16 MiB、总计 128 MiB、路径深度 24。源码和备份不执行任何 Skill 脚本，复制保留执行位。

首版统一使用 JSON 保存 skills/mcp 元数据。来源未知的外部安装保留 unknown，不读取或回写 CLI/cc-switch 的锁文件；同名不同来源分别保留，并由用户选择实际路径/应用 key。没有复制参考仓库的实现代码。

## 应用规范核对

- [Claude Code MCP](https://code.claude.com/docs/en/mcp)：用户级 `~/.claude.json`，保留项目配置；首版不修改各项目的禁用/审批列表。
- [Codex MCP](https://developers.openai.com/codex/mcp/)：`mcp_servers`、HTTP `http_headers`、`enabled`；不向 Codex 写入通用 JSON `type`，SSE 标为不支持。
- [Codex Skills](https://developers.openai.com/codex/skills/) 与 [Gemini Skills](https://geminicli.com/docs/cli/skills/)：默认共享 `~/.agents/skills`，通过共享目标一起操作。旧应用路径用于已有安装扫描。
- [Gemini MCP](https://geminicli.com/docs/tools/mcp-server/)：HTTP `httpUrl` / SSE `url`，并读取 `mcp.allowed` / `mcp.excluded` 的全局限制；不会悄悄重写这些限制。

应用支持表是配置/文件层面的支持，不代表应用已重新加载配置或服务器可以连接。此版本不提供项目级管理、OAuth、连接测试或服务器进程安装。
