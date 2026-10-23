use super::*;

impl BoxSink for NodeBuilder {
    /// `\setbox<n>=<box>`（ETRIP）：记录目标寄存器；后续封装的盒子存入该槽。
    fn setbox(&mut self, idx: usize, global: bool) -> Result<()> {
        self.box_state.setbox_target = Some(idx);
        self.box_state.setbox_global = global;
        Ok(())
    }
    /// `\hbox to/spread <dimen>`（ETRIP）：记录盒子规格，随下一个盒子组生效。
    fn box_spec(&mut self, to: Option<i64>, spread: Option<i64>) -> Result<()> {
        self.box_state.pending_box_spec = Some((to, spread));
        Ok(())
    }
    /// 无限阶胶水（ETRIP）：`\hfil`=0/`\hfill`=1/`\hss`=2/`\vfil`=3/`\vfill`=4/`\vss`=5。
    /// 方向不符当前模式的原语忽略（TeX 语义：如水平模式中的 `\vfil` 无效）。
    /// stretch/shrink 按 TeX 存 **1pt = 65536sp**（`0pt plus 1fil` 的 stretch=65536，
    /// fil 阶下 gs 按比例缩放、布局不变，showbox 显示 `plus 1.0fil` 对齐参考）。
    fn fill_glue(&mut self, kind: u8) -> Result<()> {
        let one = ntex_core::register::SP_PER_PT;
        let (horizontal, stretch, shrink, order) = match kind {
            0 => (true, one, 0, GLUE_ORDER_FIL),    // \hfil  0pt plus 1fil
            1 => (true, one, 0, GLUE_ORDER_FILL),   // \hfill 0pt plus 1fill
            2 => (true, one, one, GLUE_ORDER_FIL),  // \hss   0pt plus 1fil minus 1fil
            3 => (false, one, 0, GLUE_ORDER_FIL),   // \vfil
            4 => (false, one, 0, GLUE_ORDER_FILL),  // \vfill
            5 => (false, one, one, GLUE_ORDER_FIL), // \vss
            6 => (false, -one, 0, GLUE_ORDER_FIL),  // \vfilneg（负 1fil）
            7 => (true, -one, 0, GLUE_ORDER_FIL),   // \hfilneg（负 1fil）
            _ => return Err(Error::internal("非法 fill 胶水种类")),
        };
        let in_horizontal = matches!(self.mode(), Mode::Horizontal | Mode::RestrictedHorizontal);
        // 数学模式没有"水平列表"，却接受 h 方向 fill：tex.web `\hfil` 等是
        // `hskip` 的特例（`append_glue` 无条件 `tail_append(new_glue)`，mlist
        // 里就是 glue 节点）；amsmath `multline` 靠模板 `\hfil` 与行首
        // `\hfilneg` 相消实现首行左齐/末行右齐。v 方向 fill 在数学模式仍无意义。
        let in_math = matches!(self.mode(), Mode::Math | Mode::DisplayMath);
        if horizontal != in_horizontal && !(in_math && horizontal) {
            return Ok(()); // 方向不符：忽略
        }
        // `\leaders\hbox{…}\hfill`：fill 胶水也是"proper glue"（tex.web 把
        // \hfil/\hfill/\hss 归入 glue 赋值类）。此前漏臂 → 引导盒子挂起不消，
        // 下一 token 触发 "Leaders not followed by proper glue" 并丢盒，
        // 目录点线（\@dottedtocline 的 \leaders…\hfill）整行消失。
        if let Some((ld, box_node)) = self.box_state.leaders_box.take() {
            self.append(Node::Leaders {
                kind: ld,
                inner: Box::new(box_node),
                width: 0,
                stretch,
                shrink,
                stretch_order: order,
                shrink_order: 0,
            });
            return Ok(());
        }
        // \vfill 等不是合法数学字段开头（tex.web scan_math othercases；TRIP L396
        // `\accent\x\vfill` 在 \vfill 处报 Missing { inserted）
        self.check_math_field_break()?;
        // 数学模式：fill 胶水作为 mlist 的 pt 胶节点（`\hskip` 同款）——
        // mlist_to_hlist 只换算 mu 胶，pt 胶原样落 hlist（tex.web math_glue）。
        if in_math {
            if !horizontal {
                return Ok(()); // \vfil 等垂直 fill 在数学模式无效果
            }
            self.math_push_atom(MathAtom::MSkip {
                width: 0,
                stretch,
                shrink,
                stretch_order: order,
                shrink_order: order,
                nonscript: false,
                mu: false,
            })?;
            return Ok(());
        }
        self.append(Node::Glue {
            name: None,
            width: 0,
            stretch,
            shrink,
            stretch_order: order,
            shrink_order: order,
        });
        Ok(())
    }
    /// `\vsplit<n> to/spread <dimen>`（ETRIP）：拆分盒子寄存器 n 顶部。
    /// 寄存器 n 保留余量；结果按 `\setbox` 目标路由，否则追加。
    /// 寄存器 255 = 待输出页队首（[PAGE_BOX]）：latex.ltx `\@doclearpage` 的
    /// `\vsplit\@cclv to\z@` 从页顶取走 discardables。
    /// tex.web vsplit @<Dispense with trivial cases...@>：void 盒 → 结果 void、
    /// 静默（"The extracted box is void if and only if the original box was
    /// void"）；真 TeX 实测 `\vsplit255 to 10pt` void → `\ifvoid0`=Y、`\ht0`=0、
    /// 255 保持 void、无错误信息。
    fn vsplit(&mut self, idx: usize, to: Option<i64>, spread: Option<i64>) -> Result<()> {
        let Some(b) = self.take_box_at(idx) else {
            // void 盒：cur_box=null（tex.web）→ `\setbox` 目标存空、裸调用不产节点
            if let Some(t) = self.box_state.setbox_target.take() {
                self.store_box(t, None);
            }
            return Ok(());
        };
        let natural = b.height + b.depth;
        let target = match (to, spread) {
            (Some(t), _) => t,
            (_, Some(s)) => natural + s,
            _ => natural,
        };
        let (result, remainder) = split_vbox(b, target);
        // 余量写回寄存器：tex.web `box(n):=vpack(q,natural)`；若 q 为空则原盒
        // 变 void。LaTeX mark 代码依赖 `\vsplit <box> to \maxdimen` 后
        // `\ifvoid<box>` 为真来终止递归。
        let remainder = if remainder.children.is_empty() {
            None
        } else {
            Some(remainder)
        };
        self.write_box(idx, remainder);
        if let Some(t) = self.box_state.setbox_target.take() {
            self.store_box(t, Some(result));
        } else {
            self.append(Node::Box(result));
        }
        Ok(())
    }
    /// `\box<n>`（M3-5-3）：取出盒子寄存器；`\shipout` 前缀时封装为页面，
    /// 否则作为节点追加到当前列表。void 盒子报错（TeX "Box n is void"）。
    /// 寄存器 255 = 待输出例程处理页面的队首（[PAGE_BOX]，tex.web `box(255)`）。
    fn box_register(&mut self, idx: usize) -> Result<()> {
        if idx == 0 && std::env::var_os("AVD_BOX_DEBUG").is_some() {
            eprintln!(
                "AVD_READ0 mode={:?} level={} present={}",
                self.mode(),
                self.groups.len(),
                self.box_view(idx).is_some()
            );
        }
        // `\setbox5=\box3`：把寄存器 3 移入目标 5（\box3 变 void；tex.web set_box 赋值语义）
        if let Some(target) = self.box_state.setbox_target.take() {
            // `\setbox0=\lastbox`：优先取 \lastbox 摘下的盒子
            let b = self
                .box_state
                .lastbox_hold
                .take()
                .or_else(|| self.take_box_at(idx));
            self.store_box(target, b);
            return Ok(());
        }
        // `\box0` 紧跟在 `\lastbox` 后：取摘下的盒子（TeX 语义）
        let b = self
            .box_state
            .lastbox_hold
            .take()
            .or_else(|| self.take_box_at(idx));
        let Some(b) = b else {
            // TeX：\box 取 void 盒子 → 空 hbox 节点（tex.web：仍产生节点；TRIP L104 前 \copy200 void）
            return self.append_box_or_math_atom(crate::node::BoxNode::new_hbox(Vec::new()));
        };
        if self.page_state.shipout_next {
            self.page_state.shipout_next = false;
            self.ship_page(b);
        } else {
            self.append_box_or_math_atom(b)?;
        }
        Ok(())
    }
    /// 盒子寄存器种类（0=void、1=hbox、2=vbox）；`\ifvoid`/`\ifhbox`/`\ifvbox` 用。
    /// 寄存器 255 = 待输出页队首（[PAGE_BOX]）：页面为 vbox → `\ifvbox255`=真、
    /// 页队列空 → `\ifvoid255`=真（真 TeX 实测：页在 `VB:Y|HB:N|VOID:N`，
    /// 页尽 `VOID:Y`）。
    fn box_register_kind(&self, idx: usize) -> i64 {
        match self.box_view(idx) {
            None => 0,
            Some(b) => match b.kind {
                crate::node::BoxKind::HBox => 1,
                crate::node::BoxKind::VBox => 2,
            },
        }
    }
    /// `\wd/\ht/\dp<n>`：盒子寄存器维度（void 为 0）。
    /// 寄存器 255 = 待输出页队首（[PAGE_BOX]）：latex.ltx `\@specialoutput` 的
    /// `\@pageht \ht\@holdpg` 型查询与页尺寸记账（`\ht\@cclv`）同语义。
    fn box_dim(&self, idx: usize, dim: u8) -> i64 {
        let Some(b) = self.box_view(idx) else {
            return 0;
        };
        match dim {
            0 => b.width,
            1 => b.height,
            _ => b.depth,
        }
    }
    /// `\wd/\ht/\dp<n>=<dimen>`：设置盒子寄存器维度。
    /// 255 → 改待输出页队首（`\dp\@cclv=...` 型改写落在页面上）；void 忽略。
    fn set_box_dim(&mut self, idx: usize, dim: u8, value: i64) -> Result<()> {
        let slot = if self.page_box_queue_active(idx) {
            self.page_state.pending_pages.front_mut()
        } else {
            self.boxes_mut().get_mut(idx).and_then(|s| s.as_mut())
        };
        let Some(b) = slot else {
            // void 盒子无维度可设：忽略（TeX 恢复语义，TRIP halign 模板场景）
            return Ok(());
        };
        match dim {
            0 => b.width = value,
            1 => b.height = value,
            _ => b.depth = value,
        }
        Ok(())
    }
    /// `\copy<n>`：复制盒子寄存器为节点追加到当前列表（原寄存器保留）。
    /// 寄存器 255 = 待输出页队首（[PAGE_BOX]）——trip.tex 第二例程
    /// `\setbox255\copy255` 即复制当前页。
    fn copy_box(&mut self, idx: usize) -> Result<()> {
        let b = if let Some(h) = self.box_state.lastbox_hold.take() {
            // `\copy0` 紧跟在 `\lastbox` 后：复制摘下的盒子
            h.clone()
        } else {
            let Some(b) = self.box_view(idx) else {
                // TeX：\copy 取 void 盒子 → **空 hbox 节点**（tex.web copy_scan_box：
                // void → null box，仍产生节点触发 freeze/interline；TRIP L104 `\copy200`）
                return self.append_box_or_math_atom(crate::node::BoxNode::new_hbox(Vec::new()));
            };
            b.clone()
        };
        // `\setbox<n>=\copy<m>`：复制结果存入目标寄存器（\copy 不消耗原盒）
        if let Some(target) = self.box_state.setbox_target.take() {
            self.store_box(target, Some(b));
            return Ok(());
        }
        // `\shipout\copy<n>`（tex.web scan_box 复制语义；现代 LaTeX shipout 包装
        // `\tex_shipout:D \box_use:N \l_shipout_box`，`\box_use:N`≡`\copy`——
        // ship 副本、寄存器保留给 shipout/after 钩子）。不认此臂则页永不 ship：
        // shipout_next 泄漏毒化后续轮次 → "Output loop---100 consecutive dead cycles"。
        if self.page_state.shipout_next {
            self.page_state.shipout_next = false;
            self.ship_page(b);
            return Ok(());
        }
        self.append_box_or_math_atom(b)
    }
    /// `\unhbox<n>`/`\unhcopy<n>`：hbox 拆开，子节点追加到当前列表。
    /// tex.web unpackage：`if p=null then return`——void 盒**静默无操作**（真
    /// pdflatex 的 `\unvbox\@begindvibox`（恒 void）不报错为证）；类型/模式
    /// 不符才报 "! Incompatible list can't be unboxed."（TRIP L396 实为
    /// `\unhcopy3` 在 math 模式遇 vlist，非 voidness）。
    fn unhbox(&mut self, idx: usize, copy: bool) -> Result<()> {
        let Some(b) = self.box_view(idx).cloned() else {
            return Ok(());
        };
        // tex.web unpackage: void returns before checking the mode.  LaTeX's
        // \leavevmode uses \unhbox\voidb@x even inside math displays.
        if matches!(self.mode(), Mode::Math | Mode::DisplayMath) || b.kind != BoxKind::HBox {
            self.unbox_error_continue();
            return Ok(());
        }
        if !copy {
            self.take_box_at(idx);
        }
        for c in b.children {
            self.append(c);
        }
        Ok(())
    }
    /// `\unvbox<n>`/`\unvcopy<n>`：vbox 拆开，子节点追加到当前列表。
    fn unvbox(&mut self, idx: usize, copy: bool) -> Result<()> {
        let Some(b) = self.box_view(idx).cloned() else {
            return Ok(());
        };
        if matches!(self.mode(), Mode::Math | Mode::DisplayMath) || b.kind != BoxKind::VBox {
            self.unbox_error_continue();
            return Ok(());
        }
        if !copy {
            self.take_box_at(idx);
        }
        for c in b.children {
            self.append(c);
        }
        Ok(())
    }
    /// `\lastbox`：摘下当前列表尾的盒子节点（无则无操作）；
    /// 摘下的盒子由下一个 `\box`/`\copy` 取用（见 [`Self::box_register`]）。
    fn lastbox(&mut self) -> Result<()> {
        // M6：读-改主列表尾部——投机 builder 的尾部是窗口起点 S0 的尾部，与顺序
        // 执行到本段时的尾部（含窗口内前段产出）不同 → 段作废（`\lastbox` 是
        // RFC-5 §3.5 装配状态读取黑名单首项）。
        self.mark_spec_taint();
        let Some(list) = self.lists.last_mut() else {
            return Ok(());
        };
        if let Some(Node::Box(_)) = list.last() {
            if let Some(Node::Box(b)) = list.pop() {
                self.box_state.lastbox_hold = Some(b);
            }
        }
        // `\setbox0=\lastbox`：摘下的盒子存入目标寄存器（tex.web last_box →
        // cur_box → set_box 赋值语义）；裸 \lastbox 留给后续 \box 消费。
        // 无盒可摘 → cur_box=null → `\setbox` 目标**清空**（tex.web
        // `\setbox\thr@@\lastbox` 置 box3 void）——amsmath `\measure@` 的
        // 列宽循环 `\ifhbox\thr@@ ... \repeat` 依赖此终止；旧实现保持
        // box3 旧值 → 恒真死循环。
        if let Some(t) = self.box_state.setbox_target.take() {
            let b = self.box_state.lastbox_hold.take();
            self.store_box(t, b);
        }
        Ok(())
    }
    /// `\unskip`：移除当前列表尾部的 glue 节点（无则无操作）。
    fn unskip(&mut self) -> Result<()> {
        // M6：与 `\lastbox` 同因——"移除哪个节点"取决于窗口内前段的产出，
        // 投机 builder 的尾部不含它们。段作废（`unskip` 族不属 SideEffects
        // 快照字段，neutral 判定覆盖不到这条隐式依赖，只能在此显式标记）。
        self.mark_spec_taint();
        if let Some(list) = self.lists.last_mut() {
            if matches!(list.last(), Some(Node::Glue { .. } | Node::Leaders { .. })) {
                list.pop();
            }
        }
        Ok(())
    }
    /// `\unpenalty`：移除当前列表尾部的 penalty 节点（无则无操作）。
    fn unpenalty(&mut self) -> Result<()> {
        self.mark_spec_taint();
        if let Some(list) = self.lists.last_mut() {
            if matches!(list.last(), Some(Node::Penalty { .. })) {
                list.pop();
            }
        }
        Ok(())
    }
    /// `\unkern`：移除当前列表尾部的 kern 节点（无则无操作；TRIP L189）。
    fn unkern(&mut self) -> Result<()> {
        self.mark_spec_taint();
        if let Some(list) = self.lists.last_mut() {
            if matches!(list.last(), Some(Node::Kern { .. })) {
                list.pop();
            }
        }
        Ok(())
    }
    /// e-TeX `\lastnodetype`：当前列表尾节点类型码（空列表 -1）。
    ///
    /// `\noalign{...}` 组内：TeX 把 `\noalign` 材料并入对齐所在的垂直列表，
    /// 其尾节点是刚 `\cr` 完成的行/列盒——TeX 的 unset node（e-TeX 码 14，
    /// `\noalign` 只能跟在 `\cr` 后，行盒必已存在）。本引擎行/列盒暂存于
    /// [`Self::align_columns`]、对齐列表在 `\cr` 时已取空，读取时补此映射
    /// （不改节点生成；列表已有节点时仍按列表尾报码）。
    fn last_node_type(&self) -> i64 {
        self.mark_spec_taint();
        if let Some(n) = self.lists.last().and_then(|l| l.last()) {
            return n.node_type_code();
        }
        if matches!(self.groups.last().map(|g| g.kind), Some(GroupKind::NoAlign)) {
            return 14; // unset node（\cr 完成的行/列盒）
        }
        -1
    }
    /// `\lastpenalty`：当前列表尾为 penalty 时取值；外层 vmode 列表被
    /// page builder 抽空后改读它记录的最近贡献。
    fn last_penalty(&self) -> i64 {
        self.mark_spec_taint();
        match self.lists.last().and_then(|l| l.last()) {
            Some(Node::Penalty { penalty }) => *penalty,
            None if self.mode() == Mode::Vertical && self.lists.len() == 1 => {
                self.page_state.page.last_penalty()
            }
            _ => 0,
        }
    }
    /// `\lastskip` 完整胶水量（tex.web L8535）：非空列表只认尾节点；
    /// 外层 vmode 的空贡献列表回读 page builder 最近贡献。
    fn last_glue(&self) -> Option<Glue> {
        self.mark_spec_taint();
        match self.lists.last().and_then(|l| l.last()) {
            Some(Node::Glue {
                width,
                stretch,
                shrink,
                stretch_order,
                shrink_order,
                ..
            }) => Some(Glue {
                width: *width,
                stretch: *stretch,
                shrink: *shrink,
                stretch_order: *stretch_order,
                shrink_order: *shrink_order,
            }),
            None if self.mode() == Mode::Vertical && self.lists.len() == 1 => {
                self.page_state.page.last_glue()
            }
            _ => None,
        }
    }
    /// e-TeX `\lastskip` 作尺寸（tex.web L8535）：胶水取宽度，否则 0。
    /// 此前向后回溯到任意 glue 节点，`\ifdim\lastskip=\z@`
    /// （latex.ltx `\sw@slant`）在前文有胶水时误判非零。
    fn last_skip(&self) -> i64 {
        self.last_glue().map_or(0, |g| g.width)
    }
    /// e-TeX `\lastkern`：当前列表尾 kern 节点的宽度（tex.web L8535 dimen_val
    /// 臂同款——非空列表只看尾节点；外层 vmode 空列表回读 page builder）。
    fn last_kern(&self) -> i64 {
        self.mark_spec_taint();
        match self.lists.last().and_then(|l| l.last()) {
            Some(Node::Kern { width }) => *width,
            Some(Node::Discretionary { replace, .. }) => match replace.last() {
                Some(Node::Kern { width }) => *width,
                _ => 0,
            },
            None if self.mode() == Mode::Vertical && self.lists.len() == 1 => {
                self.page_state.page.last_kern()
            }
            _ => 0,
        }
    }
    /// `\raise`/`\lower<dimen>`：记录盒子参考点位移（下一个封装盒子生效）。
    fn raise(&mut self, amount: i64) -> Result<()> {
        self.box_state.pending_shift = Some(amount);
        Ok(())
    }
    /// `\moveleft<dimen>`：记录盒子水平左移（下一个封装盒子生效；TRIP 冲刺简化）。
    fn move_left(&mut self, amount: i64) -> Result<()> {
        self.box_state.pending_hshift = Some(-amount);
        Ok(())
    }
    /// `\moveright<dimen>`：记录盒子水平右移（下一个封装盒子生效；TRIP 冲刺简化）。
    fn move_right(&mut self, amount: i64) -> Result<()> {
        self.box_state.pending_hshift = Some(amount);
        Ok(())
    }
    /// `\showbox<n>`：把盒子寄存器内容格式化到转录（TeX show_box 风格）。
    fn showbox(&mut self, idx: usize) -> Result<()> {
        let Some(b) = self.box_view(idx) else {
            // TeX：\showbox 空盒 → 显示 void 并恢复（TRIP 中 box 状态差异不致命）
            let out = format!("> \\box{idx}=\nvoid\n! OK.\n");
            self.io_state.transcript.push_str(&out);
            return Ok(());
        };
        let mut out = format!("> \\box{idx}=\n");
        showbox_format_box(b, 0, &self.fonts, &self.font_cs_names, &mut out);
        out.push_str("! OK.\n");
        self.io_state.transcript.push_str(&out);
        Ok(())
    }
}
