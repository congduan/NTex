//! wasm 时钟/线程门禁（**机器锁**）：生产代码里对 wasm32 不可用的 std API
//! 必须显式门控。
//!
//! ## 为什么需要这条锁
//!
//! `wasm32-unknown-unknown` 的 std 没有 OS 时钟，`Instant::now()` /
//! `SystemTime::now()` **调用即 panic**（`library/std/src/sys/time/unsupported.rs`：
//! "time not implemented on this platform"）；`std::thread::spawn`/`sleep` 同理
//! 不可用（无线程）。`[profile.release] panic = "abort"` 会把这类 panic 落成
//! **无位置信息的裸 trap**，宿主（浏览器/WebView）只报一句
//! `RuntimeError: Unreachable code should not be executed (evaluating 'wasm.compile_document(...)')`。
//!
//! native 上一切正常，所以这条路的违约**照不出来**：native 单测、深 fuzz、差分
//! 全绿，wasm 却一编译就死。
//!
//! 现场（2026-10-11）：`expand/macros.rs::scan_edef_body_pairs` 的
//! `NTEX_EDEF_PROG` 诊断计时起点原是无条件 `let prog_start = std::time::Instant::now();`
//! （e1a5bb33, 2026-09-28，ctex 第九刀）。`\edef`/`\xdef` 是全部 LaTeX 文档的
//! 必经路径 ⇒ wasm 工作台（Tauri B/C 档）**任何 LaTeX 文档一编译就 trap**，
//! 且静默两周——`docs/KNOWN-SIMPLIFICATIONS.md` 已登记本次修复。
//!
//! ## 判据（任一满足即放行）
//!
//! 1. **条目/语句级门控**：调用点同一行，或前 [`CFG_WINDOW`] 行内出现
//!    `#[cfg(not(target_arch = "wasm32"))]`（`param.rs` 的 `secs`、
//!    `expand/mod.rs` 的 `now_millis` / `step_start` 均属此形态）；
//! 2. **惰性求值**：同一行是 `.then(std::time::Instant::now)` 一类——诊断开关
//!    关着就永不触碰时钟（`expand/macros.rs` 的 `prog_start`）。
//!
//! ## 扫描口径
//!
//! 本 crate `src/**/*.rs`，跳过 `tests*.rs`（测试分片里用时钟无害）。内联
//! `#[cfg(test)] mod` 里若用了时钟会被误报——测试请照既有惯例放 `tests*.rs`
//! 分片文件。注释行（`//` / `///` / `//!`）只是"提到"这些名字，不计。

use std::path::{Path, PathBuf};

/// wasm32 上不可用的 std API（调用即 panic / 直接不支持）。
const WASM_UNAVAILABLE_CALLS: &[&str] = &[
    "Instant::now(",
    "SystemTime::now(",
    "thread::spawn(",
    "thread::sleep(",
];

/// 门控属性（`#[cfg(not(target_arch = "wasm32"))]` 的匹配子串）。
const CFG_NOT_WASM: &str = "cfg(not(target_arch = \"wasm32\"))";

/// 属性与调用点之间允许的最大行距。
///
/// 8 不是随手取的：`expand/mod.rs` 的 `step_start = Instant::now()` 在
/// `#[cfg(not(target_arch = "wasm32"))] if steps & 63 == 0 … { … }` 块内，
/// 距属性 7 行（中间的 `eprintln!` 多行实参把距离拉开了）。
const CFG_WINDOW: usize = 8;

/// 递归收集待扫描的 `.rs`（跳过 `tests*.rs`）。
fn collect_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = std::fs::read_dir(dir).expect("src 目录应可读");
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_sources(&path, out);
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if path.extension().and_then(|e| e.to_str()) != Some("rs") || name.starts_with("tests") {
            continue;
        }
        out.push(path);
    }
}

#[test]
fn production_code_gates_wasm_unavailable_std_calls() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    collect_sources(&root, &mut files);
    assert!(
        files.len() >= 15,
        "扫描面异常小（{} 个文件）——路径口径可能错了",
        files.len()
    );

    let mut offenders: Vec<String> = Vec::new();
    for file in &files {
        let src = std::fs::read_to_string(file).expect("源文件应可读");
        let lines: Vec<&str> = src.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            if line.trim_start().starts_with("//") {
                continue; // 注释里"提到"这些调用不算（本仓库文档习惯）。
            }
            let Some(call) = WASM_UNAVAILABLE_CALLS.iter().find(|c| line.contains(**c)) else {
                continue;
            };
            if line.contains(".then(") {
                continue; // 惰性：开关关着永不执行。
            }
            let from = i.saturating_sub(CFG_WINDOW);
            if lines[from..=i].iter().any(|l| l.contains(CFG_NOT_WASM)) {
                continue;
            }
            offenders.push(format!(
                "{}:{}  [{call}]  ← {}",
                file.strip_prefix(&root).unwrap_or(file).display(),
                i + 1,
                line.trim()
            ));
        }
    }

    assert!(
        offenders.is_empty(),
        "生产代码出现**未为 wasm32 门控**的时钟/线程调用（wasm 上一调用即 panic，\
         经 panic=abort 变成无位置信息的裸 trap，宿主只报 \
         `RuntimeError: Unreachable code should not be executed`）：\n{}\n\
         修法：该行/整段加 `#[cfg(not(target_arch = \"wasm32\"))]`，或改惰性求值\
         （`.then(std::time::Instant::now)`）让默认路径永不触碰时钟。",
        offenders.join("\n")
    );
}
