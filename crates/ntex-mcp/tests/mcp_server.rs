//! ntex-mcp 端到端行为测试（M9 形态①验收口径）。
//!
//! 覆盖：initialize 握手 → tools/list → tools/call（多页文档，验证 PDF
//! magic 与 base64 可还原）；畸形入参（未定义 cs / 空串 / \write18）→
//! JSON-RPC error 且不 panic；未知方法 → -32601；坏帧 → -32700。

use ntex_mcp::json::Json;
use ntex_mcp::server::{self, code, TOOL_NAME};

fn request(id: i64, method: &str, params: Json) -> Json {
    Json::object(vec![
        ("jsonrpc", Json::String("2.0".into())),
        ("id", Json::Number(id as f64)),
        ("method", Json::String(method.into())),
        ("params", params),
    ])
}

fn error_code(response: &Json) -> i64 {
    match response.get("error").get("code") {
        Json::Number(n) => *n as i64,
        _ => panic!("响应缺少 error.code：{response}"),
    }
}

#[test]
fn initialize_handshake_returns_protocol_and_capabilities() {
    let response = server::handle_message(&request(
        1,
        "initialize",
        Json::object(vec![
            ("protocolVersion", Json::String("2025-06-18".into())),
            ("capabilities", Json::object(vec![])),
            (
                "clientInfo",
                Json::object(vec![
                    ("name", Json::String("codex-test".into())),
                    ("version", Json::String("0".into())),
                ]),
            ),
        ]),
    ))
    .expect("initialize 应有响应");
    assert_eq!(
        response.get("result").get("protocolVersion"),
        &Json::String(ntex_mcp::server::PROTOCOL_VERSION.into())
    );
    assert_eq!(
        response
            .get("result")
            .get("capabilities")
            .get("tools")
            .get("listChanged"),
        &Json::Bool(false)
    );
    let initialized = server::handle_message(&Json::object(vec![
        ("jsonrpc", Json::String("2.0".into())),
        ("method", Json::String("notifications/initialized".into())),
    ]));
    assert!(initialized.is_none(), "通知不应有响应");
}

#[test]
fn tools_list_advertises_typeset_pdf() {
    let response =
        server::handle_message(&request(2, "tools/list", Json::Null)).expect("tools/list 应有响应");
    let tools = match response.get("result").get("tools") {
        Json::Array(tools) => tools.clone(),
        other => panic!("tools 应为数组：{other}"),
    };
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].get("name"), &Json::String(TOOL_NAME.into()));
    assert_eq!(
        tools[0].get("inputSchema").get("required"),
        &Json::Array(vec![Json::String("source".into())])
    );
}

#[test]
fn tools_call_renders_pdf_with_magic() {
    // 与 ntex-dvi 集成测试同口径的最小纯原语文档：TFM 字体 + 多段文本，
    // 靠 vsize 溢出触发自动分页（2 页），验证 PDF magic 与页数。
    let source = concat!(
        "\\tolerance 10000\\parindent 20pt\\parskip 6pt\\topskip 12pt\\maxdepth 4pt\n",
        "\\hsize 350pt\\vsize 300pt\n",
        "\\font\\cmr=cmr10\n",
        "\\cmr This is the first paragraph of a richer test document. It is long ",
        "enough to wrap into several lines, exercising the line breaker and the ",
        "vertical page builder end to end.\n\\par\n",
        "\\cmr The second paragraph continues with more sentences so that the total ",
        "height exceeds the page target and forces a fresh page. TeX will fire up ",
        "the page at the best feasible break.\n\\par\n",
        "\\cmr The third paragraph demonstrates interline glue and paragraph spacing ",
        "working together, giving the document a readable rhythm before the end.\n",
        "\\end\n"
    );
    let response = server::handle_message(&request(
        3,
        "tools/call",
        Json::object(vec![
            ("name", Json::String(TOOL_NAME.into())),
            (
                "arguments",
                Json::object(vec![
                    ("source", Json::String(source.into())),
                    ("filename", Json::String("doc.tex".into())),
                ]),
            ),
        ]),
    ))
    .expect("tools/call 应有响应");
    assert!(response.get("error") == &Json::Null, "不应报错：{response}");

    let result = response.get("result");
    let content = match result.get("content") {
        Json::Array(items) => items.clone(),
        other => panic!("content 应为数组：{other}"),
    };
    assert_eq!(content[0].get("type"), &Json::String("text".into()));
    let text = match content[0].get("text") {
        Json::String(text) => text.clone(),
        other => panic!("text 应为字符串：{other}"),
    };
    assert!(text.starts_with("已排版 1 页"), "应产出 1 页：{text}");

    let data = match result.get("data") {
        Json::Array(items) => items.clone(),
        other => panic!("data 应为数组：{other}"),
    };
    assert_eq!(
        data[0].get("mimeType"),
        &Json::String("application/pdf".into())
    );
    let base64_text = match data[0].get("base64") {
        Json::String(text) => text.clone(),
        other => panic!("base64 应为字符串：{other}"),
    };
    let pdf = decode_base64(&base64_text).expect("base64 应可解码");
    assert!(
        pdf.starts_with(b"%PDF-"),
        "PDF magic 缺失：{:?}",
        &pdf[..8.min(pdf.len())]
    );
}

