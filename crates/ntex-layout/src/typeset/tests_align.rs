use super::*;

// ── \halign to/spread 摊派（S2/S7 修复对拍，2026-09-08）────────────────
// tex.web fin_align 真语义：列宽保持自然最大值；`to` 锁定行宽、`spread`
// 以最宽行自然宽为基准增量；差额由行级 hpack 的 glue set 摊给行内
// tabskip 胶水（4 阶）。旧行为：to 差额均摊进列宽、spread 被丢弃。

/// 提取页面顶层节点的盒子宽度（sp）。
///
/// tex.web fin_align「Insert the current list into its environment」(L15989)
/// 后行盒**直接**进外层竖列表——没有对齐封装盒；单行对齐时顶层唯一节点
/// 就是行盒本身。
fn top_box_width(nodes: &[Node]) -> i64 {
    assert_eq!(nodes.len(), 1, "应产出单个对齐盒：{nodes:?}");
    as_box(&nodes[0]).width
}

#[test]
fn halign_to_locks_row_width_tabskip_absorbs() {
    // 自然宽 = 10pt(规则) + 10pt(规则) = 20pt；to 200pt → 差额 180pt
    // 摊给 3 个边界 tabskip（各 plus1fil）→ 每个 +60pt。列宽不变。
    let nodes = typeset(concat!(
        "\\tabskip=0pt plus1fil ",
        "\\halign to 200pt{\\hfil#\\hfil&\\hfil#\\hfil\\cr ",
        "\\vrule width 10pt & \\vrule width 10pt \\cr}",
    ))
    .unwrap();
    // 结构（29adbbd 起，tex.web fin_align L15989）：行盒直接进外层竖列表，
    // 不再 vpack 成单只对齐封装盒（真 TeX \showbox 同构：`\halign to` 的
    // 每行是 unset box 设宽后逐行封装，`\vbox{\halign…}` 的 children 就是
    // 行盒 + 行间胶水）。单行 → 顶层唯一节点即行盒。
    assert_eq!(nodes.len(), 1, "单行对齐：顶层即行盒本身：{nodes:?}");
    let row = as_box(&nodes[0]);
    assert_eq!(row.width, 200 * SP_PER_PT, "行宽锁定 200pt");
    // 行内胶水吸收差额
    let mut glue_widths = Vec::new();
    for c in &row.children {
        if let Node::Glue { width, .. } = c {
            glue_widths.push(*width);
        }
    }
    assert_eq!(
        glue_widths,
        vec![
            60 * SP_PER_PT,
            60 * SP_PER_PT,
            60 * SP_PER_PT
        ],
        "3 个 fil tabskip 各吸收 60pt：{glue_widths:?}"
    );
}

#[test]
fn halign_spread_extends_natural_width() {
    // spread 20pt：目标 = 20pt 自然 + 20pt = 40pt（旧实现 spread 被丢弃 → 20pt）
    let nodes = typeset(concat!(
        "\\tabskip=0pt plus1fil ",
        "\\halign spread 20pt{\\hfil#\\hfil&\\hfil#\\hfil\\cr ",
        "\\vrule width 10pt & \\vrule width 10pt \\cr}",
    ))
    .unwrap();
    assert_eq!(
        top_box_width(&nodes),
        40 * SP_PER_PT,
        "spread 应在自然宽上加增量"
    );
}

#[test]
fn halign_natural_keeps_own_width() {
    // 无 to/spread：行保持自然宽（列宽自然最大值 10pt + 规则 2 个 + 胶水 0）
    let nodes = typeset(concat!(
        "\\halign{\\hfil#\\hfil&\\hfil#\\hfil\\cr ",
        "\\vrule width 10pt & \\vrule width 10pt \\cr}",
    ))
    .unwrap();
    assert_eq!(
        top_box_width(&nodes),
        20 * SP_PER_PT,
        "无规格对齐盒应为自然宽"
    );
}

// ── 命令层组定界 / cs 形态 mac_param（2026-09-19：LaTeX tabular 全线打通）──
// tex.web 的 `scan_left_brace`(L8196)、`align_peek`(L15517)、
// `get_preamble_token`(L15464) 判据全在**命令层**（`cur_cmd=left_brace` /
// `mac_param`），而 `\let\cs=<字符>` 型 cs token 读取时
// `cur_cmd:=eq_type(cur_cs)` 即该字符的 catcode、且 `get_x_token` 不会把它
// 展开成字符——故 cs 形态与字符形态等价。LaTeX 全依赖这条：
// `\ialign\bgroup`（`\@preamble`）、`\let\@sharp##`（`\@mkpream` 生成的
// preamble 里 `#` 写作 `\@sharp`）、`\endtabular` 的 `\crcr\egroup…`。
// 此前只认字符形态 → 任何 `tabular`/`array` 都在首个 `\halign` 处报
// "Missing { inserted" 并整篇排空（0 页）。

#[test]
fn halign_bgroup_egroup_alias_matches_char_form() {
    // `\let\bgroup={` / `\let\egroup=}` → 与 `\halign{…}` 完全等价
    // （LaTeX `\@preamble` = `\ialign \noexpand\@halignto \bgroup …`）。
    let alias = typeset(concat!(
        "\\let\\bgroup={\\let\\egroup=}",
        "\\vbox{\\halign\\bgroup\\hfil#\\hfil\\cr a\\cr b\\cr\\egroup}",
    ))
    .unwrap();
    let chars = typeset(concat!(
        "\\let\\bgroup={\\let\\egroup=}",
        "\\vbox{\\halign{\\hfil#\\hfil\\cr a\\cr b\\cr}}",
    ))
    .unwrap();
    assert_eq!(
        format!("{alias:?}"),
        format!("{chars:?}"),
        "\\bgroup/\\egroup 形态应与字符形态产出逐位一致"
    );
}

