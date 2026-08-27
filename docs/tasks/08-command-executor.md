# T08：命令执行器、日志、超时与取消

- 状态：Pending
- 阶段：Rust 执行内核
- 依赖：T06、T07
- 阻塞：T10–T16、T21

## 目标

执行已经确认的 CommandPlan，并提供可靠的状态机、stdout/stderr 流、超时、取消、进程树清理和脱敏日志。

## 交付内容

- 直接 spawn 绝对 program + argv，stdin 默认关闭。
- 分流读取 stdout/stderr，生成有序序号和时间戳事件。
- 支持最大内存缓冲、完整落盘、ANSI/异常编码容错。
- 支持 timeout、用户取消、graceful termination 和最终强制清理子进程树。
- 实现 Run 状态机：queued/running/succeeded/failed/cancelled/timed_out/interrupted/partial。
- 对常见 secret 环境名和值模式脱敏；复制日志只返回脱敏内容。
- 退出码成功后执行 post-check，区分命令成功与验证成功。

## 集成测试

- 使用 fixture 假程序模拟 stdout/stderr 交错、退出 0/非 0。
- 超时与取消后子进程/孙进程不残留。
- 超长输出不导致无界内存增长，落盘日志完整。
- 无效 UTF-8 不导致 panic。
- 路径与参数含空格可正确执行。
- 日志中的 TOKEN/KEY/SECRET/PASSWORD 值被脱敏。
- 测试不得执行真实 `brew upgrade`、`npm install -g` 或 `npx skills`。

## 完成标准

全部 fixture 集成测试稳定通过；任何外部命令都只能来自已验证 CommandPlan；执行失败不会使应用进程崩溃。

## 非目标

- 不实现任务队列或 Tauri event。

