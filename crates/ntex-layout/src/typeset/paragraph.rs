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
        // 段落末尾：裁剪尾部可丢弃节点 + 追加 `\parfillskip`（默认 0pt plus 1fil，
        // 末行无限拉伸；`\parfillskip=0pt` 时末行保持自然宽度）。
        let pf = self.params.parfillskip;
        children.push(Node::Glue {
            width: pf.width,
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
            last_natural = Some(hbox_dimensions(&line).width);
            // 折行警告（tex.web §922-930）：行自然宽超 \\hsize → Overfull
            // `(<超宽> too wide) in paragraph at lines <a>--<b>`（\par 行号；
            // 段落开始行暂用 \par 行——单行段落精确，跨行段落待 D 组行号追踪）。
            let natural = hbox_dimensions(&line).width;
            if natural > self.params.hsize {
                let over = natural - self.params.hsize;
                let _ = self.write16(format!(
                    "Overfull \\hbox ({} too wide) in paragraph at lines {}--{}\n",
                    ntex_core::register::format_dimen(over),
                    self.last_par_line,
                    self.last_par_line
                ));
            }
            // 行盒 = `\hbox to \hsize`（tex.web line_break：恰好 hsize 宽，胶水拉伸/收缩）
            self.push_box(Node::Box(hpack(&line, self.params.hsize)));
        }
        last_natural
    }

    /// M4-6 断字：对连续字母 run（同字体、ASCII 字母）调用模式表计算断点，
    /// 在断点后插入 discretionary 节点（`pre` 为连字符，`post`/`replace` 为空——
    /// 字母留在主列表，未断时连字符不计宽，断点处行尾补连字符）。
    fn hyphenate_paragraph(&self, children: Vec<Node>) -> Vec<Node> {
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
                    // 异常词优先（精确匹配小写字母）；否则走模式表
                    let breaks = match self.exception_breaks(&letters) {
                        Some(b) => b,
                        None => self.patterns.hyphenate(&letters),
                    };
                    let mut bi = 0;
                    // 异常词允许词首断点（`-q-` 的首 `-`）：首字母前插 discretionary
                    if breaks.first() == Some(&0) {
                        out.push(self.make_discretionary(run_font));
                        bi = 1;
                    }
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

    /// 异常词表查词：小写字母精确匹配 → 返回其允许断点（可含 0 = 词首、len = 词尾）。
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