#[test]
fn halign_cs_mac_param_splits_template() {
    // `\let\hs=#` → 命令层 mac_param 也须算 u→v 分界（LaTeX `\@sharp`）。
    // 认不出会让整列落进 u 段并报 "Missing # inserted in alignment preamble"。
    let nodes = typeset(concat!(
        "\\let\\hs=#\\let\\bgroup={\\let\\egroup=}",
        "\\vbox{\\halign\\bgroup\\hfil\\hs\\hfil\\cr a\\cr\\egroup}",
    ))
    .unwrap();
    let vbox = as_box(&nodes[0]);
    assert_eq!(vbox.children.len(), 1, "单行");
    assert!(top_box_width(&nodes) > 0, "cs 形态 # 分出的 u/v 模板应产出宽度");
}

#[test]
fn halign_multispan_keeps_group_balance() {
    // `\omit\span\omit`（LaTeX `\multispan`/`\multicolumn` 内核）：tex.web
    // fin_col 的 span 分支跳过 `unsave; new_save_level(align_group)` 与
    // `init_span(p)`——续列既不新开单元组也不重置 cur_span。此前每列都开组
    // → 每个 `\span` 泄漏一个组 → 外层 `\vbox` 的 `}` 被吞、整页为空。
    let nodes = typeset(r"\vbox{\halign{#&#\cr a&b\cr \omit\span\omit c\cr}}").unwrap();
    assert_eq!(nodes.len(), 1, "外层 \\vbox 应收口成盒（单元组未泄漏）");
    let vbox = as_box(&nodes[0]);
    // 结构（29adbbd 起，tex.web fin_align L15989「Insert the current list
    // into its environment」+ fin_row 的 append_to_vlist）：vbox 的 children
    // 就是 [行盒, baselineskip 胶水, 行盒]——行间插 interline 胶水、首行前无。
    assert_eq!(
        vbox.children.len(),
        3,
        "vbox 内是两行盒 + 一段行间胶水：{:?}",
        vbox.children
    );
    // 跨列单元必须覆盖**两列**宽度（span_len=2），否则只占首列宽
    let row1 = as_box(&vbox.children[0]);
    assert!(
        matches!(vbox.children[1], Node::Glue { .. }),
        "行间应是 baselineskip interline 胶水：{:?}",
        vbox.children[1]
    );
    let row2 = as_box(&vbox.children[2]);
    assert_eq!(
        row2.width, row1.width,
        "跨列行应与两列行同宽（span_len 必须为 2，不能重置 cur_span）"
    );
}

// ── S6 模式合法性路由（tex.web main_control 大 case 键 abs(mode)+cur_cmd）──
// core 单测的 VecSink `mode_code` 恒返 1，路由只能在真 Typesetter 侧锁。

#[test]
fn align_mode_route_math_mode_halign_is_illegal() {
    // mmode+halign 无合法位 → report_illegal_case（GT pdftex 同首错；
    // 级联的 #/\cr 各自报错，错误数 GT 4 / NTex 3，登记差）。
    let mut ts = Typesetter::with_metrics(metrics);
    let _ = ts.typeset(r"$\halign{#\cr b\cr}$");
    let t = ts.take_transcript();
    assert!(
        t.contains("You can't use `\\halign' in math mode."),
        "GT 首错：{t}"
    );
}

#[test]
fn align_mode_route_restricted_hmode_off_save() {
    // 受限水平（\hbox 内）的 \halign：mode<0 → off_save，插配对 `}` 并报
    // "Missing } inserted"（GT 错误序：Missing } inserted → Too many }'s.）。
    let mut ts = Typesetter::with_metrics(metrics);
    let _ = ts.typeset(r"\hbox{\halign{#\cr b\cr}}");
    let t = ts.take_transcript();
    assert!(t.contains("! Missing } inserted."), "off_save 报错：{t}");
    assert!(t.contains("Too many }'s."), "GT 次错：{t}");
}

#[test]
fn valign_packs_columns_with_row_extents_and_tabskip() {
    // pdfTeX GT（showbox）：`\hbox{\valign{#\vfil&#\vfil\cr ...}}`
    // 的外层 hbox 直接挂 column vbox；列内结构是 tabskip / row box /
    // tabskip / row box / tabskip，row box 取该行最大高度+深度，宽度取列宽。
    let nodes = typeset(concat!(
        "\\hbox{\\valign{#\\vfil&#\\vfil\\cr ",
        "\\hrule height 5pt depth 2pt width 2pt&",
        "\\hrule height 10pt depth 4pt width 4pt\\cr}}",
    ))
    .unwrap();
    let outer = as_box(&nodes[0]);
    assert_eq!(outer.children.len(), 1, "\\valign 不应额外包一层 hbox");
    let col = as_box(&outer.children[0]);
    assert_eq!(col.kind, BoxKind::VBox);
    assert_eq!(col.width, 4 * SP_PER_PT);
    assert_eq!(col.height + col.depth, 21 * SP_PER_PT);
    assert_eq!(col.children.len(), 5, "首尾和中间 tabskip 都应保留");
    assert!(matches!(col.children[0], Node::Glue { width: 0, .. }));
    assert!(matches!(col.children[2], Node::Glue { width: 0, .. }));
    assert!(matches!(col.children[4], Node::Glue { width: 0, .. }));
    let row0 = as_box(&col.children[1]);
    let row1 = as_box(&col.children[3]);
    assert_eq!(row0.width, 4 * SP_PER_PT);
    assert_eq!(row0.height + row0.depth, 7 * SP_PER_PT);
    assert_eq!(row1.width, 4 * SP_PER_PT);
    assert_eq!(row1.height + row1.depth, 14 * SP_PER_PT);
}
