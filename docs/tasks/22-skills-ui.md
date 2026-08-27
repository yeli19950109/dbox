# T22：Skill 管理界面

- 状态：Future
- 阶段：后续图形界面
- 依赖：T17、T21
- 阻塞：无

## 目标

为 `npx skills` 薄适配提供桌面界面，不绕过 CLI 或维护第二份 Skill 状态。

## 交付内容

- 已安装 Skill 列表，按 global/project/Agent 筛选。
- 搜索/查看 source 中的 Skills。
- add/check/update/remove 的命令预览、确认、实时日志与结果。
- project 操作必须先选择并显示 workspace。
- JSON 不可用时展示原始 CLI 输出和 unknown 状态。
- 显示 `DISABLE_TELEMETRY=1` 与 `npx` 可能联网下载的说明。

## 测试要求

- 使用 mock `npx skills` DTO 覆盖列表、部分失败、未知输出和版本不兼容。
- UI 不直接读取 Skill 文件或 lock 文件。
- 改变状态的操作必须经过后端 CommandPlan。

## 完成标准

用户可以通过 `npx skills` 完成常用操作；CLI 不支持的能力在 UI 中明确不可用，不做文件系统 workaround。

