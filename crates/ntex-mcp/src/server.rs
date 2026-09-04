//! MCP server 核心（形态①排版渲染）：请求分派 + typeset_pdf 工具。
//!
//! 协议面（MCP 规范 stdio 传输的最小子集，plan.md §11 M9）：
//! - `initialize` → 返回协议版本与服务端能力（tools）；
//! - `notifications/initialized`（及旧式 `initialized`）→ 客户端通知，无响应；
//! - `tools/list` → 工具清单（JSON Schema 形态的入参描述）；
//! - `tools/call` → 执行 typeset_pdf：源码 → 内存 DVI → PDF（base64）；
//! - 其余方法 → JSON-RPC 错误 -32601。
//!
//! 安全基线（RFC-3）：排版器注入 MemVfs，`\openout`/`\write` 全部落在内存，
//! `\write18` 由引擎直接拒绝——server 进程无落盘副作用、无子进程。
//!
//! 错误口径：帧解析失败 → -32700；请求结构不合法 → -32600；工具入参不合法或
//! 引擎报错 → -32602（data.details 带引擎结构化消息，含行号上下文）；运行期
//! 故障 → -32603。所有路径 Result 收敛，无 panic（引擎契约：畸形输入不 panic）。

use ntex_core::error::Error as CoreError;
use ntex_io::MemVfs;
use ntex_layout::typeset::Typesetter;
use ntex_pdf::PdfOptions;

use crate::base64;
use crate::json::{parse, Json};

/// MCP 协议版本（服务端支持的日期化版本）。
pub const PROTOCOL_VERSION: &str = "2025-06-18";

/// 工具名（形态①排版渲染）。
pub const TOOL_NAME: &str = "typeset_pdf";

/// JSON-RPC 2.0 标准错误码。
pub mod code {
    pub const PARSE_ERROR: i64 = -32700;
    pub const INVALID_REQUEST: i64 = -32600;
    pub const METHOD_NOT_FOUND: i64 = -32601;
    pub const INVALID_PARAMS: i64 = -32602;
    pub const INTERNAL_ERROR: i64 = -32603;
}

/// 处理一帧 JSON-RPC 请求文本，返回响应 JSON（通知返回 None）。
pub fn handle_frame(line: &str) -> Option<Json> {
    match parse(line) {
        Ok(value) => handle_message(&value),
        Err(e) => Some(error_response(
            Json::Null,
            code::PARSE_ERROR,
            &e.to_string(),
        )),
    }
}

/// 处理一条已解析的 JSON-RPC 消息（测试与 stdio 循环共用）。
pub fn handle_message(value: &Json) -> Option<Json> {
    if value.get("jsonrpc") != &Json::String("2.0".into()) {
        return Some(error_response(
            extract_id(value),
            code::INVALID_REQUEST,
            "jsonrpc 必须为 2.0",
        ));
    }
    let method = match value.get("method") {
        Json::String(method) => method.clone(),
        _ => {
            return Some(error_response(
                extract_id(value),
                code::INVALID_REQUEST,
                "缺少 method 字符串字段",
            ))
        }
    };
    let id = extract_id(value);
    let is_notification = matches!(value.get("id"), Json::Null);
    let params = value.get("params").clone();

    let outcome = match method.as_str() {
        "initialize" => Ok(initialize_result()),
        "tools/list" => Ok(tools_list_result()),
        "tools/call" => tools_call_result(&params),
        // 通知按 MCP 生命周期处理：不产生任何响应。
        "notifications/initialized" | "initialized" => return None,
        _ => Err((code::METHOD_NOT_FOUND, "method not found".to_owned(), None)),
    };
    match outcome {
        Ok(result) => Some(result_response(id, result)),
        Err((code, message, data)) if !is_notification => {
            Some(error_data_response(id, code, &message, data))
        }
        Err(_) => None,
    }
}

/// 抽取请求 id：JSON-RPC 允许字符串/数字/null（通知）。
fn extract_id(value: &Json) -> Json {
    match value.get("id") {
        id @ (Json::String(_) | Json::Number(_)) => id.clone(),
        _ => Json::Null,
    }
}

/// initialize 响应（MCP 生命周期握手）。
fn initialize_result() -> Json {
    Json::object(vec![
        ("protocolVersion", Json::String(PROTOCOL_VERSION.into())),
        (
            "capabilities",
            Json::object(vec![(
                "tools",
                Json::object(vec![("listChanged", Json::Bool(false))]),
            )]),
        ),
        (
            "serverInfo",
            Json::object(vec![
                ("name", Json::String("ntex-mcp".into())),
                ("version", Json::String(env!("CARGO_PKG_VERSION").into())),
            ]),
        ),
    ])
}

