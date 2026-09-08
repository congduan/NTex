use super::*;

// ── \halign to/spread 摊派（S2/S7 修复对拍，2026-09-08）────────────────
// tex.web fin_align 真语义：列宽保持自然最大值；`to` 锁定行宽、`spread`
// 以最宽行自然宽为基准增量；差额由行级 hpack 的 glue set 摊给行内
// tabskip 胶水（4 阶）。旧行为：to 差额均摊进列宽、spread 被丢弃。

/// 提取页面顶层节点的盒子宽度（sp）。
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
    assert_eq!(
        top_box_width(&nodes),
        200 * SP_PER_PT,
        "对齐盒应锁到 to 目标宽"
    );
    // 行内胶水吸收差额
    let vbox = as_box(&nodes[0]);
    assert_eq!(vbox.children.len(), 1, "单行");
    let row = as_box(vbox.children.first().unwrap());
    assert_eq!(row.width, 200 * SP_PER_PT, "行宽锁定 200pt");
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
