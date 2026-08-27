# 前端依赖治理记录

> 检查日期：2026-08-27  
> 基线：T17–T19  
> 本地验证：Node 24.19.0、npm 12.0.2

所有直接依赖都在 `package.json` 使用精确版本，完整传递依赖由 `package-lock.json` 锁定。版本与许可证在加入时通过 npm 官方 registry 元数据核对；维护状态以 2026-08-27 能取得当前正式版本、上游项目持续维护为准。

| 依赖 | 精确版本 | 许可证 | Node 要求 | 必要性与使用边界 |
| --- | --- | --- | --- | --- |
| `vue` | 3.5.42 | MIT | 项目基线 | 组件运行时；不引入第二套 UI 框架 |
| `vue-router` | 5.3.0 | MIT | 项目基线 | Tools、Runs、Settings、Skills 与确认页路由 |
| `pinia` | 4.0.3 | MIT | 项目基线 | snapshot、运行事件/队列、设置与通知状态 |
| `@tauri-apps/api` | 2.11.1 | Apache-2.0 OR MIT | 项目基线 | 只由生成 bindings 使用的 Tauri transport |
| `@tanstack/vue-virtual` | 3.13.36 | MIT | 项目基线 | 500 项工具列表与持续增长日志的 DOM 回收 |
| `@vueuse/core` | 14.4.0 | MIT | 项目基线 | 搜索 debounce、日志 paint throttle 与运行耗时刷新 |
| `vitest` | 4.1.11 | MIT | `^20.0.0 \|\| ^22.0.0 \|\| >=24.0.0` | 前端测试 runner |
| `@vue/test-utils` | 2.5.0 | MIT | 项目基线 | Vue 组件与交互测试 |
| `axe-core` | 4.13.0 | MPL-2.0 | `>=4` | 组件可访问性、可访问名称及 color-contrast 规则回归 |
| `jsdom` | 30.0.1 | MIT | `^22.22.2 \|\| ^24.15.0 \|\| >=26.0.0` | Vitest DOM 环境；只属于开发依赖 |

现有构建工具 `Vite` 8 将项目 Node 基线确定为 `^20.19.0 || ^22.12.0 || >=24.0.0`；`jsdom` 30 的要求更严格，因此执行前端测试时使用 Node `^22.22.2 || ^24.15.0 || >=26.0.0`。生产包不包含 Vitest、Vue Test Utils、axe-core 或 jsdom。

## 验证

- `npm list --depth=0`：所有直接依赖版本与 `package.json` 一致。
- `npm audit`：0 vulnerabilities。
- `npm run build`：生成 bindings 及所有应用/测试 TypeScript 均通过类型检查。
- 未加入 shell/process、viewport 算法、router、store、debounce/throttle 或 accessibility runner 的自研替代。
