# T20：MVP 打包、文档与发布验收

- 状态：Pending
- 阶段：发布
- 依赖：T16、T18、T19
- 阻塞：无

## 目标

把已通过后端和 UI 验收的功能打包为可分发的 macOS MVP。

## 交付内容

- Apple Silicon 与 Intel 构建验证。
- 产品图标、窗口信息、CSP、capability 和文件权限复核。
- 使用 Tauri CLI/bundler 生成 macOS 安装包，仓库不复制 bundler 功能或实现自定义安装器。
- 使用 Tauri 官方签名、公证配置和官方 CI/action 作为发布主路径；dbox 只维护 bundle 配置、密钥注入策略、channel、版本策略和验收脚本。
- 使用 Tauri updater 处理 dbox 自身升级；禁止自建增量更新协议。签名/公证方案与未签名开发包限制均写入文档。
- 用户文档：首次扫描、更新确认、Provider 设置、manifest 增强、PATH 故障排查、日志位置。
- 新用户、无 npm、无 brew、离线、权限失败的手工验收记录。
- 发布版本和已知限制。

## 验收要求

- T16 后端门禁与所有前端测试再次通过。
- 新用户不从终端启动 dbox 也能完成扫描。
- 安装包不包含调试密钥、真实 fixture 日志或用户绝对路径。
- Tauri bundler 配置在 Apple Silicon/Intel CI 路径验证；签名/公证凭据只由 CI secret 注入。
- Tauri updater 使用受控 channel/fixture 验证签名、无更新、损坏包和回退提示，不对正式用户执行真实自动更新。

## 完成标准

通过 Tauri 官方链路生成可安装 MVP 包、用户文档和验收报告；常见失败均有可操作恢复路径；仓库没有自研 bundler、installer 或 updater 协议。
