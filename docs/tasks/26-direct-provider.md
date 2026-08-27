# T26：Direct/Self-update Provider（后续）

- 状态：Future
- 阶段：后续 Provider
- 依赖：T03–T09、T13、T16
- 阻塞：无

## 目标

支持 Deno、Bun、Flutter 等由官方脚本、压缩包或自身命令直接安装的实例，同时避免按产品名猜测安装来源。

## 交付内容

- 只通过用户/内置 manifest 明确声明 discovery、version source 和 update strategy。
- executable 绝对路径与安装来源确认后才生成计划。
- 同一产品来自 direct/mise/brew/npm 时保留独立 Installation。
- 支持 self-update、下载器或 unsupported；不提供任意 shell 字符串。
- 命令语义变化通过版本化 catalog/用户 override 处理。

## 测试要求

- fake Deno/Bun/Flutter manifest 覆盖多来源与路径冲突。
- matcher 歧义时拒绝自动更新并要求确认来源。
- shell 管道/重定向配置被拒绝。
- 测试不下载或更新真实 SDK。

## 完成标准

至少三个 fixture direct 工具可被发现并生成安全计划；主流程无需加入产品名分支。

