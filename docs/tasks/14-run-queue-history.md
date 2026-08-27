# T14：批量队列、运行历史与恢复

- 状态：Completed
- 阶段：Rust 应用层
- 依赖：T06、T08、T13
- 阻塞：T15、T16、T19

## 目标

为单项和批量更新提供默认串行、可取消、可恢复且结果可追溯的运行队列。

## 交付内容

- 默认全局并发 1；同一 Tool 永远互斥。
- 批量更新拆成独立 Run item，一个失败不阻断后续项。
- 支持取消 queued/running 项、重试失败项和批量汇总。
- 持久化状态转移、摘要和日志引用。
- 应用重启后把未结束 Run 标为 interrupted，并触发相关 Tool 重新检查。
- 为未来按 Provider 互斥组并行保留调度策略接口，MVP 不启用。

## 单元与集成测试

- 20 个 fake task 串行完成且顺序确定。
- 第 N 项失败后 N+1 继续，最终结果为 partial。
- queued 取消不启动进程；running 取消清理进程。
- 同一 Tool 的两个任务不会并行。
- 重启恢复把 running 变为 interrupted，不重复执行。
- 重试生成新 RunId，并关联原 Run。

## 完成标准

压力 fixture 不死锁、不泄漏任务；所有状态可从持久化重建；测试通过。

## 非目标

- MVP 不并行执行 npm/brew 更新。

## 验证记录

- 完成日期：2026-08-27
- 关键文件：`src-tauri/src/application/run_queue.rs`、`src-tauri/src/domain/mod.rs`、`src-tauri/tests/run_queue_history.rs`、`src-tauri/tests/fixtures/fake-queue-command.sh`
- 执行命令：`cargo fmt --check`、`cargo clippy --all-targets --all-features -- -D warnings`、`cargo test`
- 测试结果：20 项压力 fixture 串行且顺序确定；覆盖失败后继续、批量 Partial 汇总、queued/running 取消、进程树清理、同 Tool 互斥、重启 interrupted、历史重建、日志引用和 retry RunId 关联。
- 已知限制：MVP 只接受并发度 1 的调度策略；Provider 互斥组和并行策略接口已保留，正式启用由后续任务完成。
