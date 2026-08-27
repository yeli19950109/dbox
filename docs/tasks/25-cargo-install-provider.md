# T25：Cargo Install Provider（后续）

- 状态：Future
- 阶段：后续 Provider
- 依赖：T03、T04、T07–T09、T13、T16
- 阻塞：无

## 目标

管理通过 `cargo install` 安装的用户级 binary crates，并与 rustup toolchain 管理保持分离。

## 交付内容

- 扫描 Cargo 安装元数据和可执行文件。
- 表达 crate 名、版本、features/locked 等可可靠获得的信息。
- 查询最新版本失败时保留 installed 状态。
- 生成单 crate 更新 CommandPlan，保留必要安装选项或明确提示无法重建。

## 测试要求

- fake cargo metadata 覆盖普通 crate、多个 binaries、git/path source 和损坏记录。
- 无法可靠更新的 source 标记 unsupported，不猜命令。
- 测试不写真实 Cargo home。

## 完成标准

registry crate 可管理，git/path 等特殊来源不会被错误升级。

