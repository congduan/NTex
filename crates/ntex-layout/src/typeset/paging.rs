/// 页标签（tex.web `ship_out` L12694-12699）：`[` + count0 起逐段点分，**遇 0
/// 截断**——`j:=9; while (count(j)=0)and(j>0) do decr(j)` 取最高非零下标 j，
/// 打 count(0)..count(j)。全零时 j 停在 0 → 恒打 `[0]`（不省略整个标签）。
/// 负值照打（TRIP 参考 log：`[-5000.0.0.0.11.53110374]`）。
pub(crate) fn format_page_label(counts: &[i64; 10]) -> String {
    let mut j = 9;
    while counts[j] == 0 && j > 0 {
        j -= 1;
    }
    let mut out = String::from("[");
    for (k, c) in counts[..=j].iter().enumerate() {
        out.push_str(&c.to_string());
        if k < j {
            out.push('.');
        }
    }
    out.push(']');
    out
}

impl NodeBuilder {
    /// `\tracingoutput` 转录：shipout 时输出 "Completed box being shipped out [页号]"
    /// 后接完整盒树（tex.web ship_out L12687-12691：tracing_output>0 时 print_nl
    /// 标题；树用 show_box 同款格式——TRIP L42 起参考转录；TRIP 语义 diff 大头
    /// 之一）。页号 = count0..最高非零 count（tex.web L12694-12699）。
    fn trace_shipout(&mut self, b: &BoxNode) {
        if self.params.misc[27] <= 0 {
            return;
        }
        self.ship_seq += 1;
        let mut out = format!(
            "Completed box being shipped out {}\n",
            format_page_label(&self.page_counts)
        );
        showbox_format_box(b, 0, &self.fonts, &self.font_cs_names, &mut out);
        out.push('\n');
        let _ = self.write16(out);
    }

    /// `\count<n>=<值>`（含 `\advance`、组结束回滚；idx < 10 才推送，输出例程刀 5）：
    /// 维护页号链镜像，shipout 边界打标签 / 组装 DVI bop 计数用。
    fn count_changed(&mut self, idx: usize, value: i64) {
        if idx < 10 {
            self.page_counts[idx] = value;
        }
    }

    fn accept_page(&mut self, p: BoxNode) {
        let p = self.insert_accumulate(p);
        if self.output_defined {
            self.pending_pages.push_back(p);
        } else {
            self.ship_page(p);
        }
    }

    /// fire_up 的 insert 计账（tex.web fire_up
    /// `@<Either insert the material specified by node |p| into box |n|...@>`）：
    /// 页上 ins_node 的体进入 `box(class)`（插入累积盒，多个 insert 依次追加），
    /// ins_node 本身从页里删除——真 TeX 页盒树里没有 ins 节点，脚注由输出例程
    /// `\unvbox\footins` 回流（plain 默认例程 `\shipout\box255` 则直接丢弃）。
    ///
    /// NTex 体未排版 → 累积盒的子节点是 [`Node::Ins`] 本尊（token 体无损保留）：
    /// `\ifvoid<insert号>`/`\unvbox<insert号>`/`\box<insert号>` 因此走既有盒子
    /// 寄存器面（tex.web：`box(c)` 就是插入号 c 的累积盒，无需新寄存器文件）。
    /// box(255) 是页队列 → `\insert255` 已在 [`TokenSink::insert_node`] 报错改道 0。
    fn insert_accumulate(&mut self, mut p: BoxNode) -> BoxNode {
        let mut moved: Vec<(usize, Node)> = Vec::new();
        let mut children = Vec::with_capacity(p.children.len());
        for n in std::mem::take(&mut p.children) {
            match &n {
                Node::Ins { class, .. } => moved.push((*class, n)),
                _ => children.push(n),
            }
        }
        p.children = children;
        for (class, ins) in moved {
            let slot = self.box_view(class).cloned();
            // tex.web ensure_vbox：累积盒只许是 vbox；本实现遇 hbox/异型直接重建
            // （`\setbox150=\hbox{}` 与 `\insert150` 撞号在真 TeX 报
            // "Improper \hbox"，此处静默覆盖——insert 号与盒寄存器撞号见 survey §5.bis.4）
            let mut b = match slot {
                Some(b) if b.kind == BoxKind::VBox => b,
                _ => BoxNode::new_vbox(Vec::new()),
            };
            b.children.push(ins);
            // 裸写不入组级日志（tex.web：页面构建器对 box(n) 的写不走 set_box）
            self.write_box(class, Some(b));
        }
        p
    }

    /// 页面真正输出（tex.web `ship_out`）：转录标题 + 入 shipped 队列 + flush 标记。
    /// `page_shipped` 供引擎清零 `\deadcycles`（tex.web ship_out `dead_cycles:=0`）。
    /// 页号链（刀 5）：`\count0..9` 快照与页面对应入 [`Self::shipped_counts`]——
    /// DVI bop 的 10 计数字由此取值（tex.web ship_out `dvi_out(count(k))`）。
    fn ship_page(&mut self, p: BoxNode) {
        self.trace_shipout(&p);
        self.push_shipped(p);
        self.write_flush_pending = true;
        self.page_shipped = true;
    }

    /// 页面入 shipped 队列（计数快照同步入 [`Self::shipped_counts`]——两表恒同长，
    /// 增量回滚截断时按同一长度截）。
    fn push_shipped(&mut self, p: BoxNode) {
        self.shipped_counts.push(self.page_counts);
        self.shipped.push(p);
    }

    /// 各页面 shipout 边界的 `\count0..9` 快照（与 [`Self::shipped`] 一一对应）。
    pub fn shipped_page_counts(&self) -> &[[i64; 10]] {
        &self.shipped_counts
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
            // 只在贡献列表已空（全部材料已进页构建器、需要强制断出这最后一页）时
            // 才追加 eject 材料（空盒 + vfill + 强制惩罚）。若贡献列表非空，先让
            // feed_one 消化既有材料——它们往往已以强制惩罚收尾（上一次 fire_up
            // 在胶水处断页时把 [空盒, vfill, 惩罚] 留在贡献前端），再补一组会让
            // 残留三元组与新增材料自持循环（每次断出"空盒+胶水"页后仍剩三元组，
            // 无限冲页 OOM——M5 阶段三 consecutive 编辑后页面状态实测复现）。
            if self.lists[0].is_empty() {
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
            }
            // 产出一页则返回；材料全部入页但未触发断页 → 循环（贡献已空时补 eject
            // 节点再试，贡献非空时继续消化既有材料）。
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
