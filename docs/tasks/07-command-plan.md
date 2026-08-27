# T07：安全命令规格与 UpdatePlan

- 状态：Pending
- 阶段：Rust 执行内核
- 依赖：T02、T04
- 阻塞：T08、T10–T15、T21

## 目标

把所有外部操作建模为可预览、不可变、可验证的 CommandPlan，确保用户看到的命令和实际执行命令一致。

## 交付内容

- 定义 `CommandSpec`：program、args 数组、cwd、env、timeout、允许退出码、联网提示、post-check。
- program 在计划生成时解析为绝对路径；默认禁止 shell、管道、重定向和命令替换。
- 定义 `UpdatePlan`、`plan_id`、`plan_hash`、配置 revision 和过期时间。
- hash 覆盖 program、args、cwd、env 名称/值、strategy、目标 Installation 和 Component。
- 配置、路径或版本快照变化时计划失效。
- 提供安全的人类可读命令展示，但展示字符串绝不反向参与执行。

## 单元测试

- 参数含空格、引号、分号、`$()`、反引号时仍是单独 argv，不触发 shell 语义。
- 任一计划字段变化都会改变 hash。
- 相同输入生成稳定 hash；过期/配置 revision 变化被拒绝。
- 相对 program、空 program、非法 cwd 和 NUL 参数被拒绝。
- env secret 在展示层脱敏，但 hash/执行值保持正确。

## 完成标准

CommandPlan 可纯 Rust 生成和验证；测试证明不使用 shell 字符串，并能阻止过期或被篡改计划。

## 非目标

- 不启动真实子进程。

