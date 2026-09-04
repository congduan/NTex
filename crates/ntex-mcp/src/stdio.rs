//! stdio 传输循环（MCP stdio 传输）：stdin 一行一帧 JSON-RPC，stdout 回帧。
//!
//! MCP stdio 约定：消息以换行分隔、不得内嵌裸换行；本循环逐行读取并 flush，
//! 保证客户端（AI 宿主）即时收到响应。EOF 正常退出；stdout 写失败（如管道
//! 关闭）同样以退出码 0 结束，避免与宿主断连时刷错误日志。日志一律走 stderr。

use std::io::{self, BufRead, Write};

use crate::server;

/// 阻塞式消息循环；返回进程退出码（EOF → 0）。
pub fn run() -> io::Result<i32> {
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Some(response) = server::handle_frame(&line) {
            writeln!(stdout, "{response}")?;
            stdout.flush()?;
        }
    }
    Ok(0)
}

/// 供 main 使用的入口：把 IO 错误转成 stderr 日志 + 非零退出码。
pub fn main_entry() -> i32 {
    match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("ntex-mcp：stdio 读写失败：{e}");
            1
        }
    }
}
