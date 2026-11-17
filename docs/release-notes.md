# SubBoost v2.11.0

## 中文

- 后端迁移为纯 Rust，提供 HTTP API、独立 CLI 与原生 MCP stdio 控制。
- 使用内嵌 SQLite，移除 PostgreSQL、Prisma 和独立 cron 容器要求；由服务内轻量计时器刷新。
- 支持游客免登录，按 Cookie 隔离工作空间；保留可选管理员登录和受令牌保护的初始化。
- 运行配置与解析安全预算移至独立文件，采用命名策略结构、明确的操作模式和 rustfmt 风格。
- 优化导入、预览、导出及游客提示，升级 Next.js 补丁版本。
- 提供旧 PostgreSQL 的离线原子导入；SQLite 备份同时保留密钥、WAL 和配置，更新失败时回滚数据及两个镜像。
- 保留订阅链接重置、逻辑规则、节点智能匹配和 VLESS Reality 导入支持。
- 修复自定义模板列表与详情的 API 字段契约，避免上传后页面崩溃或无法应用模板。
- 修复重新编辑订阅时按原名误删其它节点，保留删除快照中的节点身份。
- 修复批量改名名称交叠或互换时监听端口丢失、错配。
- 修复单个、批量改名及恢复原名后，高级分组的节点成员、排除和顺序引用未同步的问题。

升级提示：本次存储架构不兼容直接替换旧数据库卷。按 README-CN 的导出与迁移步骤新建 SQLite 实例，保留旧卷并验证后再切换流量。数据库与 `.key` 必须配套备份；旧会话需重新登录。

## English

- Replace the backend with native Rust, exposing HTTP, standalone CLI and native MCP stdio tools.
- Use embedded SQLite instead of PostgreSQL/Prisma, with an in-process refresh timer instead of a cron container.
- Enable cookie-isolated guest workspaces without login, while retaining optional administrator login and token-protected setup.
- Move runtime policy and parser budgets to independent files; use named policy structures, explicit operation modes and rustfmt.
- Improve import/preview/export and guest guidance; update the Next.js patch release.
- Add atomic offline PostgreSQL import and SQLite backups containing keys, WAL and configuration, with data and both images restored on failed updates.
- Retain link reset, logical rules, smart node matching and VLESS Reality import support.

Upgrade note: do not replace an old database volume in place. Follow the README export/migration procedure, retain old volumes and verify the new SQLite instance before switching traffic. Database and `.key` backups must stay together; old sessions require a new login.
