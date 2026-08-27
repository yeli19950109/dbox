# T17：前端壳层与类型化 API

- 状态：Blocked until T16
- 阶段：图形界面
- 依赖：T16
- 阻塞：T18、T19、T22

## 目标

在 Rust 后端门禁通过后，建立最小 Vue 应用壳层和类型化 Tauri API，不重新实现后端业务规则。

## 交付内容

- 删除模板欢迎 UI，建立 Tools、Runs、Settings 导航与路由。
- 建立类型化 invoke/event 封装，DTO 对齐 T15 schema revision。
- 建立 stores，状态来源仅为后端 snapshot/events。
- 建立全局 loading、空状态、错误边界和通知基础组件。
- 测试环境使用 mock Tauri transport，不依赖真实包管理器。

## 测试要求

- DTO 映射、路由、store 初始化和事件去重单测。
- 过期事件、跨 Run 输出和未知后端字段不会破坏页面。
- `npm run build` 及前端测试通过。

## 完成标准

应用可以连接 Fake/真实 Tauri API 展示空快照和导航；没有工具管理细节 UI；不在 TypeScript 复制版本比较或计划校验逻辑。

