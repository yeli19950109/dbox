# T15：Tauri 后端 API 与事件边界

- 状态：Pending
- 阶段：Rust 接口层
- 依赖：T09、T13、T14
- 阻塞：T16–T19、T21、T22

## 目标

把已测试的 Rust 应用服务暴露为粗粒度、类型化 Tauri API；在此阶段仍不开发正式 Vue 界面。

实现前遵循 [`ADR 0002`](../adr/0002-rust-typescript-contract-generation.md) 的已验证方案；若锁定版本无法继续满足 Tauri 2 或项目 toolchain，先更新 spike 与 ADR，不回退到手工镜像 DTO。

## 交付内容

- commands：snapshot、refresh、preview、confirm、cancel、run history、settings、manifest validation/save。
- 禁止暴露万能 `run_command(program, args)`。
- DTO 与领域类型分离，serde 字段、错误码和可选字段稳定。
- 仅在公开 API DTO 上派生 Specta 类型；内部领域类型和 persistence document 不直接生成到前端。
- 使用精确锁定的 `tauri-specta`/`specta` 版本生成 command、event 和 TypeScript bindings；生成文件带“禁止手改”标记并提交仓库。
- TypeScript 禁止手工维护 Rust DTO 的镜像 interface；mock transport 也必须复用生成类型。
- Rust `u64`/`i64` 等可能超过 JavaScript 安全整数的值在 API DTO 中使用十进制字符串，或经明确上界证明后收窄为 `u32`；不得开启有损 BigInt-to-number 导出。
- events：refresh progress、run state、run output、tool state。
- 对高频输出做批量/节流，事件带 RunId 和单调序号。
- capability 只允许调用明确 commands；前端不能直接访问 shell/process。
- command 层只做转换和授权，不复制业务逻辑。

## 单元与契约测试

- DTO JSON snapshot/golden test，防止无意破坏前端契约。
- 固定命令生成 bindings，并在 CI 重新生成后执行无 diff 检查；生成文件通过项目 TypeScript 编译。
- 每个 command 的成功、not-found、invalid-plan、conflict 和内部错误映射。
- 事件序号有序、跨 Run 不串流。
- 非法 ToolId/RunId/plan hash 在进入执行器前被拒绝。
- 检查 Tauri capability 中没有任意 shell 执行权限。

## 完成标准

Rust 测试可以直接调用 command handler 或其薄封装验证完整契约；不打开窗口也能通过全部测试；bindings 只能由固定命令更新，golden wire format 和生成结果检查同时通过。

## 非目标

- 不实现 Vue store、页面或视觉交互。
