use super::*;

impl PageSink for NodeBuilder {
    fn page_dimen(&self, index: u8) -> i64 {
        self.page_state
            .page
            .page_dimen(index, self.page_state.in_output_group)
    }
    fn output_defined(&mut self, defined: bool) -> Result<()> {
        self.page_state.output_defined = defined;
        if !defined {
            // 例程恢复未定义：未处理页面无法再经例程产出，直接丢弃（TeX 语义）
            self.page_state.pending_pages.clear();
            self.page_state.page.clear_fired_penalties();
        }
        Ok(())
    }
    fn output_pending(&self) -> bool {
        !self.page_state.pending_pages.is_empty()
    }
    fn take_output_pending(&mut self) -> bool {
        self.output_pending()
    }
    fn output_pending_count(&self) -> usize {
        self.page_state.pending_pages.len()
    }
    fn output_consumed(&mut self) -> bool {
        self.page_state.output_consumed_count > 0
    }
    fn discard_pending_pages(&mut self) {
        self.page_state.pending_pages.clear();
        // 队列与断点惩罚一一对应，丢弃页面时同步丢弃惩罚
        self.page_state.page.clear_fired_penalties();
    }
    fn output_break_penalty(&mut self) -> Option<i64> {
        // 每次注入取走队首页的断点惩罚（take 语义：无新点火不重复读同一值，
        // 与单槽口径一致）；队空回退最近一次点火值——\setbox255 直接种页没有
        // fire_up，tex.web 中 \outputpenalty 是只写不清的全局整数
        let _ = std::mem::take(&mut self.page_state.output_consumed_count);
        self.page_state.page.take_front_output_penalty()
    }
    fn insert_penalties(&self) -> i64 {
        // M6：页面构建器的插入物累计——投机 builder 的页状态从 S0 克隆，
        // 不含窗口内前段对页面的推进（RFC-5 §3.5 黑名单项）。
        self.mark_spec_taint();
        self.page_state.page.insert_penalties()
    }
    fn take_page_shipped(&mut self) -> bool {
        std::mem::take(&mut self.page_state.page_shipped)
    }
    fn default_output_routine(&mut self) {
        // tex.web @<Perform the default output routine@>：待处理页面不经用户
        // 例程直接 shipout（dead cycles 分支——`\output` 例程从不 ship 时）。
        self.page_state.page.clear_fired_penalties();
        while let Some(p) = self.page_state.pending_pages.pop_front() {
            self.ship_page(p);
        }
    }
    /// `\count<n>` 赋值镜像（输出例程刀 5 页号链）：shipout 页标签与 DVI bop
    /// 计数的取值源（tex.web ship_out L12694 直接读 count(j)）。
    fn count_changed(&mut self, idx: usize, value: i64) -> Result<()> {
        self.page_state.insert_regs.set_count(idx, value);
        self.count_changed(idx, value);
        Ok(())
    }
    fn dimen_changed(&mut self, idx: usize, value: i64) -> Result<()> {
        self.page_state.insert_regs.set_dimen(idx, value);
        Ok(())
    }
    fn skip_changed(&mut self, idx: usize, value: ntex_core::Glue) -> Result<()> {
        self.page_state.insert_regs.set_skip(idx, value);
        Ok(())
    }
    // ---- ETRIP 冲刺：e-TeX marks 族查询 ----
    // （注意：轮转在 feed_one 产出页时立即执行，不在查询时修改状态。）
    //
    // M6：marks 随断页轮转演化——投机 builder 的 marks 是 S0 时刻的值，不含
    // 窗口内前段造成的轮转 → 读取即 taint（段作废）。
    fn topmarks(&self, class: i64) -> String {
        self.mark_spec_taint();
        self.page_state
            .marks_top
            .get(&class)
            .cloned()
            .unwrap_or_default()
    }
    fn firstmarks(&self, class: i64) -> String {
        self.mark_spec_taint();
        self.page_state
            .marks_first
            .get(&class)
            .cloned()
            .unwrap_or_default()
    }
    fn botmarks(&self, class: i64) -> String {
        self.mark_spec_taint();
        self.page_state
            .marks_bot
            .get(&class)
            .cloned()
            .unwrap_or_default()
    }
    fn splitfirstmarks(&self, class: i64) -> String {
        self.mark_spec_taint();
        self.page_state
            .marks_split_first
            .get(&class)
            .cloned()
            .unwrap_or_default()
    }
    fn splittopmarks(&self, class: i64) -> String {
        self.mark_spec_taint();
        self.page_state
            .marks_split_top
            .get(&class)
            .cloned()
            .unwrap_or_default()
    }
    fn splitbotmarks(&self, class: i64) -> String {
        self.mark_spec_taint();
        self.page_state
            .marks_split_bot
            .get(&class)
            .cloned()
            .unwrap_or_default()
    }
    /// `\insert<num>{`：下一个组为 insert 组（tex.web group_code=insert_group=11）。
    ///
    /// tex.web `begin_insert_or_adjust`：`scan_eight_bit_int` → `saved(0):=cur_val`
    /// → `new_save_level(insert_group); scan_left_brace; normal_paragraph; push_nest;
    /// mode:=-vmode`——组体由主循环在内部垂直模式照常排版，`}` 处 group_end 的
    /// Insert 臂 vpack(natural) 后挂 ins_node（体排版在此真正发生，脚注文本因此
    /// 可回流页底）。`\insert255` 按 tex.web 报错并改道 0（box 255 是页面寄存器）
    /// ——否则 fire_up 累积会写穿 [`PAGE_BOX`] 页队列。
    fn insert_begin(&mut self, class: usize) -> Result<()> {
        let mut class = class;
        if class == PAGE_BOX {
            self.report_error("You can't \\insert255.");
            self.report_help("I'm changing to \\insert0; box 255 is special.");
            class = 0;
        }
        self.box_state.pending_kind = Some(GroupKind::Insert);
        self.box_state.pending_insert_class = class;
        Ok(())
    }
    /// `\vadjust{...}`：下一个组为 adjust 组，体在内部垂直模式排版。
    fn vadjust(&mut self, toks: Vec<Token>) -> Result<()> {
        debug_assert!(toks.is_empty());
        self.box_state.pending_kind = Some(GroupKind::VAdjust);
        Ok(())
    }
    /// 输出例程的隐式组：组种类 8（output group，tex.web group_code）。
    fn output_routine_begin(&mut self) -> Result<()> {
        self.box_state.pending_kind = Some(GroupKind::Output);
        Ok(())
    }
    /// `\mark`/`\marks<n>`：mark 节点追加到当前列表（无维度）。
    /// 同步更新当前页 marks_first/marks_bot（class=None 映射到 0，即 \mark=\marks0）。
    fn mark(&mut self, class: Option<i64>, text: String) -> Result<()> {
        // TeX 语义：\mark 等价于 \marks0（class 0）。
        let c = class.unwrap_or(0);
        // marks_first：该 class 在当前页第一次出现时设置。
        self.page_state
            .marks_first
            .entry(c)
            .or_insert_with(|| text.clone());
        // marks_bot：每次出现都更新（最后一次出现）。
        self.page_state.marks_bot.insert(c, text.clone());
        self.append(Node::Mark { class, text });
        Ok(())
    }
    fn take_write_flush_pending(&mut self) -> bool {
        let v = self.page_state.write_flush_pending;
        self.page_state.write_flush_pending = false;
        v
    }
}
