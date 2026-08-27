# T05：可选 Manifest 与 Catalog 合并

- 状态：Pending
- 阶段：Rust 核心
- 依赖：T02、T04
- 阻塞：T12、T13

## 目标

实现版本化 TOML 增强清单。npm/brew 安装项不依赖 manifest 才能出现；manifest 只补充匹配规则、展示信息、Component 和 Strategy。

## 交付内容

- 定义 schema v1 及 Rust serde 类型。
- 实现内置 catalog、用户 `tools.d` 和自动发现 Tool 的合并顺序。
- matcher 支持 Provider ID、包坐标、package kind 和 executable 名。
- 同 ID 数组按子项 ID 合并；标量字段允许用户覆盖。
- 实现校验错误：文件、字段路径、错误类型和可操作提示。
- 未支持的未来 schema version 必须拒绝，不静默降级。

## 单元测试

- 无 manifest 的自动发现 Tool 保持可管理。
- 删除增强清单不会删除底层 Installation。
- 用户覆盖单个 Strategy 时不需要复制整个内置 manifest。
- matcher 多命中、零命中和歧义时行为明确。
- 未知字段策略、重复 ID、非法 regex/命令配置被正确处理。
- TOML round-trip 和现有 Pi 示例均可解析。

## 完成标准

fixture 自动发现结果可以与内置/用户清单合并为确定快照；所有错误包含源文件位置；测试通过。

## 非目标

- 不提供 UI 编辑器。
- 不把 catalog 做成远端服务。

