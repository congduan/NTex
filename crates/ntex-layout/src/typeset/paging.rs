impl NodeBuilder {
    /// `\tracingoutput` 转录：shipout 时输出 "Completed box being shipped out [页号]"
    /// 后接完整盒树（tex.web ship_out L12687-12691：tracing_output>0 时 print_nl
    /// 标题；树用 show_box 同款格式——TRIP L42 起参考转录；TRIP 语义 diff 大头
    /// 之一）。页号 = count0..最高非零 count（tex.web L12694-12699），暂 0 占位。
    fn trace_shipout(&mut self, b: &BoxNode) {
        if self.params.misc[27] <= 0 {
            return;
        }
        self.ship_seq += 1;
        let mut out =
            format!("Completed box being shipped out [0.0.0.0.{}]\n", self.ship_seq);
        showbox_format_box(b, 0, &self.fonts, &self.font_cs_names, &mut out);
        out.push('\n');
        let _ = self.write16(out);
    }

    fn accept_page(&mut self, p: BoxNode) {
        if self.output_defined {
            self.pending_pages.push_back(p);
        } else {
            self.trace_shipout(&p);
            self.shipped.push(p);
            self.write_flush_pending = true;
        }
    }

    /// 结束开放段落：Knuth-Plass 折行成行 hbox 并追加到上层列表（行间插 interline glue）。
    /// 段落末尾：裁剪尾部可丢弃节点 + 追加 `\parfillskip`（0pt plus 1fil，末行无限拉伸）。
    /// 返回末行**自然宽度**（未拉伸前；显示数学 short 判定用），空段落返回 None。
    /// 但 feed_one 在 fire_up 后即返回（触发节点残留在贡献前端），若不主动
    /// 清除，eject 循环会把这些可丢弃节点误判为"有待冲材料"而反复追加
    /// eject 节点 → 空页死循环。
    fn drop_empty_page_discardables(&mut self) {
        if !self.page.is_empty() {
            return;
        }
        while let Some(n) = self.lists[0].first() {
            if matches!(n, Node::Glue { .. } | Node::Kern { .. } | Node::Penalty { .. }) {
                self.lists[0].remove(0);
            } else {
                break;
            }
        }
    }

    /// M3-5-2 `\end` 冲页（tex.web `its_all_over`）：页或贡献非空时追加
    /// `\hbox to \hsize{}\vfill\penalty-'10000000000` 强制断页；触发节点（penalty）
    /// 面对新空页被页面构建器丢弃，故不产生多余空页。
    ///
    /// 增量版：每调用最多冲出一页（返回是否冲出）；`finish` 与输出例程
    /// 交错执行——页面经 [`Self::accept_page`] 路由（box255+例程 或 直通 shipout）。
    fn eject_one_page(&mut self) -> Result<bool> {
        loop {
            if self.page.is_empty() && self.lists[0].is_empty() {
                return Ok(false);
            }
            let hsize = self.params.hsize;
            let mut empty_box = BoxNode::new_hbox(Vec::new());
            empty_box.width = hsize; // \hbox to \hsize{}
            self.lists[0].push(Node::Box(empty_box));
            self.lists[0].push(Node::Glue {
            name: None,                width: 0,
                stretch: 1,
                shrink: 0,
                stretch_order: GLUE_ORDER_FIL,
                shrink_order: 0,
            });
            self.lists[0].push(Node::Penalty {
                penalty: -(1 << 30), // \penalty-'10000000000
            });
            // 产出一页则返回；材料全部入页但未触发断页 → 补充 eject 节点再试
            if let Some(p) = self.page.feed_one(&mut self.lists[0], &self.params) {
                self.accept_page(p);
                // ETRIP 冲刺：断页 marks 轮转（top = 旧 bot，first 清空，bot 保留继承）
                self.rotate_marks();
                self.drop_empty_page_discardables();
                return Ok(true);
            }
        }
    }
}
