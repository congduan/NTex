use super::*;

// ── 栈转储仪器（诊断设施，2026-09-13）──────────────────────────────────────
//
// 这组测试钉的是**仪器本身**，不是引擎语义。理由：转储是「循环主体在哪」的
// 唯一现场证据，方向/标签一旦错，排查会得出反向结论——09-11 那次从转储读出
// 「栈顶 4997 个 TokenList 帧」，实为**栈底**视图（`stack[..head]` 配 `n-1-i`
// 标注），据此判断的循环主体是错的。仪器错了比没有仪器更坏。

/// 栈顶在前（降序）、栈底在后（升序），中间一段用省略标记。
#[test]
fn frame_dump_plan_top_then_bottom() {
    let (plan, omitted) = Expander::frame_dump_plan(100, 3, 2);
    assert_eq!(
        plan,
        vec![Some(99), Some(98), Some(97), None, Some(0), Some(1)]
    );
    assert_eq!(omitted, 95);
}

/// 退化输入：栈比 top 浅、空栈、只取栈底。
#[test]
fn frame_dump_plan_degenerate_cases() {
    // 栈比 top 浅：全部帧降序，无省略段。
    let (plan, omitted) = Expander::frame_dump_plan(2, 60, 5);
    assert_eq!(plan, vec![Some(1), Some(0)]);
    assert_eq!(omitted, 0);

    // 空栈：空计划。
    let (plan, omitted) = Expander::frame_dump_plan(0, 60, 5);
    assert!(plan.is_empty());
    assert_eq!(omitted, 0);

    // top=0（只看栈底）：省略标记在前，底段升序。
    let (plan, omitted) = Expander::frame_dump_plan(10, 0, 3);
    assert_eq!(plan, vec![None, Some(0), Some(1), Some(2)]);
    assert_eq!(omitted, 7);
}

/// 不变量：序号不越界、顶/底两段不重叠、覆盖数 + 省略数 = 栈深。
#[test]
fn frame_dump_plan_invariants() {
    for n in 0..40usize {
        for top in [0usize, 1, 5, 60] {
            for bottom in [0usize, 1, 5, 60] {
                let (plan, omitted) = Expander::frame_dump_plan(n, top, bottom);
                let idxs: Vec<usize> = plan.iter().flatten().copied().collect();
                assert!(
                    idxs.iter().all(|i| *i < n),
                    "序号越界：n={n} top={top} bottom={bottom} plan={plan:?}"
                );
                let mut uniq = idxs.clone();
                uniq.sort_unstable();
                uniq.dedup();
                assert_eq!(
                    uniq.len(),
                    idxs.len(),
                    "顶/底两段重叠：n={n} top={top} bottom={bottom} plan={plan:?}"
                );
                assert_eq!(
                    idxs.len() + omitted,
                    n,
                    "计数不守恒：n={n} top={top} bottom={bottom} plan={plan:?}"
                );
            }
        }
    }
}

/// 帧类型名必须覆盖全部帧型——尤其 `Macro`：宏递归爆栈现场全是宏帧，
/// 此前它落进 `Other`，等于转储在最需要它的地方失明。
#[test]
fn frame_kind_covers_every_variant() {
    let a = Token::char(Catcode::Letter, 'a' as u32);
    let body: TokenArray = Arc::from(vec![a]);
    let cs = Token::control_sequence(0);
    let code = Arc::new(crate::bytecode::compile(&[a]));
    let items: Arc<[(Token, bool)]> = Arc::from(vec![(a, false)]);
    let cases: [(InputFrame, &str); 9] = [
        (
            InputFrame::Source {
                bytes: Arc::from(vec![b'a']),
                pos: 0,
                state: ScanState::default(),
                line_starts: Arc::from(vec![0u32]),
                eof_mark: None,
            },
            "Source",
        ),
        (
            InputFrame::Macro {
                body: body.clone(),
                pos: 0,
                args: Vec::new(),
            },
            "Macro",
        ),
        (
            InputFrame::Bytecode {
                code,
                pc: 0,
                args: Vec::new(),
            },
            "Bytecode",
        ),
        (
            InputFrame::TokenList {
                items: items.clone(),
                pos: 0,
            },
            "TokenList",
        ),
        (
            InputFrame::MacroArg {
                items: Arc::from(body.iter().map(|&t| (t, false)).collect::<Vec<_>>()),
                pos: 0,
            },
            "MacroArg",
        ),
        (
            InputFrame::One {
                tok: cs,
                noexpand: false,
            },
            "One",
        ),
        (
            InputFrame::OutputRoutine {
                items: items.clone(),
                pos: 0,
            },
            "OutputRoutine",
        ),
        (
            InputFrame::AlignU {
                items: body.clone(),
                pos: 0,
            },
            "AlignU",
        ),
        (
            InputFrame::AlignV { items: body, pos: 0 },
            "AlignV",
        ),
    ];
    for (frame, expect) in &cases {
        assert_eq!(Expander::frame_kind(frame), *expect);
    }
}

