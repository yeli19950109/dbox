# T19：更新确认、运行记录与设置界面

- 状态：Completed
- 阶段：图形界面
- 依赖：T14、T17、T18
- 阻塞：T20

## 目标

完成 CLI 工具管理的端到端桌面交互，并保持后端 UpdatePlan 为唯一执行依据。

## 交付内容

- 单组件、单工具和多选批量更新入口。
- 确认页展示 program、argv、来源、Component、Strategy、cwd、联网提示和风险。
- 只回传 plan_id/hash，不由前端重建命令。
- 实时运行页使用 `@tanstack/vue-virtual` 展示持续增长的队列与 stdout/stderr，并支持耗时、取消、部分失败和重试；不实现 viewport/DOM 回收算法。
- 历史页展示摘要并复制后端返回的脱敏日志。
- 设置页展示 PATH、npm/brew 路径、Provider 开关、超时、日志保留和策略默认值。
- manifest 校验/保存只调用 T15 API，处理 revision 冲突。
- 高频事件使用 VueUse `useThrottleFn`/`useDebounceFn` 或等价维护库控制 UI 刷新；后端事件序号、有界日志和最终状态仍是正确性来源。

## 测试要求

- 过期计划要求重新预览，不能继续执行。
- 批量部分失败、取消、重试、post-check unknown。
- 高频日志事件节流后 UI 不丢最终状态。
- 大日志 fixture 验证虚拟列表 DOM 有界、滚动稳定，结束/失败/取消最终状态不受节流影响。
- 使用 `axe-core` 回归确认运行状态、日志区域和取消/重试操作具有正确可访问名称与焦点行为。
- 前端代码不存在任意 process/shell 执行入口。
- 构建和前端测试通过。

## 完成标准

使用后端 fake fixture 可以从刷新走到更新结果；真实命令永远由已确认的后端计划执行；持续日志没有自研 virtualizer 或通用 throttle/debounce。

## 验证记录

- 完成日期：2026-08-27
- 关键文件：`src/views/ConfirmView.vue`、`src/views/RunsView.vue`、`src/components/RunLog.vue`、`src/views/SettingsView.vue`、`src/stores/runs.ts`、`src/stores/settings.ts`、对应 `*.test.ts`
- 执行命令：`npm test`、`npm run build`、`npm run bindings:check`、`rg -n "plugin-shell|child_process|Deno.Command|Bun.spawn" src -g '!bindings.ts' -g '!*.test.ts' -g '!test/**'`
- 测试结果：覆盖 plan id/hash-only 确认、失效计划重新预览、批量部分失败、取消、重试、post-check unknown、10,000 行虚拟日志 DOM 有界、高频事件不丢行/最终状态、跨 Run 隔离、脱敏日志复制、设置与 manifest revision 冲突，以及运行/日志/取消/重试的 axe-core 可访问性；fake transport 可完成 preview → confirm → result 流程。
- 实现约束：program/argv/cwd/网络风险全部直接展示后端 UpdatePlan；日志正确性来自每 Run 十进制 sequence，VueUse throttle 仅触发绘制；前端不存在 shell/process 执行入口。
- 已知限制：T15 当前 confirm API 返回单项执行结果，批量 UI 按计划顺序确认并汇总，任一执行失败不会阻止后续已生成计划；API 级错误会逐项提示。

## 2026-09-07 实时执行修复

- 确认后立即进入运行页，任务继续在 store 中执行；收到新 Run 事件时自动选择并展示日志，手动选择历史记录后不抢走选择。
- 启动中展示等待状态；运行页保留取消按钮、错误提示及失效计划的重新预览入口，阻止重复提交正在执行的批次。
- 执行前等待日志订阅注册完成，HTTP 确认还会等待 SSE 建立。四类事件共用一条 SSE 连接，服务端立即发送连接帧，避免连接数占用和等待首个保活包。
- 输出按序号独立去重、排序，状态按自己的序号更新，较新的状态不会吞掉延迟到达的输出。
- 后端日志批次增加定时 flush，单条输出后保持安静的命令也会在批次间隔后推送；终态 flush 不重复发送已推送的批次。
- 验证：127 项 Rust 测试、46 项前端测试、Clippy、格式检查及构建通过。隔离的真实 fake-command + Rust API + SSE + Chrome 测试确认：确认请求未完成时已跳转并显示 stdout/stderr，结束状态与取消操作正常，确认请求仍仅包含 plan ID/hash。未通过测试执行真实包管理器更新。
