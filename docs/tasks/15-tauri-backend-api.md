# T15：Tauri 后端 API 与事件边界

- 状态：Pending
- 阶段：Rust 接口层
- 依赖：T09、T13、T14
- 阻塞：T16–T19、T21、T22

## 目标

把已测试的 Rust 应用服务暴露为粗粒度、类型化 Tauri API；在此阶段仍不开发正式 Vue 界面。

## 交付内容

- commands：snapshot、refresh、preview、confirm、cancel、run history、settings、manifest validation/save。
- 禁止暴露万能 `run_command(program, args)`。
- DTO 与领域类型分离，serde 字段、错误码和可选字段稳定。
- events：refresh progress、run state、run output、tool state。
- 对高频输出做批量/节流，事件带 RunId 和单调序号。
- capability 只允许调用明确 commands；前端不能直接访问 shell/process。
- command 层只做转换和授权，不复制业务逻辑。

## 单元与契约测试

- DTO JSON snapshot/golden test，防止无意破坏前端契约。
- 每个 command 的成功、not-found、invalid-plan、conflict 和内部错误映射。
- 事件序号有序、跨 Run 不串流。
- 非法 ToolId/RunId/plan hash 在进入执行器前被拒绝。
- 检查 Tauri capability 中没有任意 shell 执行权限。

## 完成标准

Rust 测试可以直接调用 command handler 或其薄封装验证完整契约；不打开窗口也能通过全部测试。

## 非目标

- 不实现 Vue store、页面或视觉交互。

