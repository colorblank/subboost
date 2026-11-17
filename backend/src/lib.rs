//! SubBoost: 高性能订阅聚合转换与规则生成引擎。
//!
//! 提供纯 Rust 实现的节点解析、Mihomo 配置生成、订阅自动刷新调度、
//! 本地加密 SQLite 存储、HTTP API 以及 CLI 和 MCP（Model Context Protocol）服务接口。

/// HTTP API 服务层，基于 Axum 实现，包含鉴权、CSRF 防护、速率限制与路由分发。
pub mod api;

/// CLI 命令行工具实现，基于 Clap 解析，支持独立解析、生成、服务启动与数据管理。
pub mod cli;

/// 系统配置模块，包含网络、限额、认证与生成参数的定义与 TOML 解析。
pub mod config;

/// Mihomo (Clash.Meta) YAML 配置文件生成器，处理代理组拓扑、路由分流与 DNS 编排。
pub mod generator;

/// 订阅信息提取与解析工具，支持流量用量（Upload/Download/Total）与到期时间计算。
pub mod info;

/// 历史数据迁移工具，支持从旧版 PostgreSQL/Prisma 加密导出导入至 SQLite。
pub mod legacy;

/// Model Context Protocol (MCP 2024-11-05) 服务端实现，支持基于标准 I/O 的 JSON-RPC。
pub mod mcp;

/// 网络客户端封装，提供安全出网、SSRF 防护、DNS-over-HTTPS (DoH) 与重试机制。
pub mod network;

/// 订阅与协议解析器，支持 SS、SSR、Vmess、Vless、Trojan、Hysteria2、TUIC、WireGuard 等协议。
pub mod parser;

/// 订阅自动更新与刷新子系统，包含后台调度器与快照比对算法。
pub mod refresh;

/// 分流规则管理模块，提供规则集分类检索、版本检测与智能推荐。
pub mod rules;

/// 核心业务服务编排层，协调存储、网络、配置与解析引擎。
pub mod service;

/// 本地加密持久化存储引擎，基于 SQLite 实现原子更新、版本控制与配额隔离。
pub mod store;

/// 内置与自定义规则模版系统，提供轻量、标准与全功能订阅模版。
pub mod templates;
