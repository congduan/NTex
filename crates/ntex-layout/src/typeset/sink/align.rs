use super::*;

impl AlignSink for NodeBuilder {
    /// `\valign{`/`\halign{`：下一个组为对齐组（6）。M4-5：建排版上下文
    /// （两遍法；`to`/`spread` 规格取 box_spec 槽）。
    fn align_begin(&mut self, is_halign: bool) -> Result<()> {
        let dir = if is_halign {
            AlignDir::Halign
        } else {
            AlignDir::Valign
        };
        let (to, spread) = self
            .box_state
            .pending_box_spec
            .take()
            .unwrap_or((None, None));
        self.align_stack.push((
            dir,
            AlignCtx {
                tabskips: Vec::new(),
                stream: Vec::new(),
                cur_cells: Vec::new(),
                cur_col: 0,
                cell_box_saves: None,
                cell_group_depth: 0,
                to,
                spread,
            },
        ));
        self.box_state.pending_kind = Some(GroupKind::Align);
        Ok(())
    }
    /// M4-5 `\cr`（对齐行/列结束，tex.web fin_row）：当前行单元入流
    /// （原始列表，fin_align 统一封装；空行跳过——tex.web 空行盒高 0）。
    fn align_row_end(&mut self) -> Result<()> {
        let Some((_, ctx)) = &mut self.align_stack.last_mut() else {
            return Ok(());
        };
        let cells = std::mem::take(&mut ctx.cur_cells);
        ctx.cur_col = 0;
        if cells.is_empty() {
            return Ok(());
        }
        ctx.stream.push(AlignItem::Row(cells));
        Ok(())
    }
    /// M4-5 对齐 preamble 结束或周期列扩展：替换列边界 tabskip 快照（len = 列数 + 1）。
    fn align_preamble_end(&mut self, tabskips: Vec<ntex_core::Glue>) -> Result<()> {
        if let Some((_, ctx)) = self.align_stack.last_mut() {
            ctx.tabskips = tabskips;
        }
        Ok(())
    }
    /// M4-5 对齐单元开始（tex.web init_span 的 push_nest）：压入单元内容
    /// 列表——\halign 受限水平（v 模板通常以 \hfil 收尾）；\valign 垂直。
    fn align_cell_begin(&mut self) -> Result<()> {
        if let Some((_, ctx)) = self.align_stack.last_mut() {
            ctx.cell_box_saves = Some(Vec::new());
            ctx.cell_group_depth = self.groups.len();
        }
        self.lists.push(Vec::new());
        let mode = match self.align_stack.last() {
            Some((AlignDir::Valign, _)) => Mode::Vertical,
            _ => Mode::RestrictedHorizontal,
        };
        self.list_modes.push(mode);
        Ok(())
    }
    /// M4-5 对齐单元结束（tex.web fin_col 的单元封装时机）：单元列表出栈，
    /// 原始节点攒入当前行（封装延迟到 fin_align 统一列宽）。`&`（Tab）推进
    /// 列指针（跨列单元按 span_len 累计）；`\cr`（Cr）的行收集由
    /// [`Self::align_row_end`] 完成。
    fn align_cell_end(&mut self, end: ntex_core::sink::AlignCellEnd, span_len: u16) -> Result<()> {
        // \valign 单元（垂直列表）内开段时先收段
        if self.mode() == Mode::Horizontal {
            self.close_paragraph();
        }
        let nodes = self.lists.pop().unwrap_or_default();
        self.list_modes.pop();
        if let Some((_, ctx)) = self.align_stack.last_mut() {
            if let Some(saves) = ctx.cell_box_saves.take() {
                for (idx, old) in saves.into_iter().rev() {
                    self.write_box(idx, old);
                }
            }
        }
        let Some((_, ctx)) = &mut self.align_stack.last_mut() else {
            return Ok(());
        };
        let start = ctx.cur_col;
        ctx.cur_cells.push(AlignCellBox {
            start_col: start,
            span_len,
            nodes,
        });
        if matches!(end, ntex_core::sink::AlignCellEnd::Tab) {
            ctx.cur_col += span_len as usize;
        }
        Ok(())
    }
    /// `\noalign{`：下一个组为无对齐组（7）。
    fn noalign_begin(&mut self) -> Result<()> {
        self.box_state.pending_kind = Some(GroupKind::NoAlign);
        Ok(())
    }
}
