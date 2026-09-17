impl NodeBuilder {
    fn close_paragraph(&mut self) -> Option<i64> {
        let Some(mut children) = self.lists.pop() else {
            return None; // 防御：列表栈异常为空（错误恢复弹栈失衡）
        };
        self.list_modes.pop();
        while children.last().is_some_and(Node::is_discardable) {
            children.pop();
        }
        if children.is_empty() {
            return None; // 空段落不产生盒子
        }
        // M4-6 断字：patterns 或异常词表非空时对字母 run 插入 discretionary 节点
        if !self.patterns.is_empty() || !self.hyph_exceptions.is_empty() {
            children = self.hyphenate_paragraph(children);
        }
        // M9 中文刀 5：汉字字间断点（`\cjkbreakmode`）
        // 默认关——TeX 原语义里汉字之间既无胶水也无断点，长中文行只能
        // Overfull 出页；打开后在**可断**字间插零宽胶水（含行首/行尾禁则），
        // 折行器据此折行、行盒据其拉伸对齐。与断字同层：都在水平列表完整、
        // 折行尚未开始的当口改列表。
        if self.params.misc[ntex_core::param::MISC_CJK_BREAK_MODE] > 0 {
            children = crate::linebreak::insert_cjk_glue(&children);
        }
        // 段落末尾：裁剪尾部可丢弃节点 + 追加 `\parfillskip`（默认 0pt plus 1fil，
        // 末行无限拉伸；`\parfillskip=0pt` 时末行保持自然宽度）。
        let pf = self.params.parfillskip;
        children.push(Node::Glue {
            name: None,            width: pf.width,
            stretch: pf.stretch,
            shrink: pf.shrink,
            stretch_order: if pf.stretch != 0 { GLUE_ORDER_FIL } else { 0 },
            shrink_order: 0,
        });
        // \tracingparagraphs（misc 29）：折行追踪输出到转录（tex.web @firstpass 等）
        let tracing = self.params.misc[29] > 0;
        let (lines, trace) = knuth_plass(
            &children,
            self.params.hsize,
            self.params.tolerance,
            self.params.misc[13], // \pretolerance（-1 时跳过第一遍）
            tracing,
        );
        if tracing && !trace.is_empty() {
            let _ = self.write16(trace);
        }
        // 行间惩罚（tex.web interline_penalty 语义）：除首行外每行前插入
        // \interlinepenalties 数组值（按行序索引，超出用末值；数组空用
        // \interlinepenalty 单值参数）。节点留在列表 → \lastpenalty 可读
        // （ETRIP L1243 \3 检查），断页器也据此决策。
        let interline = self.penalty_arrays[0].clone();
        let interline_default = self.params.interlinepenalty;
        let mut last_natural: Option<i64> = None;
        for (line_no, (s, e)) in lines.into_iter().enumerate() {
            if line_no > 0 {
                let p = interline
                    .get((line_no - 1).min(interline.len().saturating_sub(1)))
                    .copied()
                    .unwrap_or(interline_default);
                self.push_node(Node::Penalty { penalty: p });
            }
            // 断点胶水已在折行时排除；末行保留 \parfillskip（fil 拉伸填满行宽）。
            // discretionary 物化：行首补前一断点的 post、行内用 replace、行尾断点补 pre
            let mut line: Vec<Node> = Vec::new();
            // 行首 `\leftskip`（tex.web：每行行首 leftskip glue——参考行结构
            // `.\glue(\leftskip) 3.0 ...`；此前行盒只有内容缺左右 skip）
            let ls = self.params.leftskip;
            if ls.width != 0 || ls.stretch != 0 || ls.shrink != 0 {
                line.push(Node::Glue {
                    name: Some("leftskip"),
                    width: ls.width,
                    stretch: ls.stretch,
                    shrink: ls.shrink,
                    stretch_order: 0,
                    shrink_order: 0,
                });
            }
            if s > 0 {
                if let Node::Discretionary { post, .. } = &children[s - 1] {
                    line.extend(post.iter().cloned());
                }
            }
            for node in &children[s..e] {
                match node {
                    Node::Discretionary { replace, .. } => line.extend(replace.iter().cloned()),
                    other => line.push(other.clone()),
                }
            }
            if let Some(Node::Discretionary { pre, .. }) = children.get(e) {
                line.extend(pre.iter().cloned());
            }
            // 行尾 `\rightskip`（tex.web：每行行尾 rightskip glue——末行的
            // \parfillskip 在 children 内、rightskip 在其后；参考 `.\\glue(\\rightskip) 0.0`）
            let rs = self.params.rightskip;
            if rs.width != 0 || rs.stretch != 0 || rs.shrink != 0 {
                line.push(Node::Glue {
                    name: Some("rightskip"),
                    width: rs.width,
                    stretch: rs.stretch,
                    shrink: rs.shrink,
                    stretch_order: 0,
                    shrink_order: 0,
                });
            }
            last_natural = Some(hbox_dimensions(&line).width);
            // 折行警告（tex.web §922-930）：行自然宽超 \hsize 且**收缩不足**才 Overfull
            // ——tex.web 的 overfull 判定是行 badness 达 inf_bad（收缩/拉伸无法容纳），
            // 而非"自然宽 > \hsize"：glue 收缩能把超宽压回 \hsize 时（badness 有限）
            // 不算 overfull。故此处需比较超宽量与该行 glue 总可收缩量。
            // `(<超宽> too wide) in paragraph at lines <a>--<b>`（\par 行号；
            // 段落开始行暂用 \par 行——单行段落精确，跨行段落待 D 组行号追踪）。
            let natural = hbox_dimensions(&line).width;
            if natural > self.params.hsize {
                // 行总可收缩量（普通阶 shrink_order==0；高阶 shrink 不参与有限行）
                let mut shrinkable = 0i64;
                for node in &line {
                    if let Node::Glue {
                        shrink,
                        shrink_order: 0,
                        ..
                    } = node
                    {
                        shrinkable += *shrink;
                    }
                }
                let over = natural - self.params.hsize;
                if over > shrinkable {
                    let _ = self.write16(format!(
                        "Overfull \\hbox ({} too wide) in paragraph at lines {}--{}\n",
                        ntex_core::register::format_dimen(over),
                        self.last_par_line,
                        self.last_par_line
                    ));
                }
            }
            // 行盒 = `\hbox to \hsize`（tex.web line_break：恰好 hsize 宽，胶水拉伸/收缩）
            self.push_box(Node::Box(hpack(&line, self.params.hsize)));
        }
        last_natural
    }

    /// M4-6 断字：对连续字母 run（同字体、ASCII 字母）调用模式表计算断点，
    /// 在断点后插入 discretionary 节点（`pre` 为连字符，`post`/`replace` 为空——
    /// 字母留在主列表，未断时连字符不计宽，断点处行尾补连字符）。
    ///
    /// `\lefthyphenmin`/`\righthyphenmin` 过滤（tex.web §924 `hyphenate` 的
    /// `found:` 标签）：断点 `j`（`j` = 断点左侧字母数）仅在
    /// `l_hyf <= j <= hn - r_hyf` 时保留，异常词表与模式表**同受此限**——
    /// tex.web 两条路径都汇到同一个 `found:`，先 `hyf[0..l_hyf-1]:=0`
    /// 再 `hyf[hn-j]:=0 (j=0..r_hyf-1)`。故 `\hyphenation{-abcde-}` 标出的
    /// 词首/词尾断点在 `l_hyf,r_hyf >= 1` 时一律被清掉（`l_hyf` 经 `norm_min`
    /// 钳到 `>= 1`，词首断点恒不可达）；词长 `hn < l_hyf + r_hyf` 直接不断字。
    fn hyphenate_paragraph(&self, children: Vec<Node>) -> Vec<Node> {
        // tex.web `norm_min`：`<=0` → 1、`>=63` → 63（见 §927）
        let norm_min = |v: i64| -> usize { v.clamp(1, 63) as usize };
        let l_hyf = norm_min(self.params.misc[ntex_core::param::MISC_LEFT_HYPHEN_MIN]);
        let r_hyf = norm_min(self.params.misc[ntex_core::param::MISC_RIGHT_HYPHEN_MIN]);
        let mut out: Vec<Node> = Vec::with_capacity(children.len());
        let n = children.len();
        let mut i = 0;
        while i < n {
            // 收集字母 run [i, j)
            let run_start = i;
            if let Node::Char { font, charcode, .. } = &children[i] {
                if is_alpha(*charcode) {
                    let run_font = *font;
                    let mut j = i + 1;
                    while j < n {
                        match &children[j] {
                            Node::Char { font: f, charcode: c, .. }
                                if *f == run_font && is_alpha(*c) =>
                            {
                                j += 1;
                            }
                            _ => break,
                        }
                    }
                    let letters: Vec<u8> = children[run_start..j]
                        .iter()
                        .map(|node| match node {
                            Node::Char { charcode, .. } => *charcode as u8,
                            _ => unreachable!("run 内必为 Char"),
                        })
                        .collect();
                    let hn = letters.len();
                    // 异常词优先（精确匹配小写字母）；否则走模式表
                    let raw = match self.exception_breaks(&letters) {
                        Some(b) => b,
                        None if hn < l_hyf + r_hyf => Vec::new(), // 词过短：tex.web `hn<l_hyf+r_hyf`
                        None => self.patterns.hyphenate(&letters),
                    };
                    // tex.web `found:`：仅保留 `l_hyf <= j <= hn - r_hyf`
                    let breaks: Vec<usize> = raw
                        .into_iter()
                        .filter(|&j| j >= l_hyf && j + r_hyf <= hn)
                        .collect();
                    let mut bi = 0;
                    for (k, node) in children[run_start..j].iter().enumerate() {
                        out.push(node.clone());
                        // 断点 = 第 k 个字母之后（位置 k+1）：插入 discretionary
                        if bi < breaks.len() && breaks[bi] == k + 1 {
                            out.push(self.make_discretionary(run_font));
                            bi += 1;
                        }
                    }
                    i = j;
                    continue;
                }
            }
            out.push(children[i].clone());
            i += 1;
        }
        out
    }

    /// 异常词表查词：小写字母精确匹配 → 返回其允许断点（可含 0 = 词首、len = 词尾，
    /// 由调用方的 `l_hyf`/`r_hyf` 过滤，与 tex.web `found:` 同口径）。
    fn exception_breaks(&self, letters: &[u8]) -> Option<Vec<usize>> {
        self.hyph_exceptions
            .iter()
            .find(|(w, _)| w.as_slice() == letters)
            .map(|(_, b)| b.clone())
    }

    /// 断字 discretionary 节点：`pre` = 连字符（charcode 45，当前 run 字体度量），
    /// `post`/`replace` 为空（字母留在主列表）。
    fn make_discretionary(&self, font: FontId) -> Node {
        let (w, h, d) = self.fonts.metrics(font, 45);
        Node::Discretionary {
            pre: vec![Node::Char {
                font,
                charcode: 45,
                width: w,
                height: h,
                depth: d,
            }],
            post: Vec::new(),
            replace: Vec::new(),
        }
    }
}
