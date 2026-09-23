// ---------- \showbox 格式化（TeX show_box 风格） ----------
//
// 模块级自由函数：与 TokenSink 主实现分离，便于阅读与单测定位。
// 通过 `typeset/mod.rs` 末尾的 `include!("sink_showbox.rs");` 嵌入，
// 直接继承 mod.rs 已声明的 use/类型（BoxNode/BoxKind/Node/Fonts/...
// 全部在作用域内）。

/// 组内字符 token → 文本（insert/adjust 节点内容；cs 等忽略）。
fn toks_to_text(toks: &[Token]) -> String {
    toks.iter()
        .filter_map(|t| t.charcode())
        .filter_map(char::from_u32)
        .collect()
}

/// sp → pt 字符串（tex.web print_scaled：整数 + 最多 5 位小数去尾 0；
/// 余数 0 显示 `X.0`——如 `4.4`、`1055.44061`、`-1.0`）。
fn showbox_pt(sp: i64) -> String {
    let neg = sp < 0;
    let sp = sp.abs();
    let int = sp / SP_PER_PT;
    let rem = sp % SP_PER_PT;
    let sign = if neg { "-" } else { "" };
    if rem == 0 {
        return format!("{sign}{int}.0");
    }
    let frac = rem * 100_000 / SP_PER_PT;
    let frac_s = format!("{frac:05}").trim_end_matches('0').to_string();
    format!("{sign}{int}.{frac_s}")
}

/// 胶水阶名（tex.web print_glue：1=fil、2=fill、3=filll、0 无）。
fn order_name(order: u8) -> &'static str {
    match order {
        1 => "fil",
        2 => "fill",
        3 => "filll",
        _ => "",
    }
}

/// 胶水 spec（tex.web print_spec）：`10.0 plus 2.0fil`；无拉伸/收缩只有宽度。
/// insert 节点的 `split(...)` 用（真 TeX 走 print_spec(split_top_ptr)）。
fn showbox_glue_spec(g: &Glue) -> String {
    let mut s = showbox_pt(g.width);
    if g.stretch != 0 {
        s.push_str(&format!(
            " plus {}{}",
            showbox_pt(g.stretch),
            order_name(g.stretch_order)
        ));
    }
    if g.shrink != 0 {
        s.push_str(&format!(
            " minus {}{}",
            showbox_pt(g.shrink),
            order_name(g.shrink_order)
        ));
    }
    s
}

fn showbox_format_box(
    b: &BoxNode,
    depth: usize,
    fonts: &Fonts,
    cs_names: &[Option<String>],
    out: &mut String,
) {
    let p = ".".repeat(depth);
    let kind = match b.kind {
        BoxKind::HBox => "hbox",
        BoxKind::VBox => "vbox",
    };
    out.push_str(&format!(
        "{p}\\{kind}({}+{})x{}",
        showbox_pt(b.height),
        showbox_pt(b.depth),
        showbox_pt(b.width)
    ));
    // tex.web show_node_list：shift ≠ 0 时追加 ", shifted <dimen>"
    // （\raise/\lower 参考点位移、\vtop 基线移到首行、\moveleft/\moveright 水平位移）
    if b.shift != 0 {
        out.push_str(&format!(", shifted {}", showbox_pt(b.shift)));
    }
    out.push('\n');
    for c in &b.children {
        showbox_format_node(c, depth + 1, fonts, cs_names, out);
    }
}

