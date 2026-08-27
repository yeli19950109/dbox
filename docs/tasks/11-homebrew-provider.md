# T11：Homebrew Provider

- 状态：Pending
- 阶段：Rust Provider
- 依赖：T03、T04、T07–T09
- 阻塞：T12、T13、T16

## 目标

扫描并管理全部 Homebrew formula 和 cask，保留两类 package 的不同身份与更新语义。

## 交付内容

- probe brew 路径、版本、prefix 和架构。
- 读取全部 installed formula/cask 及版本，解析 outdated JSON 元数据。
- 使用 `homebrew-formula:<name>` 与 `homebrew-cask:<token>` 等来源限定 ID。
- 最佳努力关联 Homebrew 暴露的 executable；没有 executable 的包仍作为 Installation 展示。
- formula 与 cask 分别生成精确到单项的升级 CommandPlan。
- 识别 pinned、keg-only、multiple versions、disabled/deprecated 和权限错误。
- 离线/远端检查失败时仍展示 installed 信息，不误报最新。

## 单元与 fixture 测试

- 空列表、formula、cask、同名 formula/cask、多版本和 pinned。
- outdated JSON v2 正常/缺字段/未知新增字段。
- 未收录 catalog 的 formula/cask 仍可管理。
- executable 关联失败不删除 Installation。
- 更新计划不会混淆 formula 和 cask。
- fake brew 记录 argv；测试不调用真实 `brew upgrade`。

## 完成标准

fixture brew 可以端到端完成 scan → check → plan；formula/cask 身份稳定；测试通过。

## 非目标

- 不运行 `brew cleanup`。
- 不自动 unpin 或处理 sudo。

