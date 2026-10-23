use super::*;

impl IoSink for NodeBuilder {
    // （report_error, report_help 走子 trait 默认实现，NodeBuilder 未覆写）
    /// `\patterns{...}`（M4-6）：解析文本为 Liang trie（后续段落折行按需断字）。
    fn patterns(&mut self, patterns: Vec<u8>) -> Result<()> {
        self.patterns = PatternTrie::parse(&patterns);
        Ok(())
    }
    /// `\hyphenation{...}`（ETRIP）：追加异常词表（小写字母 + 允许断点）。
    fn hyphenation(&mut self, words: Vec<(Vec<u8>, Vec<usize>)>) -> Result<()> {
        self.hyph_exceptions.extend(words);
        Ok(())
    }
    // ETRIP 冲刺：终端转录（\message/\show/\showthe/\write16）
    fn message(&mut self, text: String) -> Result<()> {
        self.io_state.transcript.push_str(&text);
        Ok(())
    }
    fn show(&mut self, text: String) -> Result<()> {
        self.io_state.transcript.push_str(&text);
        self.io_state.transcript.push('\n');
        Ok(())
    }
    /// `\showbox<n>`：把盒子寄存器内容格式化到转录（TeX show_box 风格）。
    fn write16(&mut self, text: String) -> Result<()> {
        self.io_state.transcript.push_str(&text);
        self.io_state.transcript.push('\n');
        Ok(())
    }
    /// `\showgroups`：把组上下文栈格式化为转录（诊断用）。
    fn showgroups(&mut self) -> Result<()> {
        let mut out = String::from("### begin group\n");
        for (i, g) in self.groups.iter().enumerate() {
            out.push_str(&format!(
                "level {i}: {:?} (code {})\n",
                g.kind,
                g.kind.code()
            ));
        }
        out.push_str("### end group\n");
        self.io_state.transcript.push_str(&out);
        Ok(())
    }
    /// `\showlists`：把当前列表简化为转录（诊断用；盒子内容递归展示）。
    fn showlists(&mut self) -> Result<()> {
        let mut out = String::from("### begin list\n");
        for (li, list) in self.lists.iter().enumerate() {
            out.push_str(&format!(
                "### list {li} (mode {:?}, {} nodes)\n",
                self.list_modes.get(li),
                list.len()
            ));
            for n in list {
                showbox_format_node(n, 1, &self.fonts, &self.font_cs_names, &mut out);
            }
        }
        out.push_str("### end list\n");
        self.io_state.transcript.push_str(&out);
        Ok(())
    }
    /// `\write<n>{...}`（非 \immediate）：whatsit 节点追加到当前列表（无维度）。
    fn whatsit(&mut self, text: String) -> Result<()> {
        self.append(Node::Whatsit {
            text,
            special: false,
        });
        Ok(())
    }
    /// `\special{...}`：whatsit 节点（`special: true`——shipout 经 DVI xxx
    /// 落后端；`\pdfrefximage` 的图片载荷同走此通道）。
    fn special(&mut self, text: String) -> Result<()> {
        self.append(Node::Whatsit {
            text,
            special: true,
        });
        Ok(())
    }
}
