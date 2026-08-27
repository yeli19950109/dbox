# T02：核心领域模型

- 状态：Completed
- 阶段：Rust 核心
- 依赖：T01
- 阻塞：T03、T04、T05、T13

## 目标

定义与具体包管理器无关的 Provider、Installation、Tool、Executable、Component、Strategy 和 Run 类型，为 npm、Homebrew 及未来 Provider 提供稳定边界。

## 交付内容

- 使用 newtype 定义 `ProviderId`、`InstallationId`、`ToolId`、`ComponentId`、`StrategyId`、`RunId`。
- Installation ID 必须包含 Provider 与源级包坐标，不能只用展示名称或 executable 名。
- 一个 Installation 支持多个 Executable。
- Tool 默认可由一个 Installation 合成，也允许 catalog 增强多个 Component/Strategy。
- 定义来源、scope、安装路径、版本状态和用户隐藏状态。
- 类型实现必要的 serde、相等比较和稳定排序，不在领域层引用 Tauri 类型。

## 单元测试

- npm、brew 同名包生成不同 Installation ID。
- scoped npm package、cask/formula、路径中含空格时 ID 稳定。
- 同一 package 的多个 executable 保留在同一个 Installation。
- Tool 汇总 Component 时不丢失 unknown/failed 状态。
- serde round-trip 不丢字段，非法空 ID 被拒绝。

## 完成标准

领域模块可被纯 Rust 测试构造和序列化；不依赖 npm、brew、文件系统、网络或 Tauri runtime；全部单元测试通过。

## 非目标

- 不实现版本比较、Provider trait 或持久化。
