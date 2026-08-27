# T03：Provider 接口与注册表

- 状态：Pending
- 阶段：Rust 核心
- 依赖：T02
- 阻塞：T09、T10、T11、T13、T23–T26

## 目标

用可注册的 `ToolProvider` 接口隔离包管理器差异，避免在应用服务中堆积 npm/brew/mise/rustup 分支。

## 交付内容

- 定义 `ToolProvider`：`id`、`capabilities`、`probe`、`scan`、`check_updates`、`plan_update`。
- 定义 `ProviderCapabilities`：全量扫描、检查更新、更新、版本固定、多版本、executable、层级 Component 等。
- 实现 Provider Registry：注册、按 ID 获取、列出、启停、重复 ID 拒绝。
- 定义结构化 Provider 错误，保留 provider_id、operation、可恢复性和脱敏摘要。
- 提供 `FakeProvider` 测试工具，后续测试不得依赖真实包管理器。

## 单元测试

- 重复 Provider ID 注册失败且不覆盖旧值。
- capability 能正确限制不支持的操作。
- 单个 Provider probe/scan 失败不会污染其他注册项。
- registry 顺序稳定，启停设置可预测。
- fake provider 可模拟成功、空结果、失败和延迟。

## 完成标准

可以只靠 FakeProvider 完成注册、扫描和生成计划的单元测试；主流程无需知道 npm/brew 的具体类型；测试通过。

## 非目标

- 不实现任何真实 Provider。