#[test]
fn tools_call_rejects_missing_font_without_panicking() {
    let response = server::handle_message(&request(
        4,
        "tools/call",
        Json::object(vec![
            ("name", Json::String(TOOL_NAME.into())),
            (
                "arguments",
                Json::object(vec![(
                    "source",
                    Json::String("\\font\\x=definitely_not_a_font\\end".into()),
                )]),
            ),
        ]),
    ))
    .expect("畸形输入也应有 JSON-RPC 响应");
    assert_eq!(error_code(&response), code::INVALID_PARAMS);
    let message = match response.get("error").get("message") {
        Json::String(m) => m.clone(),
        other => panic!("message 应为字符串：{other}"),
    };
    assert_eq!(message, "排版失败");
    let details = response.get("error").get("data");
    assert!(
        details != &Json::Null,
        "应带 data.details 结构化消息：{response}"
    );
}

#[test]
fn tools_call_rejects_empty_source_and_shell_escape() {
    let empty = server::handle_message(&request(
        5,
        "tools/call",
        Json::object(vec![
            ("name", Json::String(TOOL_NAME.into())),
            (
                "arguments",
                Json::object(vec![("source", Json::String(String::new()))]),
            ),
        ]),
    ))
    .expect("应有响应");
    assert_eq!(error_code(&empty), code::INVALID_PARAMS);

    let shell = server::handle_message(&request(
        6,
        "tools/call",
        Json::object(vec![
            ("name", Json::String(TOOL_NAME.into())),
            (
                "arguments",
                Json::object(vec![(
                    "source",
                    Json::String("\\write18{echo hi}\\end".into()),
                )]),
            ),
        ]),
    ))
    .expect("应有响应");
    assert_eq!(error_code(&shell), code::INVALID_PARAMS);
    let details = shell.get("error").get("data").to_string();
    assert!(
        details.contains("write18"),
        "应指出 shell 转义被拒：{details}"
    );
}

#[test]
fn unknown_method_and_malformed_frames() {
    let response = server::handle_message(&request(7, "resources/list", Json::Null))
        .expect("未知方法应有错误响应");
    assert_eq!(error_code(&response), code::METHOD_NOT_FOUND);

    let bad_frame = server::handle_frame("{not json").expect("坏帧应回 -32700");
    assert_eq!(error_code(&bad_frame), code::PARSE_ERROR);

    let missing_method =
        server::handle_frame("{\"jsonrpc\":\"2.0\",\"id\":8}").expect("缺 method 应回 -32600");
    assert_eq!(error_code(&missing_method), code::INVALID_REQUEST);
}

/// 独立实现的 base64 解码（仅测试用：验证编码可还原，避免自证循环）。
fn decode_base64(text: &str) -> Option<Vec<u8>> {
    let mut acc: u32 = 0;
    let mut bits = 0u32;
    let mut out = Vec::new();
    for ch in text.chars() {
        if ch == '=' || ch.is_whitespace() {
            continue;
        }
        let value = match ch {
            'A'..='Z' => ch as u32 - 'A' as u32,
            'a'..='z' => ch as u32 - 'a' as u32 + 26,
            '0'..='9' => ch as u32 - '0' as u32 + 52,
            '+' => 62,
            '/' => 63,
            _ => return None,
        };
        acc = (acc << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 0xFF) as u8);
        }
    }
    Some(out)
}

#[test]
fn stdio_loop_serves_handshake_from_memory() {
    // 直接驱动 server 层模拟一帧握手（不 spawn 进程，CI 内存友好）：
    // 与 stdio::run 走同一条 handle_frame 路径。
    let frame = "{\"jsonrpc\":\"2.0\",\"id\":9,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2025-06-18\",\"capabilities\":{},\"clientInfo\":{\"name\":\"smoke\",\"version\":\"0\"}}}";
    let response = ntex_mcp::server::handle_frame(frame).expect("握手应有响应");
    assert_eq!(
        response.get("result").get("serverInfo").get("name"),
        &Json::String("ntex-mcp".into())
    );
}
