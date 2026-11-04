# ADR 0005：Codex 会话用量统计

- 状态：Accepted
- 日期：2026-11-04
- 范围：从 Codex JSONL 会话日志统计 token 用量（只读）

新增 `usage` 模块与“用量”页面，从 `$CODEX_HOME`（默认 `~/.codex`）的 `sessions/**/rollout-*.jsonl` 与 `archived_sessions/` 逐请求统计 token，并按模型、按本地日期汇总。解析逻辑移植自 cc-switch 的 `services/session_usage_codex.rs`，保留其精确语义：

- 优先使用 `last_token_usage`；缺失时以累计 `total_token_usage` 的高水位求差；
- 限额刷新在不同 `limit_id` 下重发的相同快照只计一次；
- `turn_context`、`thread_settings_applied` 与 token 事件里的模型名都会归一化（小写、去掉 `provider/` 前缀、去掉 `-YYYY-MM-DD` / `-YYYYMMDD` 日期后缀）；
- 远程压缩只写在 `token_usage_record` 里的用量，在随后 `last` 全 0 的 `token_count` 处补记；
- 子代理 / fork 会话开头重放的父会话 token 事件按父时间线做子序列对齐后剔除；可验证的 `history_base` 有界 fork 不需剔除；分页续写页继承前序分段的累计基线；
- 父会话缺失、ID 冲突、续写页基线无法恢复等无法可靠归属的文件不计入，并在页面列出原因。

与 cc-switch 的差异：dbox 不引入数据库，每次查询全量重扫，按文件大小 + mtime 缓存单文件解析结果（依赖其他文件的续写页不缓存），请求在内存中按 `线程 ID:序号` 去重，因此 `sessions/` 与 `archived_sessions/` 的同一 rollout 不会重复计算。未移植费用计算（dbox 没有定价表）和请求耗时估算。

## 依赖选择

| 用途 | 依赖 / feature | 上游许可证 / 声明 MSRV |
| --- | --- | --- |
| 读取 Codex 压缩会话 `*.jsonl.zst` | zstd 0.13.4（zstd-safe 7.4.0、zstd-sys 2.1.2），默认 feature | MIT / 1.64 |

Codex 会把 7 天未活动的 rollout 压缩成单帧 zstd 文件；不读取它们会少算较早的历史，因此引入与 cc-switch 相同的 zstd 绑定。zstd-sys 构建时编译 C 源码（BSD-3-Clause / GPLv2 双许可，按 BSD 使用）。
