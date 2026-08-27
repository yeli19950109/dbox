# T23：mise Provider（后续）

- 状态：Future
- 阶段：后续 Provider
- 依赖：T03、T04、T07–T09、T13、T16
- 阻塞：无

## 目标

通过 mise 的公开 CLI 管理全局 scope 工具及多版本信息，不把 mise 简化为 npm/brew 单版本包。

## 交付内容

- probe mise 与 capability/version。
- 扫描全局安装和激活版本，保留 backend、版本、scope 与安装路径。
- 表达多版本共存和当前激活版本。
- 生成精确更新 CommandPlan，不影响未选择工具或项目级配置。
- 同一产品由 mise/npm/brew 安装时分开显示。

## 测试要求

- fake mise 覆盖多版本、未激活、全局/项目混合、插件缺失和离线。
- 测试不得修改真实 mise 配置。

## 完成标准

fixture scan/check/plan 通过，且不修改通用领域/API。

