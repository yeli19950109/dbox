# T12：Catalog 增强与 Pi 多组件示例

- 状态：Completed
- 阶段：Rust 功能验证
- 依赖：T05、T10、T11
- 阻塞：T13、T16

## 目标

证明 catalog 只是通用 Provider 结果的增强层，并用 Pi 验证一个工具多个 Component、多个更新 Strategy 的设计。

## 交付内容

- 添加 Pi 内置增强 manifest，以 npm 包坐标和 executable matcher 匹配已有 Installation。
- core 提供 self 与 npm global 策略；extensions 提供 `pi update --extensions`。
- 不为 `pi update --all` 建立会与 core/extensions 重复执行的默认批量项。
- extensions 无可靠版本源时标记 `version_tracking = unsupported`。
- 添加两个未知名称 fixture，证明无 catalog 的 npm/brew Tool 不受影响。
- catalog 加载失败只影响对应增强项，不删除 Provider 基础结果。

## 单元测试

- Pi npm Installation 被增强而不是复制为第二个 Tool。
- core 策略切换后持久选择准确。
- extensions 单独生成正确 argv。
- core + extensions 批量计划不重复包含 `--all`。
- matcher 不命中时普通 Pi 安装记录仍存在。
- 未知 npm/brew fixture 仍有通用 Provider 更新计划。

## 完成标准

纯 fixture 测试证明“全部通用工具 + 少量特殊增强”模型成立；不需要机器安装 Pi。

## 非目标

- Pi 不是产品白名单或 MVP 唯一支持对象。

## 验证记录

### 2026-10-13：扩展更新支持调整

旧实现仅提供 `extensions` 占位组件和 `pi update --extensions` 策略，
`version_tracking = unsupported`，并未发现或检查实际扩展包。
现在内置 `pi-extensions` Provider 将 Pi 用户级托管 npm 包分别建为 Installation/Tool，
复用 Provider 的刷新、预览、更新与安装版本复查；生产 manifest 移除旧占位组件，
避免 Pi Core 因扩展未知状态一直显示「部分未知」。上述 T12 验证记录保留为原始实现记录。

范围为 Pi agent 目录下声明并托管的 npm 包；按原始版本范围或 dist-tag 查询可用版本，
精确版本不自动更新。更新单个包使用 `pi update --extension <source> --no-approve`，
项目配置、Git/本地包和旧全局 npm 安装不纳入该 Provider。

依据：[Pi package commands](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/cli.md#package-commands)
及本机 Pi 1.2.0 的 package-manager 实现。

### 原始实现记录（2026-08-27）

- 完成日期：2026-08-27
- 关键文件：`src-tauri/resources/catalog/pi.toml`、`src-tauri/src/catalog/mod.rs`、`src-tauri/tests/catalog_pi_enrichment.rs`
- 执行命令：`cargo fmt --check`、`cargo clippy --all-targets --all-features -- -D warnings`、`cargo test`
- 测试结果：fixture 证明 Pi 只增强已有 npm Installation；core 支持 self/npm global，extensions 独立使用 `--extensions` 且不生成 `--all`；无匹配和 catalog 文件损坏均保留通用工具。
- 已知限制：内置 matcher 使用当前文档中的 npm 包坐标；发行坐标变化时通过内置 catalog 升级或用户 manifest 覆盖。
