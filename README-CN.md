<!-- markdownlint-disable MD033 MD041 -->
<div align="center">
  <p><img src="docs/assets/logo.png" alt="SubBoost" width="96"></p>
  <h1>SubBoost</h1>
  <p>
    <img src="https://img.shields.io/badge/platform-Linux%20%2B%20Docker-lightgrey.svg" alt="平台：Linux + Docker">
    <img src="https://img.shields.io/badge/version-2.11.0-green.svg" alt="版本 2.11.0">
    <a href="https://subboost.org"><img src="https://img.shields.io/badge/app-subboost.org-brightgreen.svg" alt="在线入口"></a>
    <a href="https://docs.subboost.org"><img src="https://img.shields.io/badge/docs-subboost.org-blue.svg" alt="文档"></a>
    <img src="https://img.shields.io/badge/image-GHCR-blue.svg" alt="GHCR 镜像">
  </p>
  <p><strong><a href="README.md">English</a> | <a href="README-CN.md">中文</a></strong></p>
</div>
<!-- markdownlint-enable MD033 MD041 -->

**SubBoost** 是一个 **Clash/Mihomo 订阅转换、增强和管理** 工具。可以将机场订阅和自建节点转换为优化后的聚合订阅，并自动更新。通过 UI 可视化，一键实现 **链式代理、精确分流、防 DNS 泄露和多订阅聚合** 等高级功能。

## 亮点与场景

- **订阅转换**：支持订阅链接、YAML 文件和节点链接等多种格式导入。
- **Tailscale 出站**：支持用表单添加 Tailscale / Headscale 出站节点。
- **节点管理**：支持批量对节点重命名、删除或配置监听端口。
- **节点筛选**：可在分流组高级模式中按导入源、地区和自定义规则筛选节点。
- **链式代理**：一键可视化配置链式代理和 `中转代理组`。
- **精确分流**：内置 30 多个常用代理组和 2000 多条远程规则集供启用。
- **规则管理**：可修改规则顺序，供高级用户深度自定义。
- **防 DNS 泄露**：默认的 `基础和 DNS 配置` 可防止 DNS 泄露。
- **自动刷新**：定时自动刷新订阅，刷新时可智能匹配节点。

## 界面展示

<p align="center">
  <img src="docs/assets/screenshot-main.png" alt="SubBoost 可视化配置界面" width="960">
</p>

## 使用和部署

