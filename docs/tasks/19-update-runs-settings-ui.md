# T19：更新确认、运行记录与设置界面

- 状态：Blocked until T16
- 阶段：图形界面
- 依赖：T14、T17、T18
- 阻塞：T20

## 目标

完成 CLI 工具管理的端到端桌面交互，并保持后端 UpdatePlan 为唯一执行依据。

## 交付内容

- 单组件、单工具和多选批量更新入口。
- 确认页展示 program、argv、来源、Component、Strategy、cwd、联网提示和风险。
- 只回传 plan_id/hash，不由前端重建命令。
- 实时运行页展示队列、stdout/stderr、耗时、取消、部分失败和重试。
- 历史页展示摘要并复制后端返回的脱敏日志。
- 设置页展示 PATH、npm/brew 路径、Provider 开关、超时、日志保留和策略默认值。
- manifest 校验/保存只调用 T15 API，处理 revision 冲突。

## 测试要求

- 过期计划要求重新预览，不能继续执行。
- 批量部分失败、取消、重试、post-check unknown。
- 高频日志事件节流后 UI 不丢最终状态。
- 前端代码不存在任意 process/shell 执行入口。
- 构建和前端测试通过。

## 完成标准

使用后端 fake fixture 可以从刷新走到更新结果；真实命令永远由已确认的后端计划执行。

