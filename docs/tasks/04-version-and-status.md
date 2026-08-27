# T04：版本比较与状态汇总

- 状态：Pending
- 阶段：Rust 核心
- 依赖：T02
- 阻塞：T10–T14

## 目标

可靠表示当前版本、最新版本、不可比较版本和组件/工具汇总状态，禁止用字符串字典序猜测升级关系。

## 交付内容

- 定义 `VersionValue`、`VersionSource`、`ComparisonResult` 和检查时间。
- SemVer 优先比较，允许保留原始前缀/后缀用于展示。
- 支持 `unknown`、`unsupported`、`check_failed`，并保留结构化原因。
- 实现 Component → Tool 的状态汇总规则。
- 明确更新命令成功但 post-check 未变化时的 `verification_unknown`/`verification_failed`。

## 单元测试

- `1.10.0 > 1.9.0`，证明不是字符串比较。
- `v1.2.3`、pre-release、build metadata 和非 SemVer 输入。
- 无 latest version 时返回 unknown，不误报 up-to-date。
- 任一组件 update_available 时 Tool 有更新。
- 一部分成功、一部分 check_failed 时保留 partial/unknown。
- 非法状态跳转被拒绝。

## 完成标准

版本和状态逻辑为纯函数/纯类型，测试覆盖全部状态分支，并被 npm/brew Provider 复用。

## 非目标

- 不执行远端版本查询。

