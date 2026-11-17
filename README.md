<!-- markdownlint-disable MD033 MD041 -->
<div align="center">
  <p><img src="docs/assets/logo.png" alt="SubBoost" width="96"></p>
  <h1>SubBoost</h1>
  <p>
    <img src="https://img.shields.io/badge/platform-Linux%20%2B%20Docker-lightgrey.svg" alt="Platform: Linux + Docker">
    <img src="https://img.shields.io/badge/version-2.11.0-green.svg" alt="Version 2.11.0">
    <a href="https://subboost.org"><img src="https://img.shields.io/badge/app-subboost.org-brightgreen.svg" alt="Online app"></a>
    <a href="https://docs.subboost.org"><img src="https://img.shields.io/badge/docs-subboost.org-blue.svg" alt="Documentation"></a>
    <img src="https://img.shields.io/badge/image-GHCR-blue.svg" alt="GHCR image">
  </p>
  <p><strong><a href="README.md">English</a> | <a href="README-CN.md">中文</a></strong></p>
</div>
<!-- markdownlint-enable MD033 MD041 -->

**SubBoost** is a **Clash/Mihomo subscription conversion, enhancement, and management** tool. It can convert airport subscriptions and self-hosted nodes into optimized aggregate subscriptions, then update them automatically. With the visual UI, you can configure advanced features such as **chained proxies, precise routing, DNS leak prevention, and multi-subscription aggregation** in one click.

## Highlights & Use Cases

- **Subscription conversion**: Import subscription links, YAML files, node links, and other common formats.
- **Tailscale outbound**: Add Tailscale or Headscale outbound nodes through a form.
- **Node management**: Rename, delete, or configure listening ports for nodes in batches.
- **Node filtering**: Build `filtered proxy groups` with only selected nodes by source, region, and custom rules.
- **Chained proxies**: Configure chained proxies and `relay proxy groups` visually in one click.
- **Precise routing**: Enable more than 30 common proxy groups and over 2,000 remote rule sets.
- **Rule management**: Reorder rules for deeper customization by advanced users.
- **DNS leak prevention**: The default `basic and DNS configuration` helps prevent DNS leaks.
- **Automatic refresh**: Refresh subscriptions on a schedule and intelligently match nodes during refresh.

## Interface Preview

<p align="center">
  <img src="docs/assets/screenshot-main.png" alt="SubBoost visual configuration interface" width="960">
</p>

## Usage & Deployment

