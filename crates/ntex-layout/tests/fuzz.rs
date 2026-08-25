//! 输入层 fuzz：畸形输入不 panic 契约的机器验证（M1-4）。
//!
//! 现代解释器定位问题的基础设施（plan.md P1 fuzz 目标）：确定性 PRNG
//! 多策略生成任意字节输入，喂完整输入路径（扫描 → 展开 → 排版），断言
//! `typeset_bytes` 对任意输入不 panic（返回 `Ok` 或 `Err` 均合法）。
//!
//! - 普通测试（`cargo test --workspace` 默认运行）：每策略 500 轮快速冒烟；
//! - `#[ignore]` 深 fuzz（CI real-engine job 显式 `-- --ignored` 运行）：
//!   每策略 5_000 轮。
//!
//! seed 固定 → 结果可复现；新 panic 路径出现即红，回归可见。

use fastrand::Rng;
use ntex_layout::Typesetter;

/// 快速冒烟轮次（跟随 `cargo test --workspace`）。
const QUICK_ROUNDS: usize = 500;
/// 深 fuzz 轮次（CI 显式运行）。
const DEEP_ROUNDS: usize = 5_000;
/// 确定性 seed（固定 → 可复现）。
const SEED: u64 = 0x4E54_4558_5EED;

/// 输入生成策略。
enum Strategy {
    /// 全范围 0-255 字节：覆盖扫描/编码/高位字节路径。
    RandomBytes,
    /// ASCII 可打印 + TeX 特殊字符：覆盖控制序列/参数/分组路径。
    TexAscii,
    /// TeX 原语片段随机拼接：覆盖原语/宏/条件/数学路径。
    Fragments,
    /// 随机嵌套大括号：覆盖组深度/状态机路径。
    NestedBraces,
}

/// 核心断言：`typeset_bytes` 对任意输入不 panic（Ok/Err 均合法）。
fn assert_no_panic(bytes: &[u8]) {
    let mut ts = Typesetter::new();
    let _ = ts.typeset_bytes(bytes);
}

/// 按策略生成一个随机源码字节串。
fn gen_input(rng: &mut Rng, strategy: &Strategy) -> Vec<u8> {
    match strategy {
        Strategy::RandomBytes => {
            let len = rng.usize(0..512);
            (0..len).map(|_| rng.u8(..)).collect()
        }
        Strategy::TexAscii => {
            const CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789 \
\\{}[]$&#^_~%\n\t`'\"<>|/:-=+*.,;!?()@";
            let len = rng.usize(0..512);
            (0..len).map(|_| CHARS[rng.usize(0..CHARS.len())]).collect()
        }
        Strategy::Fragments => {
            const FRAGMENTS: &[&str] = &[
                "\\def\\foo#1{#1}",
                "\\let\\a=\\b",
                "\\iftrue",
                "\\iffalse",
                "\\else",
                "\\fi",
                "\\ifx",
                "\\ifnum",
                "\\ifcat",
                "\\ifdim",
                "$",
                "{",
                "}",
                "\\hbox",
                "\\vbox",
                "\\hbox to 10pt{}",
                "\\count0=",
                "\\dimen0=",
                "\\skip0=",
                "\\advance",
                "\\multiply",
                "\\divide",
                "\\write16{}",
                "\\write",
                "\\input",
                "\\font\\f=",
                "\\relax",
                "\\par",
                "\\expandafter",
                "\\noexpand",
                "\\the",
                "\\show",
                "\\message",
                "\\chardef",
                "\\mathchar",
                "\\catcode`\\a=12",
                "\\uppercase",
                "\\lowercase",
                "\\csname",
                "\\endcsname",
                "\\numexpr",
                "\\dimexpr",
                "\\glueexpr",
                "\\muexpr",
                "\\romannumeral",
                "\\string",
                "\\meaning",
                "\\jobname",
                "\\shipout",
                "\\copy",
                "\\box",
                "\\unhbox",
                "\\vsplit",
                "\\insert",
                "\\mark",
                "\\penalty",
                "\\kern",
                "\\hskip",
                "\\vskip",
                "\\leaders",
                "\\hrule",
                "\\vrule",
                "\\halign",
                "\\valign",
                "\\noindent",
                "\\indent",
                "\\over",
                "\\atop",
                "\\above",
                "\\sqrt",
                "\\left",
                "\\right",
                "\\limits",
                "\\nolimits",
                " ",
                "123",
                "x",
                "%",
                "^^",
                "~",
            ];
            let mut parts: Vec<&[u8]> = Vec::new();
            for _ in 0..rng.usize(1..16) {
                parts.push(FRAGMENTS[rng.usize(0..FRAGMENTS.len())].as_bytes());
            }
            parts.concat()
        }
        Strategy::NestedBraces => {
            let mut out = Vec::new();
            let depth = rng.usize(0..16);
            out.resize(depth, b'{');
            let inner = rng.usize(0..32);
            out.resize(depth + inner, b'a');
            out.resize(depth + inner + depth, b'}');
            out
        }
    }
}

/// 以固定 seed 跑指定策略与轮次。
fn run_strategy(strategy: Strategy, rounds: usize, seed: u64) {
    let mut rng = Rng::with_seed(seed);
    for _ in 0..rounds {
        let bytes = gen_input(&mut rng, &strategy);
        assert_no_panic(&bytes);
    }
}

#[test]
fn quick_random_bytes() {
    run_strategy(Strategy::RandomBytes, QUICK_ROUNDS, SEED);
}

#[test]
fn quick_tex_ascii() {
    run_strategy(Strategy::TexAscii, QUICK_ROUNDS, SEED + 1);
}

#[test]
fn quick_fragments() {
    run_strategy(Strategy::Fragments, QUICK_ROUNDS, SEED + 2);
}

#[test]
fn quick_nested_braces() {
    run_strategy(Strategy::NestedBraces, QUICK_ROUNDS, SEED + 3);
}

/// 深 fuzz：全部策略、大轮次。CI real-engine job 用 `-- --ignored` 运行。
#[test]
#[ignore]
fn deep_fuzz_all() {
    for (i, strategy) in [
        Strategy::RandomBytes,
        Strategy::TexAscii,
        Strategy::Fragments,
        Strategy::NestedBraces,
    ]
    .into_iter()
    .enumerate()
    {
        run_strategy(strategy, DEEP_ROUNDS, SEED + i as u64);
    }
}
