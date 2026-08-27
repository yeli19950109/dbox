# T06：设置、缓存与运行记录持久化

- 状态：Completed
- 阶段：Rust 核心
- 依赖：T01、T02
- 阻塞：T13、T14

## 目标

集成经过验证的原子写入库，实现可测试且不硬编码用户目录的本地持久化层；dbox 只负责 schema、revision 和日志保留策略。

## 交付内容

- 用 `AppPaths` 抽象 config/data/log 目录，生产环境由 Tauri path API 注入，测试使用临时目录。
- 读写 `settings.toml`、`state.json` 和按 Run 划分的 JSONL 日志。
- 使用固定版本且关闭非必要 feature 的 `atomic-write-file` 承担同目录临时文件、同步、原子替换和失败清理；dbox 不实现临时文件命名或 rename 算法。
- 验证覆盖现有文件、Unix 权限继承、commit/discard 错误语义和父目录 durability；平台差异与限制记录在 ADR。
- 支持 schema version、默认值和迁移入口。
- 支持 revision/etag 乐观锁，防止覆盖外部修改。
- 实现日志数量/总大小轮转；正在运行的日志不得被删除。

## 单元测试

- 首次启动目录不存在时创建并返回默认设置。
- 写入后读取 round-trip；discard/模拟中断时旧文件仍有效。
- 覆盖已有目标时保留内容原子性；只读目录和 commit 失败返回带目标路径的错误。
- Unix 覆盖已有文件时保留 mode；原子写入库保证临时文件与目标同目录，不产生跨设备 rename。
- revision 冲突返回错误，不静默覆盖。
- 损坏文件返回带路径错误，并保留原文件。
- 日志轮转遵守数量与大小，活动 Run 被保留。
- 测试只写临时目录，不写真实用户配置。

## 完成标准

持久化模块不依赖全局路径，可在临时目录重复运行；项目代码中不存在自研临时文件/rename 算法；单元测试覆盖异常文件和库的关键原子写入语义。

## 非目标

- MVP 不引入 SQLite。
- Run 一次一份 JSONL 的跨文件 retention policy 不交给通用日志轮转库。

## 验证记录

- 完成日期：2026-08-27
- 关键文件：`src-tauri/src/persistence/mod.rs`、`src-tauri/Cargo.toml`、`docs/adr/0001-rust-infrastructure-dependencies.md`
- 执行命令：`cargo test --manifest-path src-tauri/Cargo.toml persistence::tests --no-fail-fast`
- 测试结果：12 个持久化测试通过，覆盖 discard、已有目标、只读目录、commit 失败、Unix mode、revision、损坏文件和日志 retention。
- 已知限制：非 Unix 平台不保证继承原文件权限/owner/ACL/xattr；由平台 CI 验证原子替换，Unix 由库同步父目录。
