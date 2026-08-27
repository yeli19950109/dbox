# T08：基于开源库的命令执行适配层

- 状态：Completed
- 阶段：Rust 执行内核
- 依赖：T06、T07
- 阻塞：T10–T16、T21

## 目标

使用成熟 Tokio 生态库执行已经确认的 CommandPlan。dbox 只负责编排库能力、映射 Run 状态和发送业务事件，不自行实现进程管理器、PID 树遍历、异步 runtime、日志框架或取消原语。

## 开源库优先方案

实现前以 spike/ADR 验证以下组合；版本由 `Cargo.lock` 固定，不能仅写宽泛依赖后未经测试升级：

| 能力 | 优先库 |
| --- | --- |
| 异步进程、stdout/stderr、timeout/select | [`tokio`](https://docs.rs/tokio/latest/tokio/process/struct.Command.html) |
| 进程组/session、kill-on-drop、跨平台子进程包装 | [`process-wrap`](https://docs.rs/process-wrap/latest/process_wrap/) 的 `tokio1`/process-group/kill-on-drop features |
| 协作式取消 | [`tokio-util::sync::CancellationToken`](https://docs.rs/tokio-util/latest/tokio_util/sync/struct.CancellationToken.html) |
| 结构化日志与落盘 | `tracing`、`tracing-subscriber`、`tracing-appender` |
| ANSI 清理 | `strip-ansi-escapes` |
| 字节缓冲/非 UTF-8 容错 | `bytes`、`bstr` 或等价成熟 crate |

`process-wrap` 是 `command-group` 的后继实现；优先验证它是否满足 macOS MVP 和未来 Windows Job Object。若实测能力不足，可换用 `command-group` 或其他维护中的库，但不得回退为手写 `setpgid`、signal/PID 遍历或平台 `unsafe` 实现。

## 交付内容

- 将 CommandSpec 转换为 `tokio::process::Command`/`process-wrap`，stdin 默认关闭。
- 使用 Tokio 异步 I/O 分流 stdout/stderr；dbox 只添加业务事件序号和 RunId。
- 使用现成日志/字节 crate 实现有界缓冲、落盘、ANSI/异常编码容错。
- 使用 `CancellationToken`、Tokio timeout/select 和 `process-wrap` 完成取消、kill-on-drop 与进程组清理。
- 实现 Run 状态机：queued/running/succeeded/failed/cancelled/timed_out/interrupted/partial。
- 对常见 secret 环境名和值模式脱敏；复制日志只返回脱敏内容。
- 退出码成功后执行 post-check，区分命令成功与验证成功。
- 将第三方库错误转换为稳定的 dbox 错误 DTO，不复制第三方内部实现。

## 集成测试

- 使用 fixture 假程序模拟 stdout/stderr 交错、退出 0/非 0。
- 超时与取消后子进程/孙进程不残留。
- 超长输出不导致无界内存增长，落盘日志完整。
- 无效 UTF-8 不导致 panic。
- 路径与参数含空格可正确执行。
- 日志中的 TOKEN/KEY/SECRET/PASSWORD 值被脱敏。
- drop、timeout、cancel 三条路径均验证 `process-wrap` 的进程组/kill-on-drop 行为。
- 测试不得执行真实 `brew upgrade`、`npm install -g` 或 `npx skills`。

## 完成标准

全部 fixture 集成测试稳定通过；任何外部命令都只能来自已验证 CommandPlan；执行失败不会使应用进程崩溃。代码审查确认没有手写 async runtime、PID 树、process group、signal 封装或日志轮转框架。

## 非目标

- 不实现任务队列或 Tauri event。
- 不实现通用 subprocess 库；只维护 dbox 到开源库的薄适配层。
