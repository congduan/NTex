//! 对齐自检（无头）：用 **`ntex-studio` 的真实引擎接线**（`src/engine.rs`，经
//! `#[path]` 引入，与 GUI 共用同一份代码）编译一个 .tex，打印口径/资产/字体/
//! 页数/转录——不开窗口即可验证 native studio 与 Tauri(wasm) 前端的 C 档能力
//! 是否对齐（LaTeX 模式、UTF-8 直写、CJK 回落、发行资产搜索链）。
//!
//! 运行：`cargo run -p ntex-studio --example align_probe -- samples/latex-sample2e-slim.tex`
//! （缺省用仓库 `samples/` 下的 LaTeX 样张；在仓库根运行以便定位 `assets/`）。

#[path = "../src/engine.rs"]
mod engine;

use std::path::PathBuf;

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "samples/latex-sample2e-slim.tex".to_owned());
    let src = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("读取 {path} 失败：{e}");
            std::process::exit(1);
        }
    };

    let setup = engine::Setup::install();
    println!("资产根：{:?}", setup.root);
    println!(
        "latex.fmt：{} · 字体 {} 个 · CJK 回落 {:?}",
        if setup.latex_ready() {
            "可用"
        } else {
            "不可用"
        },
        setup.fonts.len(),
        setup.fallback_font
    );
    if !setup.note.is_empty() {
        println!("提示：{}", setup.note);
    }

    let latex = setup.latex_ready() && ntex_layout::typeset::looks_like_latex(&src);
    let dirs: Vec<PathBuf> = PathBuf::from(&path)
        .parent()
        .map(PathBuf::from)
        .into_iter()
        .collect();
    let mut ts = setup.typesetter(latex, &dirs);
    let started = std::time::Instant::now();
    let outcome = ts.compile(&src);
    let transcript = ts.transcript().to_owned();

    println!(
        "口径：{} · 耗时 {} ms",
        if latex { "LaTeX" } else { "plain" },
        started.elapsed().as_millis()
    );
    match outcome {
        Ok(out) => println!(
            "OK：{} 页，{} 字体（含 nullfont 占位）",
            out.pages.len(),
            out.fonts.len()
        ),
        Err(e) => {
            let first = transcript
                .lines()
                .find(|l| l.trim_start().starts_with('!'))
                .unwrap_or("（转录无 ! 行）");
            println!("ERR：{e}\n首现场：{first}");
        }
    }
    // 长驻实例的幂等性自检：再编译一次同一源码——studio 的 250ms 防抖会反复
    // 走这条路（每次编辑后重编译），页数必须稳定（plain 预载只跑一次 + 段缓存
    // 重建 + fmt 状态不被二次导入污染）。
    match ts.compile(&src) {
        Ok(out) => println!("重编译：{} 页（应与首次一致）", out.pages.len()),
        Err(e) => println!("重编译 ERR：{e}"),
    }
    // 转录尾部（LaTeX 的 log 主体在结尾：页数/字体统计行最有诊断价值）。
    let tail: Vec<&str> = transcript.lines().rev().take(6).collect();
    if !tail.is_empty() {
        println!("--- 转录尾部 ---");
        for line in tail.iter().rev() {
            println!("{line}");
        }
    }
    // `--log <路径>`：全量转录落盘（首现场定位用——`!` 行往往不在尾部，
    // 级联报错会把真正的原因埋在前面；`--first` 则只打印首个 `!` 现场前后文）。
    let mut args = std::env::args().skip(1);
    let _ = args.next(); // 已消费的路径参数
    let mut dump: Option<String> = None;
    let mut first_only = false;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--log" => dump = args.next(),
            "--first" => first_only = true,
            _ => {}
        }
    }
    if let Some(out_path) = dump {
        match std::fs::write(&out_path, &transcript) {
            Ok(()) => println!(
                "转录已写出：{out_path}（{} 行）",
                transcript.lines().count()
            ),
            Err(e) => eprintln!("写转录失败：{e}"),
        }
    }
    if first_only {
        let lines: Vec<&str> = transcript.lines().collect();
        match lines.iter().position(|l| l.trim_start().starts_with('!')) {
            Some(i) => {
                println!("--- 首现场（第 {} 行）前后 12 行 ---", i + 1);
                let lo = i.saturating_sub(6);
                let hi = (i + 7).min(lines.len());
                for (k, line) in lines[lo..hi].iter().enumerate() {
                    println!("{:>5} | {line}", lo + k + 1);
                }
            }
            None => println!("--- 转录无 ! 行 ---"),
        }
    }
}
