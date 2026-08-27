# T13：刷新、检查与更新应用服务

- 状态：Pending
- 阶段：Rust 应用层
- 依赖：T03–T12
- 阻塞：T14–T16

## 目标

编排 Provider、catalog、缓存和执行器，形成不依赖图形界面的完整业务用例。

## 交付内容

- `refresh_tools`：probe Provider、扫描安装项、合成 Tool、应用增强、检查更新并写缓存。
- Provider 级错误隔离：一个失败不阻断其他 Provider。
- 对重复刷新做去重，支持全量和按 Tool/Provider 刷新。
- `preview_updates`：根据选择和策略生成不可变计划。
- `confirm_update`：验证 plan revision/hash 后交给执行层。
- 完成后执行 post-check 并更新 Tool 状态。
- 返回应用快照 DTO，不向上暴露内部锁或 Provider 实现。

## 单元测试

- 多 FakeProvider 同时成功、部分失败、空结果和延迟。
- catalog 增强应用顺序确定。
- refresh 去重且强制刷新可绕过缓存。
- 配置/路径改变使旧计划失效。
- 命令成功但 post-check 失败时不误报 upgraded。
- 未收录 catalog 的 npm/brew Tool 完整经过应用流程。

## 完成标准

通过纯 Rust FakeProvider 测试完成 snapshot → refresh → preview → confirm → post-check；无 Tauri/前端依赖。

## 非目标

- 不暴露 Tauri command。
- 不实现正式并发队列。

