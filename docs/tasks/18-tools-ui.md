# T18：工具列表、详情与刷新界面

- 状态：Completed
- 阶段：图形界面
- 依赖：T17
- 阻塞：T19、T20

## 目标

展示 Rust 后端已经验证的 npm/brew 全量扫描结果，并让用户清楚理解 Provider、Installation、Executable 和 Component。

## 交付内容

- 工具列表：全部/有更新/未知/失败筛选，Provider 和类别筛选，搜索与用户隐藏。
- 展示包坐标、来源、当前/最新版本、Component 汇总和最后检查时间。
- 详情展示安装路径、多个 executable、Provider 诊断和策略选择。
- 刷新全部、按 Provider、按 Tool；展示部分 Provider 失败。
- 同一产品来自多个 Provider 时分开展示，不自动合并。
- catalog 未收录项使用通用样式，不显示为“不支持”。
- 数百项列表优先使用 `@tanstack/vue-virtual`，不实现 viewport 计算或 DOM 回收；只有性能基准证明普通列表满足目标时才可延迟，并在验证记录中保存数据。
- 通用 debounce/throttle 优先使用 VueUse；前端节流只改善渲染，不能承担事件顺序、去重或最终状态正确性。

## 测试要求

- mock snapshot 覆盖空列表、数百项、未知工具、重复名称和部分失败。
- 版本 unsupported/unknown 不误显示为最新。
- 刷新去重、筛选和隐藏状态测试。
- 使用 `axe-core` 的 Vue 测试集成做基础可访问性回归，并覆盖键盘操作、焦点、对比度和可访问名称。
- 使用数百项 fixture 记录普通列表/虚拟列表的渲染基准和最终选择。

## 完成标准

用户可以浏览全部 npm/brew 安装项并理解其来源与状态；所有写操作仍未通过此页面直接执行；没有项目自研 virtualizer、通用 throttle/debounce 或 accessibility runner。

## 验证记录

- 完成日期：2026-08-27
- 关键文件：`src/views/ToolsView.vue`、`src/components/ToolCard.vue`、`src/utils/presentation.ts`、`src/styles.css`、对应 `*.test.ts`
- 执行命令：`npm test -- src/views/ToolsView.test.ts --reporter=verbose`、`npm test`、`npm run build`
- 测试结果：覆盖空列表、500 项、未知工具、跨 Provider 重名、部分失败、筛选、隐藏项、刷新去重、键盘焦点、可访问名称与 axe-core color-contrast 规则；500 项普通列表为 102.3 ms/500 个卡片，TanStack Virtual 为 29.5 ms/8 个卡片（本机 jsdom 单次基准），因此采用虚拟列表。
- 实现约束：来源限定 Tool 不合并；catalog 未收录项使用通用样式；unknown/unsupported 不显示为“已是最新”；搜索 debounce 使用 VueUse。
- 已知限制：卡片只摘要前两个 Component，完整 Component、策略、安装路径和 executable 在详情面板显示。
