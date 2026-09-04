//! # ntex-mcp：MCP server（M9 形态①排版渲染）。
//!
//! stdio JSON-RPC 2.0 服务：TeX/LaTeX 源码 → 内存 DVI → PDF（base64 返回）。
//! 不引外部 SDK：JSON 解析/序列化与 base64 均为本地最小实现（RFC 8259 /
//! RFC 4648）。协议细节与安全基线见 [`server`] 模块文档（plan.md §11 M9、
//! RFC-3 副作用隔离）。

#![deny(unsafe_code)]

pub mod base64;
pub mod json;
pub mod server;
pub mod stdio;