/// 签名只含「类型 + 头部 token」，**不含进度**：同一宏体的不同进度必须归为
/// 同一族，否则「重复度」被进度差异打散，看不出谁在自复制。
#[test]
fn frame_marker_ignores_progress() {
    let e = Expander::new();
    let a = Token::char(Catcode::Letter, 'a' as u32);
    let b = Token::char(Catcode::Letter, 'b' as u32);
    let items: Arc<[(Token, bool)]> = Arc::from(vec![(a, false), (b, false)]);
    let fresh = InputFrame::TokenList {
        items: items.clone(),
        pos: 0,
    };
    let advanced = InputFrame::TokenList { items, pos: 2 };
    assert_eq!(e.frame_marker(&fresh), e.frame_marker(&advanced));
    assert_eq!(e.frame_marker(&fresh), "TokenList[a b]");
}

/// 逐帧详解必须同时给出**帧头**（这一帧是什么）与 **pos 现场**（正在展开哪几个
/// token）——只看帧头时，439 token 的长宏体/长实参上完全看不到当前现场。
#[test]
fn render_frame_head_shows_live_position() {
    let e = Expander::new();
    let a = Token::char(Catcode::Letter, 'a' as u32);
    let frame = InputFrame::MacroArg {
        items: Arc::from(vec![(a, false), (a, false), (a, false), (a, false)]),
        pos: 3,
    };
    let s = e.render_frame_head(&frame);
    assert!(s.starts_with("MacroArg[rem=1/4]"), "{s}");
    assert!(s.contains("head="), "{s}");
    assert!(s.contains("at="), "{s}");
}

/// token / span 渲染的基本形态（cs 名、可打印字符、空白、控制字符、`#n`）。
#[test]
fn render_token_forms() {
    let mut e = Expander::new();
    let id = e.intern.intern("q_stop");
    assert_eq!(e.render_token(Token::control_sequence(id)), "\\q_stop");
    assert_eq!(e.render_token(Token::char(Catcode::Letter, 'A' as u32)), "A");
    assert_eq!(e.render_token(Token::char(Catcode::Space, b' ' as u32)), "␣");
    assert_eq!(e.render_token(Token::char(Catcode::Other, 0x0C)), "^0C");
    assert_eq!(e.render_token(Token::macro_param(2)), "#2");
    assert_eq!(e.render_token(Token::end_group()), "}");

    let items = vec![
        Token::char(Catcode::Letter, b'a' as u32),
        Token::char(Catcode::Letter, b'b' as u32),
        Token::control_sequence(id),
    ];
    assert_eq!(e.render_token_span(&items, 0, 8), "a b \\q_stop");
    assert_eq!(e.render_token_span(&items, 1, 8), "b \\q_stop");
    assert_eq!(e.render_token_span(&items, 0, 2), "a b …(+1)");
    assert_eq!(e.render_token_span(&items, 3, 8), "");
    // 越界起点不 panic（诊断设施跑在畸形状态下，不得自身触发 panic）
    assert_eq!(e.render_token_span(&items, 99, 8), "");
}

/// 转储总入口在未开开关时必须是零副作用（否则污染正常转录）；
/// 开着时也必须自身安全（不 panic、不越界）——它跑在畸形输入状态下。
#[test]
fn dump_input_stack_is_safe() {
    let mut e = Expander::new();
    e.dump_input_stack("unit-test");
    e.stack.push(InputFrame::TokenList {
        items: Arc::from(vec![(Token::char(Catcode::Letter, b'x' as u32), false)]),
        pos: 0,
    });
    e.dump_input_stack("unit-test");
}
