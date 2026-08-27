# T17：前端壳层与类型化 API

- 状态：Completed
- 阶段：图形界面
- 依赖：T16
- 阻塞：T18、T19、T22

## 目标

在 Rust 后端门禁通过后，建立最小 Vue 应用壳层和类型化 Tauri API，不重新实现后端业务规则。

## 交付内容

- 删除模板欢迎 UI，使用 Vue Router 建立 Tools、Runs、Settings 以及未来 Skills 的导航与路由；不实现自有路由器。
- 直接消费 T15 生成的 `tauri-specta` command/event bindings；禁止手工维护镜像 DTO interface 或另一套 invoke 字符串清单。
- 使用 Pinia 建立 snapshot、运行队列和设置 stores，状态来源仅为后端 snapshot/events；dbox 只实现 store action、事件去重和错误展示策略。
- 建立全局 loading、空状态、错误边界和通知基础组件。
- 使用 Vitest + Vue Test Utils 建立组件/store 测试；mock Tauri transport 模拟生成的 bindings，不另建接口模型，也不依赖真实包管理器。
- 前端依赖固定版本并记录许可证、维护状态、Node 版本要求和必要性，不启用无关能力。

## 测试要求

- DTO 映射、路由、store 初始化和事件去重单测。
- 过期事件、跨 Run 输出和未知后端字段不会破坏页面。
- 重新生成 bindings 后仓库无 diff；生成文件通过 TypeScript 检查。
- `npm run build`、Vitest 及 Vue Test Utils 测试通过。

## 完成标准

应用可以连接 Fake/真实 Tauri API 展示空快照和导航；没有工具管理细节 UI；不在 TypeScript 复制 DTO、版本比较或计划校验逻辑；没有自研 router/store/test runner。

## 验证记录

- 完成日期：2026-08-27
- 关键文件：`src/router/index.ts`、`src/api/transport.ts`、`src/stores/`、`src/components/AppLoading.vue`、`src/components/AppEmptyState.vue`、`src/components/AppErrorBoundary.vue`、`src/components/NotificationCenter.vue`、`src/test/`、`docs/frontend-dependencies.md`
- 执行命令：`npm run bindings:check`、`npm run build`、`npm test`、`npm audit`
- 测试结果：Vue Router、Pinia 初始化、生成 DTO 映射、十进制事件序号、过期/重复事件、跨 Run 隔离、未知后端字段和全局状态组件均由 Vitest/Vue Test Utils 覆盖；全部前端测试通过。
- 依赖治理：直接依赖全部使用精确版本；许可证、维护状态、Node 要求与必要性记录在 `docs/frontend-dependencies.md`。
- 已知限制：浏览器单独打开 Vite 页面没有 Tauri transport；正式运行路径为 Tauri WebView，测试使用与生成 bindings 同类型的 fake transport。