/// tools/list 响应：暴露形态①工具及其入参 schema。
fn tools_list_result() -> Json {
    let properties = Json::object(vec![
        (
            "source",
            Json::object(vec![
                ("type", Json::String("string".into())),
                (
                    "description",
                    Json::String("TeX/LaTeX 源码（引擎支持的纯 TeX 子集）".into()),
                ),
            ]),
        ),
        (
            "filename",
            Json::object(vec![
                ("type", Json::String("string".into())),
                (
                    "description",
                    Json::String("可选文档名（仅用于诊断展示）".into()),
                ),
            ]),
        ),
    ]);
    let schema = Json::object(vec![
        ("type", Json::String("object".into())),
        ("properties", properties),
        ("required", Json::Array(vec![Json::String("source".into())])),
    ]);
    let tool = Json::object(vec![
        ("name", Json::String(TOOL_NAME.into())),
        (
            "description",
            Json::String("排版 TeX/LaTeX 源码并返回 PDF（base64），全程内存无落盘".into()),
        ),
        ("inputSchema", schema),
    ]);
    Json::object(vec![
        ("tools", Json::Array(vec![tool])),
        ("nextCursor", Json::Null),
    ])
}

/// tools/call：仅支持 typeset_pdf；其余工具名按协议返回 -32602。
fn tools_call_result(params: &Json) -> Result<Json, (i64, String, Option<Json>)> {
    let name = match params.get("name") {
        Json::String(name) => name.clone(),
        _ => {
            return Err((
                code::INVALID_PARAMS,
                "tools/call 需要 name 字符串参数".to_owned(),
                None,
            ))
        }
    };
    if name != TOOL_NAME {
        return Err((code::INVALID_PARAMS, format!("未知工具：{name}"), None));
    }
    let args = params.get("arguments").clone();
    let source = match args.get("source") {
        Json::String(source) => source.clone(),
        _ => {
            return Err((
                code::INVALID_PARAMS,
                "缺少 string 类型入参 source".to_owned(),
                None,
            ))
        }
    };
    let filename = match args.get("filename") {
        Json::String(name) if !name.is_empty() => name.clone(),
        _ => "doc.tex".to_owned(),
    };

    match typeset_to_pdf(&source, &filename) {
        Ok((pages, pdf)) => {
            let text = format!("已排版 {pages} 页（{filename}，PDF {} 字节）", pdf.len());
            let content = Json::Array(vec![Json::object(vec![
                ("type", Json::String("text".into())),
                ("text", Json::String(text)),
            ])]);
            let data = Json::Array(vec![Json::object(vec![
                ("mimeType", Json::String("application/pdf".into())),
                ("base64", Json::String(base64::encode(&pdf))),
            ])]);
            Ok(Json::object(vec![
                ("content", content),
                ("data", data),
                ("isError", Json::Bool(false)),
            ]))
        }
        Err(message) => Err((
            code::INVALID_PARAMS,
            "排版失败".to_owned(),
            Some(Json::String(message)),
        )),
    }
}

/// 源码 → DVI → PDF（全部内存，RFC-3 副作用隔离）。
fn typeset_to_pdf(source: &str, filename: &str) -> Result<(usize, Vec<u8>), String> {
    let mut typesetter = Typesetter::with_tfm();
    typesetter.set_vfs(Box::new(MemVfs::new()));
    let (pages, fonts) = typesetter
        .typeset_dvi(source)
        .map_err(|e| format_error(filename, &e))?;
    if pages.is_empty() {
        return Err(format!(
            "{filename}: 排版完成但未产出页面（源码缺少 \\shipout 或未触发自动分页）"
        ));
    }
    let dvi = ntex_dvi::write_dvi(&pages, &fonts);
    let pdf = ntex_pdf::convert(&dvi, &PdfOptions::default())
        .map_err(|e| format!("{filename}: DVI→PDF 转换失败：{e}"))?;
    Ok((pages.len(), pdf))
}

/// 引擎错误 → 结构化消息（错误 Display 已含行号/上下文时原样保留）。
fn format_error(filename: &str, error: &CoreError) -> String {
    format!("{filename}: {error}")
}

// ---------- 响应构造 ----------

fn result_response(id: Json, result: Json) -> Json {
    Json::object(vec![
        ("jsonrpc", Json::String("2.0".into())),
        ("id", id),
        ("result", result),
    ])
}

fn error_response(id: Json, code: i64, message: &str) -> Json {
    error_data_response(id, code, message, None)
}

fn error_data_response(id: Json, code: i64, message: &str, data: Option<Json>) -> Json {
    let mut error = vec![
        ("code", Json::Number(code as f64)),
        ("message", Json::String(message.to_owned())),
    ];
    if let Some(data) = data {
        error.push(("data", data));
    }
    Json::object(vec![
        ("jsonrpc", Json::String("2.0".into())),
        ("id", id),
        ("error", Json::object(error)),
    ])
}