- Online entry: [No deployment required - direct access to the public service](https://subboost.org)
- Deployment docs: [One-click deployment - pulls an image to build, faster with lower requirements](https://docs.subboost.org/deploy/one-click)
- Deployment docs: [Advanced deployment - compiles from source, slower with higher requirements](https://docs.subboost.org/deploy/advanced)
- Configuration guide: [Clash configuration simple enough for a paramecium: configure precise routing and chained proxies from the UI in one click](https://ryanvan.com/t/topic/59?u=ryan)

## Rust backend, CLI and MCP

HTTP, CLI and MCP share native Rust conversion and management logic. Next.js serves the frontend and forwards API requests. SQLite is created automatically; PostgreSQL, Prisma and a separate cron container are not required.

Configuration is separate from code:
- `config/subboost.toml`: database, listener, public URL, network budgets, quotas, authentication, in-process refresh and generation defaults. Select it with `--settings FILE` or `SUBBOOST_CONFIG`. Relative paths use the process working directory; restart after editing.
- `config/frontend.json`: development frontend address and backend forwarding target. `SUBBOOST_BACKEND_URL` overrides the target, which is resolved at frontend build time for production.
- `config/parser-policy.json`: parser, YAML expansion and regex safety budgets; override its path with `SUBBOOST_PARSER_POLICY`.
- `config/mihomo/` and `config/rules/`: generation defaults and rule catalog data.

Use stable Rust and a Node.js version declared in `engines` for the frontend. The standalone CLI needs neither Node.js nor an HTTP server:

```bash
cargo install --locked --path backend
subboost --help
subboost parse subscription.txt
subboost convert subscription.txt > config.yaml
subboost subscription list
subboost mcp
```

MCP client configuration:

```json
{"mcpServers":{"subboost":{"command":"subboost","args":["mcp"]}}}
```

Native stdio JSON-RPC supports discovery, subscription/template CRUD, refresh, conversion, rules and backups. CLI/MCP share `application.local_owner`, separate from browser guests. Exchange data with backup export/import when needed. Use absolute executable paths if the deployment manager and native CLI are both installed as `subboost`.

Browser use requires no account or administrator setup. Each guest has a separate HttpOnly cookie; clearing it loses access to that workspace, so export data first. Optional administrator setup requires `LOCAL_SETUP_TOKEN`. Guests cannot enable private-network source fetching. Public subscription links are bearer credentials.

A sequential timer inside the Rust server refreshes subscriptions without accumulating missed ticks. Failed refreshes preserve the previous node snapshot. Deployment values and budgets come from configuration files.

### Deployment, backups and migration

Build containers from source:

```bash
APP_URL=http://127.0.0.1:3001 docker compose -f local/docker-compose.yml up -d --build
```

Only the frontend port is published. API, SQLite and refresh run in one backend container. Release Compose requires matching frontend/backend images and the `subboost.toml` asset from the updated release workflow; do not mix older images.

Back up SQLite with its adjacent `.key` file: losing that key makes data unrecoverable. Include WAL files when copying an open database; stopping the backend is safer. The deployment manager's `backup` pauses writers, checks SQLite and saves database/WAL/key files, environment and runtime configuration. Native `subboost backup export` produces plaintext JSON containing node credentials; store it privately.

Do not overwrite an existing PostgreSQL deployment. Back up its volumes and stop the old application and cron first, then export from an environment that can reach the old database:

```bash
(umask 077; psql "$DATABASE_URL" -XAt -v ON_ERROR_STOP=1 -f scripts/export-legacy.sql > legacy.json)
subboost --database data/subboost.sqlite3 migrate-legacy legacy.json
```

Provide the old `ENCRYPTION_KEY` through the environment and use a separate SQLite directory. Import preserves administrators, ownership, IDs, link tokens, templates and refresh state. Wrong keys or conflicts abort the import without overwriting existing records. Old sessions are not imported; sign in again. Verify data and generated subscriptions before switching traffic, and retain the old volumes until verification is complete.

## Development Notes

Developers can start a local development environment from source:

```bash
npm ci
npm run dev
```

Common checks:

```bash
npm run lint
npm run test:unit
npm run check:local-app
```

## Links

- Online entry: [https://subboost.org](https://subboost.org)
- Deployment docs: [https://docs.subboost.org](https://docs.subboost.org)
- Release announcements: [docs/release-notes.md](./docs/release-notes.md)
- Changelog: [https://subboost.org/faq](https://subboost.org/faq)
- Forum feedback: [LINUX DO](https://linux.do/) & [IDC Flare](https://idcflare.com/).

## Star History

<a href="https://www.star-history.com/?repos=subboost%2Fsubboost&type=date&legend=top-left">
 <picture>
   <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/chart?repos=subboost/subboost&type=date&theme=dark&legend=top-left" />
   <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/chart?repos=subboost/subboost&type=date&legend=top-left" />
   <img alt="Star History Chart" src="https://api.star-history.com/chart?repos=subboost/subboost&type=date&legend=top-left" />
 </picture>
</a>

## License

The public SubBoost source code is licensed under the [GNU Affero General Public License v3.0 only](./LICENSE).

If you modify SubBoost and provide it to users over a network, AGPL-3.0 requires you to offer those users the corresponding source code. The public source entry is [SubBoost/subboost](https://github.com/SubBoost/subboost).

## Disclaimer

This project does not provide any proxy service and makes no guarantee about the availability or legality of third-party subscription content.
