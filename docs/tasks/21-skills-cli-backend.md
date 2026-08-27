# T21：`npx skills` 后端薄适配

- 状态：Future
- 阶段：后续 Rust 集成
- 依赖：T08、T09、T15、T16
- 阻塞：T22

## 目标

通过公开的 `npx skills` CLI 管理 Skill。dbox 不实现 Skill 文件扫描、复制、链接、更新或删除算法。

## 交付内容

- 定位 `npx` 并做版本/`--help` capability detection。
- 映射公开命令：list/find/add/check/update/remove。
- 优先解析 `list --json`；没有稳定 JSON 的命令保留原始输出并返回 unknown。
- project 操作显式设置用户选择的 workspace cwd；global 使用公开 flag。
- 调用时设置 `DISABLE_TELEMETRY=1`。
- 状态改变命令生成 CommandPlan，用户确认后才添加必要的非交互参数。
- 不读取/修改 skills 目录、`.skill-lock.json`、`skills-lock.json` 或 CLI 内部模块。

## 单元与 fixture 测试

- list JSON 正常、未知字段、非法 JSON 和原始输出回退。
- npx 首次下载提示、超时、取消和找不到 Node。
- global/project/agent/source/skill 参数均为独立 argv。
- 输出包含逐项失败但退出 0 时返回 partial/unknown，而不是成功。
- 测试只调用 fake npx，不联网、不触碰真实 Skill。

## 完成标准

Fake npx 覆盖所有公开操作；dbox 代码库没有自研 Skill 文件管理逻辑；Rust 测试通过。

