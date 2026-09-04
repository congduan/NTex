//! `ntex-mcp` 可执行入口：启动 stdio JSON-RPC 循环（M9 形态①）。

fn main() -> std::process::ExitCode {
    std::process::ExitCode::from(ntex_mcp::stdio::main_entry() as u8)
}
