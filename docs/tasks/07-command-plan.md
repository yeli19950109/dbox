# T07：基于开源库的命令规格与 UpdatePlan

- 状态：Pending
- 阶段：Rust 执行内核
- 依赖：T02、T04
- 阻塞：T08、T10–T15、T21

## 目标

所有操作最终都是命令行调用。本任务只定义 dbox 必需的薄领域模型，把序列化、ID、hash、secret 和命令展示委托给成熟开源 crate，避免自研一套命令 DSL 或加密/转义实现。

## 开源库优先方案

实现前先提交简短 ADR，检查许可证、维护状态、MSRV、Tauri/Tokio 兼容性和平台支持。优先采用：

| 能力 | 优先库 | dbox 只保留的逻辑 |
| --- | --- | --- |
| 命令底层表示 | `std::process::Command` / `tokio::process::Command` | Tool/Component/Strategy 到 argv 的映射 |
| DTO 序列化 | `serde`、`serde_json` | dbox schema |
| Plan ID | `uuid` | ID 生命周期 |
| Plan hash | `blake3` | 选择纳入 hash 的 dbox 字段 |
| Secret 包装 | `secrecy` | 哪些字段属于敏感信息 |
| 展示用参数转义 | [`shell-quote`](https://docs.rs/shell-quote/latest/shell_quote/)；实现 spike 可与 `shlex` 比较 | 仅生成可复制展示；结果绝不参与执行 |
| 声明式校验 | 优先评估 `garde` 或 `validator` | 少量跨字段业务约束 |

不得自行实现 hash 算法、shell quoting、secret 容器或通用命令 builder。若某库不采用，ADR 必须说明缺失能力，并把自定义代码限制在最薄适配层。

## 交付内容

- 定义薄 `CommandSpec` DTO：program、args 数组、cwd、env、timeout、允许退出码、联网提示、post-check；执行时直接转换为标准/Tokio Command。
- program 在计划生成时解析为绝对路径；默认禁止 shell、管道、重定向和命令替换。
- 定义 `UpdatePlan`、`plan_id`、`plan_hash`、配置 revision 和过期时间。
- hash 覆盖 program、args、cwd、env 名称/值、strategy、目标 Installation 和 Component。
- 配置、路径或版本快照变化时计划失效。
- UI 默认分别展示 program 和 argv；可复制的单行命令使用选定 quoting crate 生成，但结果绝不反向参与执行。

## 单元测试

- 参数含空格、引号、分号、`$()`、反引号时仍是单独 argv，不触发 shell 语义。
- 任一计划字段变化都会改变 hash。
- 相同输入生成稳定 hash；过期/配置 revision 变化被拒绝。
- 相对 program、空 program、非法 cwd 和 NUL 参数被拒绝。
- env secret 在展示层脱敏，但 hash/执行值保持正确。
- 用选定 quoting crate 的公开测试向量覆盖空参数、空格、Unicode、引号和控制字符，不复制其内部算法。

## 完成标准

CommandPlan 可纯 Rust 生成和验证；测试证明不使用 shell 字符串，并能阻止过期或被篡改计划。ADR 与 `Cargo.lock` 明确记录采用的开源库；dbox 自定义部分只包含领域字段、跨字段约束和计划生命周期。

## 非目标

- 不启动真实子进程。
- 不实现通用命令执行框架、shell parser/escaper、hash 或 secret 基础设施。