- 在线入口：[无需部署 - 直接使用的公益服务](https://subboost.org)
- 部署文档：[一键部署 - 拉取镜像构建，速度快配置要求低](https://docs.subboost.org/deploy/one-click)
- 部署文档：[高级部署 - 编译源码构建，速度慢配置要求高](https://docs.subboost.org/deploy/advanced)
- 配置教程：[草履虫也能学会的 Clash 配置：UI 界面一键配置精确分流、链式代理](https://ryanvan.com/t/topic/59?u=ryan)

## Rust 后端、CLI 与 MCP

HTTP API、CLI 和 MCP 共享纯 Rust 转换与管理逻辑；Next.js 只负责前端和 API 转发。无需 PostgreSQL、Prisma 或独立定时任务容器，SQLite 会自动创建。

配置与代码分离：
- `config/subboost.toml`：SQLite 路径、监听地址、公开 URL、请求预算、游客配额、认证、进程内刷新及生成默认值。使用 `--settings FILE` 或 `SUBBOOST_CONFIG` 指定文件；相对路径以进程工作目录为准，修改后重启。
- `config/frontend.json`：前端开发地址和 Rust API 转发地址；`SUBBOOST_BACKEND_URL` 可覆盖转发目标。生产构建时确定转发地址。
- `config/parser-policy.json`：解析、YAML 展开和正则的安全预算；`SUBBOOST_PARSER_POLICY` 可指定其他文件。
- `config/mihomo/`、`config/rules/`：独立的生成默认数据和规则目录。

需要 Rust stable；前端另需项目 `engines` 声明的 Node.js。独立 CLI 不需要 Node.js 或 HTTP 服务：

```bash
cargo install --locked --path backend
subboost --help
subboost parse subscription.txt
subboost convert subscription.txt > config.yaml
subboost subscription list
subboost mcp
```

MCP 客户端配置：

```json
{"mcpServers":{"subboost":{"command":"subboost","args":["mcp"]}}}
```

MCP 使用原生 stdio JSON-RPC，支持工具发现、订阅/模板 CRUD、刷新、转换、规则和备份。CLI/MCP 共用 `application.local_owner` 工作空间，不会自动读取浏览器游客的数据。需要交换数据时使用备份导入/导出；部署运维脚本与原生 CLI 同名时，请使用各自的完整路径。

浏览器直接进入游客工作空间，无需注册或初始化管理员。不同游客使用独立的 HttpOnly Cookie；清除 Cookie 后不能重新认领原工作空间，请先导出数据。管理员登录仍可选，初始化需设置 `LOCAL_SETUP_TOKEN`。游客不能开启内网订阅访问。公开订阅链接是访问凭证，请勿泄露。

刷新由 Rust 服务内部的顺序计时器完成，不堆积错过的周期；刷新失败保留上一次节点快照。端口、间隔和预算以配置文件为准。

### 部署、备份与旧库迁移

从源码构建容器：

```bash
APP_URL=http://127.0.0.1:3001 docker compose -f local/docker-compose.yml up -d --build
```

只有前端端口对主机发布；Rust API、SQLite 和刷新器在同一个后端容器中。发行版 Compose 需要匹配的前端、后端镜像与 `subboost.toml` 附件，必须先通过新的发布工作流发布，不能混用旧版镜像。

SQLite 数据库和相邻的 `.key` 文件必须一起备份；缺失密钥无法解密数据。在线复制时还需要 WAL，建议先停止后端。运维脚本的 `backup` 会暂停写入，校验 SQLite 并保存数据库、WAL、密钥、环境文件和运行配置。`subboost backup export` 输出含节点凭据的明文 JSON，只能保存到私有文件。

旧 PostgreSQL 部署不会被直接覆盖。先备份旧卷并暂停旧应用和 cron，再在旧数据库可达的环境导出：

```bash
(umask 077; psql "$DATABASE_URL" -XAt -v ON_ERROR_STOP=1 -f scripts/export-legacy.sql > legacy.json)
subboost --database data/subboost.sqlite3 migrate-legacy legacy.json
```

迁移命令需在环境中提供旧安装的 `ENCRYPTION_KEY`，目标使用新的独立 SQLite 目录。导入保留管理员、所有者、ID、订阅 token、模板和刷新状态；错误密钥或任何冲突都会中止导入，不覆盖现有记录。旧会话不迁移，迁移后重新登录；核对数据和订阅输出后再切换流量，保留旧卷直到验证完成。

## 开发说明

开发者可以从源码启动本地开发环境：

```bash
npm ci
npm run dev
```

常用检查：

```bash
npm run lint
npm run test:unit
npm run check:local-app
```

## 相关链接

- 在线入口：[https://subboost.org](https://subboost.org)
- 部署文档：[https://docs.subboost.org](https://docs.subboost.org)
- 发行公告：[docs/release-notes.md](./docs/release-notes.md)
- 更新日志：[https://subboost.org/faq](https://subboost.org/faq)
- 论坛反馈：[LINUX DO](https://linux.do/) & [IDC Flare](https://idcflare.com/)

## ⭐轨迹

<a href="https://www.star-history.com/?repos=subboost%2Fsubboost&type=date&legend=top-left">
 <picture>
   <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/chart?repos=subboost/subboost&type=date&theme=dark&legend=top-left" />
   <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/chart?repos=subboost/subboost&type=date&legend=top-left" />
   <img alt="SubBoost Star 成长轨迹" src="https://api.star-history.com/chart?repos=subboost/subboost&type=date&legend=top-left" />
 </picture>
</a>

## 开源许可

SubBoost 公开源码以 [GNU Affero General Public License v3.0 only](./LICENSE) 授权。

如果你修改 SubBoost 并通过网络向用户提供服务，AGPL-3.0 要求你向这些用户提供对应源码。公开源码入口是 [SubBoost/subboost](https://github.com/SubBoost/subboost)。

## 免责声明

本项目不提供任何代理服务，不对第三方订阅内容的可用性与合法性作出保证。