fn showbox_format_node(
    n: &Node,
    depth: usize,
    fonts: &Fonts,
    cs_names: &[Option<String>],
    out: &mut String,
) {
    let p = ".".repeat(depth);
    match n {
        Node::Box(b) => showbox_format_box(b, depth, fonts, cs_names, out),
        Node::Char {
            font, charcode, ..
        } => {
            // TeX show_node_list：`.<字体名> <字符>`（如 `.\\trip 1`，字符直接显示）
            let c = char::from_u32(*charcode)
                .map(|c| c.to_string())
                .unwrap_or_else(|| format!("{charcode}"));
            let cs = cs_names
                .get(font.0 as usize)
                .and_then(|n| n.as_ref())
                .cloned()
                .unwrap_or_else(|| fonts.font_name(*font));
            out.push_str(&format!("{p}\\{} {c}\n", cs));
        }
        Node::Ligature {
            font,
            charcode,
            components,
            ..
        } => {
            // TeX show_node_list：`.<字体名> <结果> (ligature <组成>)`——组成字符
            // **直接连接**（参考 `..\\rip A (ligature AAA)`；etrip 的
            // `(ligature u|)` 是组成含竖线字符 124，不是分隔符）
            let c = char::from_u32(*charcode)
                .map(|c| c.to_string())
                .unwrap_or_else(|| format!("{charcode}"));
            let cs = cs_names
                .get(font.0 as usize)
                .and_then(|n| n.as_ref())
                .cloned()
                .unwrap_or_else(|| fonts.font_name(*font));
            let comps: String = components
                .iter()
                .map(|&b| char::from_u32(b as u32).map(|c| c.to_string()).unwrap_or_default())
                .collect();
            out.push_str(&format!("{p}\\{} {c} (ligature {comps})\n", cs));
        }
        Node::Glue {
            name,
            width,
            stretch,
            shrink,
            stretch_order,
            shrink_order,
        } => {
            let mut s = if let Some(n) = name {
                format!("{p}\\glue(\\{n}) {}", showbox_pt(*width))
            } else {
                format!("{p}\\glue {}", showbox_pt(*width))
            };
            if *stretch != 0 {
                s.push_str(&format!(
                    " plus {}{}",
                    showbox_pt(*stretch),
                    order_name(*stretch_order)
                ));
            }
            if *shrink != 0 {
                s.push_str(&format!(
                    " minus {}{}",
                    showbox_pt(*shrink),
                    order_name(*shrink_order)
                ));
            }
            s.push('\n');
            out.push_str(&s);
        }
        Node::Kern { width } => out.push_str(&format!("{p}\\kern{}\n", showbox_pt(*width))),
        Node::Penalty { penalty } => out.push_str(&format!("{p}\\penalty {}\n", penalty)),
        Node::Rule {
            width,
            height,
            depth,
        } => {
            // 未定宽度（NULL_FLAG，如 `\leaders\hrule` 引导）显示 `*`（tex.web print_rule_dimen）
            let w = if *width == ntex_core::NULL_FLAG {
                "*".to_string()
            } else {
                showbox_pt(*width)
            };
            out.push_str(&format!(
                "{p}\\rule({}+{})x{}\n",
                showbox_pt(*height),
                showbox_pt(*depth),
                w
            ));
        }
        Node::Leaders {
            kind,
            width,
            stretch,
            shrink,
            inner,
            ..
        } => {
            // TeX show_box：`\{kind} {胶水规格}` + 引导内容作为子节点递归显示
            // （tex.web "Display leaders"：node_list_display(leader_ptr)）。
            let name = match kind {
                LeadersKind::Leaders => "leaders",
                LeadersKind::Cleaders => "cleaders",
                LeadersKind::Xleaders => "xleaders",
            };
            let mut s = format!("{p}\\{name} {}", showbox_pt(*width));
            if *stretch != 0 {
                s.push_str(&format!(" plus {}", showbox_pt(*stretch)));
            }
            if *shrink != 0 {
                s.push_str(&format!(" minus {}", showbox_pt(*shrink)));
            }
            s.push('\n');
            out.push_str(&s);
            showbox_format_node(inner, depth + 1, fonts, cs_names, out);
        }
        Node::Discretionary { .. } => out.push_str(&format!("{p}\\discretionary\n")),
        Node::Direction { kind } => {
            let name = match kind {
                ntex_core::sink::DirectionKind::BeginL => "beginL",
                ntex_core::sink::DirectionKind::EndL => "endL",
                ntex_core::sink::DirectionKind::BeginR => "beginR",
                ntex_core::sink::DirectionKind::EndR => "endR",
            };
            out.push_str(&format!("{p}\\{name}\n"));
        }
        Node::Mark { class, text } => match class {
            Some(c) => out.push_str(&format!("{p}\\marks{c}{{{text}}}\n")),
            None => out.push_str(&format!("{p}\\mark{{{text}}}\n")),
        },
        Node::Ins {
            class,
            body,
            split_top_skip,
            split_max_depth,
            float_cost,
        } => {
            // tex.web show_node @<Display insertion |p|@>：
            // `\insert<class>, natural size <height>; split(<split_top_skip>,<depth>);
            //  float cost <float_cost>`，随后 node_list_display(ins_ptr) 逐层展开体
            // vlist（natural size = 体高 + 体深）。
            out.push_str(&format!(
                "{p}\\insert{class}, natural size {}; split({},{}); float cost {float_cost}\n",
                showbox_pt(body.height + body.depth),
                showbox_glue_spec(split_top_skip),
                showbox_pt(*split_max_depth),
            ));
            for c in &body.children {
                showbox_format_node(c, depth + 1, fonts, cs_names, out);
            }
        }
        Node::Adjust { text } => out.push_str(&format!("{p}\\vadjust {text}\n")),
        // whatsit 分型显示（tex.web short_display：\write / \special 两个名字）
        Node::Whatsit { text, special } => out.push_str(&format!(
            "{p}\\{} {text}\n",
            if *special { "special" } else { "write" }
        )),
        // 数学边界标记（tex.web math_node）：`.\\mathon`；`\\mathsurround` 非 0
        // 时 mathon 补 `, surrounded X`（tex.web 只在 mathon 显示——参考
        // trip.log `\\mathon, surrounded 143.0` + `\\mathoff` 无 surrounded）
        Node::MathOn { surrounded } => {
            if *surrounded != 0 {
                out.push_str(&format!(
                    "{p}\\mathon, surrounded {}\n",
                    showbox_pt(*surrounded)
                ));
            } else {
                out.push_str(&format!("{p}\\mathon\n"));
            }
        }
        Node::MathOff { .. } => out.push_str(&format!("{p}\\mathoff\n")),
    }
}
