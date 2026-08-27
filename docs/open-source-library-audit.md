# dbox 开源库复用审计建议

> 状态：Accepted；建议已同步到任务约束，按任务依赖逐项实现
>
> 审计日期：2026-08-27
>
> 审计基线：`a18eb17`
>
> 范围：`docs/development-plan.md`、T01–T26、当前 Rust/前端依赖与已实现代码

## 1. 目的

本文件集中记录“哪些基础设施应优先复用开源库，哪些逻辑仍应由 dbox 自己实现”。它不替代任务文件，也不表示候选依赖已经获准加入项目。

采用新依赖前必须检查：

- 许可证与项目分发方式兼容；
- 维护状态、最近发布和安全记录；
- MSRV 与 dbox/Tauri toolchain 兼容；
- macOS MVP 和未来平台支持；
- 是否能减少实质性代码，而不是只把少量业务逻辑包装进更重的框架；
- 是否能通过最小 features 控制依赖面；
- 是否有 fixture/集成测试可以锁定实际行为。

若成熟库已经满足需求，任务不得改写为自研实现。若现有库不匹配，需在 ADR 中记录验证结果和保留自定义代码的原因。

## 2. 审计结论

### 2.1 已满足开源库优先原则

以下部分已经采用合适的开源基础设施，不需要重复整改：

| 任务/能力 | 当前采用 | 结论 |
| --- | --- | --- |
| T03 Provider async/error | `async-trait`、`thiserror` | 已满足；Registry 和 capability 是 dbox 业务逻辑 |
| T04 版本比较 | `semver` | 已满足；非 SemVer/Provider 状态仍需领域规则 |
| T05 TOML 与诊断 | `serde`、`toml`、`serde_path_to_error`、`regex` | 已满足基础解析；按子项 ID 合并属于 dbox 语义 |
| T07 CommandPlan | `serde`、`uuid`、`blake3`、`secrecy`、`shell-quote` | 已由 ADR 0001 约束为薄领域 DTO |
| T08 命令执行 | `tokio`、`process-wrap`、`CancellationToken`、`bytes`、`bstr`、`tracing` | 已满足；不得重新加入自研 PID/process-group 实现 |
| T09 GUI PATH | Tauri `fix-path-env-rs`、`which`、Tauri `PathResolver` | 已满足；不得解析 shell profile |
| T06 原子写入 | `atomic-write-file` 0.3.1 | 已替换自有临时文件/rename 算法；schema、revision 与 retention 仍属 dbox |
| T10/T11 Provider 并发 | Tokio `Semaphore`、`JoinSet`，通过 npm/brew CLI 获取数据 | 已满足；CLI JSON 到领域 DTO 的映射必须保留 |
| T13 刷新服务 | Tokio `Mutex`/`RwLock` 与简单 TTL 快照 | 当前规模可接受，见 4.2 的升级条件 |
| T14 队列 | Tokio `Mutex`/`Notify`、`CancellationToken`，复用 T08 executor | 当前串行、可持久化业务队列不需要引入通用任务平台 |
| T21 Skill | 只调用 `npx skills` | 已满足；禁止读取或修改 Skill 内部目录/lock 文件 |

