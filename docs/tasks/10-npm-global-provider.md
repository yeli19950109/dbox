# T10：npm Global Provider

- 状态：Completed
- 阶段：Rust Provider
- 依赖：T03、T04、T07–T09
- 阻塞：T12、T13、T16

## 目标

扫描并管理 npm 返回的全部全局安装项，不依赖 catalog 白名单。

## 交付内容

- probe npm 路径、版本、global prefix/root。
- 通过稳定 npm 命令获取全部 global top-level package 及当前版本。
- 读取已安装 package metadata 以关联一个或多个 `bin`，不执行 lifecycle script。
- 为每个 package 生成来源限定 Installation ID、通用 Tool 和 core Component。
- 批量查询 latest/outdated，限制并发并缓存，离线时保留 installed 状态。
- 生成单包更新 CommandPlan，包坐标作为独立 argv，禁止拼接 shell。
- 正确处理 scoped package、npm 自身、extraneous/missing/invalid 项和多 Node prefix。

## 单元与 fixture 测试

- 空列表、普通包、scoped package、多 bin、缺失 bin。
- 未收录 catalog 的随机包仍生成可更新 Tool。
- 两个 npm prefix 的同名包 Installation ID 不碰撞或明确标记来源。
- outdated、up-to-date、registry 不可用和畸形 JSON。
- 更新计划严格定位一个 package，不生成全局无范围更新。
- 所有命令都由 fake npm 捕获，测试不改真实 global package。

## 完成标准

fixture npm 可以端到端完成 scan → check → plan；所有安装项均被保留；错误结构化；测试通过。

## 非目标

- 不管理项目 local dependencies。
- 不实现 npm package 安装商店。

## 验证记录

- 完成日期：2026-08-27
- 关键文件：`src-tauri/src/providers/npm_global.rs`、`src-tauri/tests/npm_global_provider.rs`、`src-tauri/tests/fixtures/fake-npm.sh`
- 执行命令：`cargo fmt --check`、`cargo clippy --all-targets --all-features -- -D warnings`、`cargo test`
- 测试结果：fake npm 端到端覆盖 probe → scan → check → plan，以及空列表、scoped package、多 bin、缺失 bin、npm 自身、异常条目、缓存、离线、畸形 JSON、多 prefix 和单包 argv。
- 已知限制：每个 Provider 实例绑定一个已解析的 npm executable/global root；多个 Node prefix 通过各自来源指纹保持 Installation ID 不碰撞。
