# T09：GUI 环境与可执行文件解析

- 状态：Pending
- 阶段：Rust Provider 基础
- 依赖：T03、T07
- 阻塞：T10、T11、T15、T21、T23–T26

## 目标

解决 macOS GUI 与终端 PATH 不一致问题，让 npm、brew、npx 以及版本管理器中的命令可以被可靠定位。

## 交付内容

- 定义可注入的 `EnvironmentSource` 和 `ExecutableResolver`。
- 解析顺序：用户绝对路径、已验证缓存、应用 PATH、已知 Homebrew 目录、可选登录 shell 环境快照。
- 登录 shell 仅用于采集环境；真正命令仍以绝对 program + argv 直接执行。
- 记录命令来源、验证时间和不可用原因。
- 提供环境诊断 DTO：Node/npm/npx/brew 路径、版本、PATH 条目和冲突项。
- 路径变化或 executable 不再存在时使相关 CommandPlan 失效。

## 单元测试

- 使用注入 PATH 和临时 fake binaries 验证优先级。
- Apple Silicon/Intel Homebrew 默认路径只作为候选，不覆盖用户配置。
- 同名多个 executable 时结果确定并显示冲突。
- 缓存路径失效后回退重新解析。
- shell 快照解析失败不阻断应用 PATH 和用户路径。
- 测试不读取真实 shell profile。

## 完成标准

在完全隔离的临时环境中可以稳定解析 npm、brew、npx 假程序并生成诊断快照；全部测试通过。

## 非目标

- 不修改用户 shell 配置。