主要参考：[`semver`](https://docs.rs/semver/latest/semver/)、[`process-wrap`](https://docs.rs/process-wrap/latest/process_wrap/)、[`tokio-util::CancellationToken`](https://docs.rs/tokio-util/latest/tokio_util/sync/struct.CancellationToken.html)、[`which`](https://docs.rs/which/latest/which/)。

### 2.2 处理状态

| 优先级 | 任务 | 已处理 | 后续实现点 |
| --- | --- | --- | --- |
| P0 | T06 | 选定并集成精确版本 `atomic-write-file`；补充 discard、覆盖、权限和 commit 失败测试 | 平台 CI 持续验证非 Unix 行为 |
| P0 | T15/T17 | 完成 `tauri-specta` command/event spike 并接受 [ADR 0002](adr/0002-rust-typescript-contract-generation.md)；任务禁止镜像 DTO | T15 实现时加入锁定依赖和生成命令 |
| P1 | T17 | 任务已明确 Vue Router、Pinia、Vitest、Vue Test Utils | T16 完成后实施 |
| P1 | T18/T19 | 任务已明确 `@tanstack/vue-virtual`、VueUse、`axe-core` 与性能例外条件 | T17/T18 实施时加入依赖和基准 |
| P1 | T20 | 任务已限制为 Tauri CLI/bundler/updater 与官方 CI/action | 发布阶段实施 |
| P1 | T16 | 已加入 `cargo-deny`、`cargo-audit`、Git dependency/feature policy；`npm audit` 先报告后决策 | T16 提交配置和 CI 证据 |
| P2 | 全部测试 | 原则保留 | 出现重复 fixture/snapshot/性质测试代码时按需引入，不为尚不存在的重复提前加依赖 |

## 3. 必须调整的任务建议

### 3.1 T06：持久化与原子写入

`src-tauri/src/persistence/mod.rs` 已保留极薄 `atomic_write` adapter，并把临时文件、同步、rename、失败清理和 Unix 父目录同步交给精确锁定的 `atomic-write-file` 0.3.1。

处理结果：

1. [`atomic-write-file`](https://docs.rs/atomic-write-file/latest/atomic_write_file/) 负责同目录临时文件，因此从结构上避免跨设备 rename；Unix 覆盖时支持 mode/owner 保留尝试，并在 commit 后同步父目录。
2. `tempfile::NamedTempFile::persist` 未选中，因为文件及父目录 durability 仍需项目自行补齐。
3. dbox 继续负责 revision/etag、schema、错误 DTO、存储路径和 retention policy。
4. 测试已覆盖 discard/模拟中断、目标已存在、只读目录、Unix mode 和 commit 失败；Windows 行为留给后续平台 CI。

日志轮转不建议直接套用通用单文件 rotate crate。dbox 当前是一 Run 一 JSONL，并且需要跨文件总大小、总数量、活动 Run 排除和历史引用一致性；这属于项目 retention policy。只有将来切换为单一连续日志文件时，再评估 [`file-rotate`](https://docs.rs/file-rotate/latest/file_rotate/)。

T06 已据此改写为“集成并验证原子写入库，dbox 实现 schema/revision/retention policy”，不再要求实现原子文件算法。

### 3.2 T15/T17：Rust 到 TypeScript 的契约生成

T15/T17 已明确自动生成类型和 invoke/event bindings，并接受 [ADR 0002](adr/0002-rust-typescript-contract-generation.md)。

已完成的 spike：

1. `tauri-specta` 2.0.0-rc.25 的 command、typed error 和 event bindings 在 Tauri 2.11.5/Rust 1.97.1 上生成成功，并通过 TypeScript 6.0.2 严格检查。
2. Spike 发现 `u64` 会因 JavaScript 精度风险被拒绝；API DTO 必须改用十进制字符串或经证明有界的 `u32`，不得开启有损导出。
3. 只从明确的 API DTO 生成 TypeScript，不从内部领域类型或 persistence document 直接生成。
4. 生成文件只允许由命令更新，CI 检查生成结果没有 diff；Golden tests 继续验证 wire format。

T15/T17 已明确“禁止手工维护镜像 DTO interface”。

### 3.3 T17：Vue 基础设施

T17 受 T16 阻塞，因此尚未提前加入正式路由、store 和测试依赖；任务已指定：

- [Vue Router](https://router.vuejs.org/)：Tools、Runs、Settings、未来 Skills 路由；
- [Pinia](https://pinia.vuejs.org/)：snapshot、运行队列、设置等跨页面状态；
- `Vitest` + `@vue/test-utils`：组件/store/transport 测试；
- mock transport 只模拟 T15 生成的 bindings，不另建第二份接口模型。

dbox 自己实现的内容仅包括 store action、事件去重规则、错误展示策略和页面状态，不应实现路由器、全局状态框架或测试 runner。

### 3.4 T18/T19：大列表、日志与高频事件

工具列表可能只有数百项，但运行日志可能持续增长。不要自行实现 viewport 计算、DOM 回收或通用 debounce/throttle。

建议：

- 使用 [`@tanstack/vue-virtual`](https://tanstack.com/virtual/latest/docs/framework/vue/vue-virtual) 虚拟化工具表格和运行日志；
- 使用 VueUse 的 `useThrottleFn`/`useDebounceFn` 或等价维护库处理 UI 刷新节流；
- 后端仍负责有界日志、事件序号和最终状态，前端节流不能成为数据正确性来源；
- 使用 `axe-core`/Vue 测试集成做基础可访问性回归，键盘焦点和业务语义仍由 dbox 测试。

只有当性能基准证明普通列表足够时，T18 可以延迟引入虚拟化；T19 的持续日志应默认按虚拟化设计。

### 3.5 T20：打包、签名、公证与更新

发布任务必须明确复用 Tauri 提供的能力：

- Tauri CLI/bundler 生成 macOS 安装包；
- Tauri updater 处理 dbox 自身升级；
- Tauri 官方签名、公证配置和 CI/action 作为主路径；
- dbox 只维护 bundle 配置、密钥注入策略、channel、版本策略和验收脚本。

禁止实现自定义安装器、自有增量更新协议或在仓库脚本中复制 Tauri bundler 功能。

### 3.6 T16：依赖治理与文档一致性

Backend Gate 已加入：

- `cargo deny check`：许可证、禁用/重复依赖和 advisory policy；
- `cargo audit`：RustSec 漏洞检查；
- `npm audit` 是否作为强制门禁需单独决定，避免上游无修复 advisory 永久阻塞；
- ADR 中记录 Git dependency 的 pinned revision、许可证和更新流程；
- 检查只启用必要 Cargo features。

T16 原有“八类场景”文档错误已修正为“九类场景”。

## 4. 当前无需替换的自定义实现

### 4.1 Catalog 合并

[`Figment`](https://docs.rs/figment/latest/figment/)适合 TOML/JSON/env 等配置来源、字段 provenance 和普通 merge/join，但 dbox 要求数组按 Component/Strategy 子项 ID 合并。Figment 对数组采用替换或拼接，不能直接表达该语义。

因此：

- settings 多来源配置将来可以评估 Figment；
- catalog matcher、按 ID 合并、Provider 基础结果不得被 manifest 删除等规则继续由 dbox 实现；
- 当前 `serde`/`toml`/`serde_path_to_error`/`regex` 已覆盖主要基础设施。

### 4.2 Refresh cache

[`moka::future::Cache`](https://docs.rs/moka/latest/moka/future/struct.Cache.html)适合高并发、按 key TTL/TTI、容量淘汰和 single-flight 场景。当前 ApplicationService 只有短 TTL provider 时间戳和全局有状态操作锁，引入 Moka 的收益有限。

保持当前实现，直到出现以下任一条件：

- Provider/Tool 粒度缓存显著增加；
- 需要按 key single-flight；
- 需要容量/权重淘汰；
- 多个读写任务不再由应用服务操作锁串行化。

满足条件后再新增 Moka spike，不提前引入。

### 4.3 Run queue

T14 需要 queued item 可定位取消、批次聚合、同 Tool 互斥、重试关联、历史持久化和 interrupted 恢复。通用 `mpsc` 单独不能表达这些业务查询；重量级分布式 job framework 对本地串行 MVP 也不合适。

当前实现已经复用 Tokio `Mutex`/`Notify` 和 T08 的 `CancellationToken`/process cleanup。`BTreeMap + VecDeque` 只承载 dbox 调度状态，因此可保留。未来启用真正并发时再评估 `mpsc`/`Semaphore` 或专业任务框架。

### 4.4 Provider 解析与领域规则

以下内容必须由 dbox 保留：

- npm/brew/mise/rustup 等公开 CLI 输出到统一 Installation/Tool 的映射；
- 来源限定 ID、多 executable 和多 Component；
- Provider capability 与特殊退出码/部分失败语义；
- Pi core/extensions 等 catalog 增强；
- UpdatePlan 选择、post-check 和状态汇总；
- `npx skills` 不稳定/非 JSON 输出到 partial/unknown 的映射。

这些是产品业务规则，不属于重复实现通用基础设施。

## 5. 建议修改顺序

1. [x] 修正 T16 “八类/九类”并加入依赖治理门禁。
2. [x] 在 T15 开工前完成 `tauri-specta`/替代方案 spike 和 ADR。
3. [x] 验证并替换自有原子写入算法，不改变 revision/retention 语义。
4. [x] T17 明确 Vue Router、Pinia、Vitest、Vue Test Utils。
5. [x] T18/T19 明确虚拟化、节流和可访问性测试库。
6. [x] T20 明确仅使用 Tauri bundler/updater/signing 链路。
7. [x] 把结论同步回对应任务文件；后续实现仍受各任务依赖与验收约束。

## 6. 验收标准

当本审计建议全部处理后，应满足：

- 通用基础设施都有依赖或明确 ADR，不存在无说明的自研替代；
- Rust/TypeScript wire types 单一来源生成，并有兼容性测试；
- 原子写入不再由 dbox 自行实现，或 ADR 证明库无法满足 durability 要求；
- 前端没有自研 router/store/virtualizer/throttle/test runner；
- 发布没有自研 bundler/updater；
- 新依赖均固定版本/feature，并经过许可证、MSRV、漏洞和平台检查；
- dbox 自定义代码集中在 Provider 映射、领域状态、策略、队列业务规则与 UI 产品行为。
