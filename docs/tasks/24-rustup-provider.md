# T24：rustup Provider（后续）

- 状态：Future
- 阶段：后续 Provider
- 依赖：T03、T04、T07–T09、T13、T16
- 阻塞：无

## 目标

按 rustup 的层级语义管理 toolchain、component、target 和 rustup 自身，不把它伪装成普通单包 Provider。

## 交付内容

- capability 声明层级 Component 与多 toolchain。
- 扫描 installed/default toolchain、component 和 target。
- 分离 rustup self-update 与 toolchain update 计划。
- 对 override/project toolchain 只读展示，MVP 后续版本不擅自修改。

## 测试要求

- fake rustup 覆盖 stable/nightly、自定义 toolchain、缺失 component、override。
- 自更新和 toolchain 更新 argv 不混淆。
- 测试不调用真实 rustup update。

## 完成标准

层级状态与计划通过 fixture 测试，通用 UI 可根据 capability 展示。

