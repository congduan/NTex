impl NodeBuilder {
    fn close_paragraph(&mut self) -> Option<i64> {
        let mut children = self.lists.pop().expect("段落列表");
        self.list_modes.pop();
        while children.last().is_some_and(Node::is_discardable) {
            children.pop();
        }
        if children.is_empty() {
            return None; // 空段落不产生盒子
        }
        // M4-6 断字：\patterns 非空时对字母 run 插入 discretionary 节点（折行断点）
        if !self.patterns.is_empty() {
            children = self.hyphenate_paragraph(children);
        }
        children.push(Node::Glue {
            width: 0,
            stretch: 1,
            shrink: 0,
            stretch_order: GLUE_ORDER_FIL,
            shrink_order: 0,
        });
        let lines = knuth_plass(&children, self.params.hsize, self.params.tolerance);
        let mut last_natural: Option<i64> = None;
        for (s, e) in lines {
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
                    let breaks = self.patterns.hyphenate(&letters);
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
