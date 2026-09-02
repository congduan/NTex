
// NodeBuilder 主 impl（含分页/段落方法）在此闭合；数学/TokenSink 分片各自开启新 impl。
/// TFM 字体加载器（M3-4）：`\font` 执行时按名字查找/解析 TFM，追加到共享字体表。
#[derive(Debug)]
struct TfmLoader {
    table: Rc<RefCell<Vec<FontMetrics>>>,
}

impl FontLoader for TfmLoader {
    fn load(&mut self, name: &str, at: Option<i64>, scaled: Option<i64>) -> Result<u32> {
        if at.is_some() && scaled.is_some() {
            return Err(Error::invalid_input("\\font 的 at 与 scaled 不能同时给出"));
        }
        let path = ntex_font::find_tfm(name)
            .ok_or_else(|| Error::invalid_input(format!("找不到 TFM 文件：{name}")))?;
        let bytes = std::fs::read(&path).map_err(|e| Error::io("读取 TFM", path, e))?;
        let mut fm = ntex_font::parse_tfm(&bytes)
            .map_err(|e| Error::invalid_input(format!("解析 {name}: {e}")))?;
        fm.name = name.to_owned(); // DVI fnt_def 的字体名
        // at：目标尺寸/设计字号；scaled：千分比
        let fm = match (at, scaled) {
            (Some(at_sp), None) => {
                let den = fm.design_size_sp;
                if den <= 0 {
                    return Err(Error::invalid_input(format!("{name} 设计字号非法")));
                }
                fm.scaled_by(at_sp, den)
            }
            (None, Some(s)) => fm.scaled_by(s, 1000),
            _ => fm,
        };
        let mut table = self.table.borrow_mut();
        // 表空时先放 nullfont 占位（**id 0 = nullfont**，tex.web 内建字体；
        // 此前第一个 \font 加载返回 id 0 与 nullfont 冲突，且 id=len+1 会与
        // 表索引错位——loaded(id) 查 table.get(id) 误判未加载）。用户字体 id
        // 从 1 起、与表索引一一对应。
        if table.is_empty() {
            table.push(FontMetrics {
                design_size_sp: 0,
                scale: 0,
                checksum: 0,
                name: "nullfont".to_owned(),
                chars: Vec::new(),
                slant: 0,
                space: 0,
                space_stretch: 0,
                space_shrink: 0,
                x_height: 0,
                quad: 0,
                extra_space: 0,
                lig_kern_steps: Vec::new(),
                kern_values: Vec::new(),
                lig_kern_index: Vec::new(),
                font_params: Vec::new(),
            });
        }
        // 同名字+同缩放复用 id（tex.web：\font\cs=name 同参重复定义不新加载）。
        let scale = fm.scale;
        if let Some(existing) = table
            .iter()
            .position(|fm| fm.name == name && fm.scale == scale)
        {
            return Ok(existing as u32);
        }
        let id = u32::try_from(table.len())
            .map_err(|_| Error::internal("字体表溢出（> 2^32 字体）"))?;
        table.push(fm);
        Ok(id)
    }

    /// 字体字符度量查询（`\iffontchar`/`\fontchar*`）：字体表与排版器共享。
    fn char_metric(&mut self, font: u32, ch: u32) -> Option<(i64, i64, i64)> {
        let table = self.table.borrow();
        let fm = table.get(font as usize)?;
        fm.chars.get(ch as usize).copied().flatten()
    }
}

/// 排版器：VM token 流 → 节点树（主垂直列表）。
pub struct Typesetter {
    expander: Expander,
    fonts: Fonts,
    /// 上一次 `finish` 收走的终端转录（`\message`/`\show`/`\write16` 累积；
    /// finish 的 take_sink 会把 NodeBuilder 摘走，先在此留档）。
    last_transcript: String,
    /// `finish` 收走的 `\shipout` 页面（DVI 统计行：页数）。
    shipped: Vec<BoxNode>,
    /// .fmt 导入的当前字体（import_state 时 sink 未装，install_builder 同步）。
    fmt_current_font: u32,
    /// finish 收走的当前字体（take_sink 后 NodeBuilder 不可达，export_state 用）。
    last_current_font: u32,
}

impl Typesetter {
    /// 创建排版器（字符维度/词间距默认全零，M3-4 TFM 前占位）。
    pub fn new() -> Self {
        Self::with_metrics(|_, _| (0, 0, 0))
    }

    /// 创建排版器并指定字符度量函数（词间距默认全零）。
    pub fn with_metrics(metrics: MetricsFn) -> Self {
        Self {
            expander: Expander::new(),
            fonts: Fonts::Fn {
                metrics,
                space: |_| Glue::ZERO,
            },
            last_transcript: String::new(),
            shipped: Vec::new(),
            fmt_current_font: 0,
            last_current_font: 0,
        }
    }

    /// 指定词间空白胶水函数（空格 token → 胶水；仅 fn 指针模式生效）。
    pub fn with_space(mut self, space: SpaceFn) -> Self {
        if let Fonts::Fn { space: s, .. } = &mut self.fonts {
            *s = space;
        }
        self
    }

    /// 注入 VFS 后端（RFC-3；`\input`/`\write` 等副作用原语的文件接口）。
    pub fn set_vfs(&mut self, vfs: Box<dyn ntex_io::Vfs>) {
        self.expander.set_vfs(vfs);
    }

    /// 取回 VFS（测试断言写入内容用）。
    pub fn take_vfs(&mut self) -> Box<dyn ntex_io::Vfs> {
        self.expander.take_vfs()
    }

    /// 导出展开引擎状态快照（`.fmt` v1；供 `ntex-format` 序列化）。
    pub fn export_state(&self) -> ntex_core::expand::FmtState {
        let mut st = self.expander.export_state();
        // pass1 结束时的当前字体（finish 存档；fmt 恢复后防全 nullfont）
        st.current_font = self.last_current_font;
        st
    }

    /// 加载展开引擎状态快照（`.fmt` v1）。
    pub fn import_state(&mut self, state: ntex_core::expand::FmtState) {
        let font_loads = state.font_loads.clone();
        let font_cs_names = state.font_cs_names.clone();
        self.fmt_current_font = state.current_font;
        self.expander.import_state(state);
        // .fmt 不含字体表：按 font_loads 重新加载 TFM（pass1 定义的
        // `\font\trip` 在 pass2 不重跑，字体表需恢复——否则字符悬空字体）
        if let Fonts::Tfm(table) = &self.fonts {
            let mut loader = TfmLoader {
                table: table.clone(),
            };
            for (name, at, scaled) in font_loads.iter().flatten() {
                let _ = loader.load(name, *at, *scaled);
            }
        }
        // 排版器参数镜像对齐（fmt 恢复的 \vsize/\tracingpages 等不会经 param_changed 推送）
        let p = *self.expander.params_ref();
        if let Some(b) = self
            .expander
            .sink_mut()
            .as_any_mut()
            .downcast_mut::<NodeBuilder>()
        {
            b.sync_params(&p);
            // fmt 里的 FontId → cs 名表（showbox 字体标识显示 `.\trip 1`）
            b.font_cs_names = font_cs_names;
        }
    }

    /// ETRIP 冲刺：`\dump` 是否已执行（驱动据此保存 fmt 并二次运行测试体）。
    pub fn dumped(&self) -> bool {
        self.expander.dumped()
    }

    /// ETRIP 冲刺：最近一次 `\typeout{Checking ...}` 的段标题（错误定位）。
    pub fn current_section(&self) -> &str {
        self.expander.current_section()
    }

    /// TFM 字体模式（M3-4）：`\font\cs=cmr10` 加载真实度量，
    /// 字符维度/词间空白来自 TFM；`\font` 定义的 cs 作为字体选择器。
    pub fn with_tfm() -> Self {
        Self {
            expander: Expander::new(),
            fonts: Fonts::Tfm(Rc::new(RefCell::new(Vec::new()))),
            last_transcript: String::new(),
            shipped: Vec::new(),
            fmt_current_font: 0,
            last_current_font: 0,
        }
    }

    /// TFM 字体模式 + 自动分页（DVI 产物路径）：与 [`Self::with_tfm`] 相同，
    /// 但打开 M3-5-2 断页（`\vsize` 断页 + 输入结束冲页），使 `\shipout`
    /// 页面与 DVI 字节数完整（统计行 "Output written" 需要真实页数）。
    pub fn with_tfm_paginated() -> Self {
        Self {
            expander: Expander::new(),
            fonts: Fonts::Tfm(Rc::new(RefCell::new(Vec::new()))),
            last_transcript: String::new(),
            shipped: Vec::new(),
            fmt_current_font: 0,
            last_current_font: 0,
        }
    }

    /// 排版源码，返回主垂直列表节点。
    pub fn typeset(&mut self, text: &str) -> Result<Vec<Node>> {
        self.install_font_loader();
        self.install_builder(NodeBuilder::new(self.fonts.clone()));
        self.expander.run_source(text)?;
        self.finish().map(|out| {
            self.shipped = out.shipped;
            out.main
        })
    }

    /// 排版字节源码。
    pub fn typeset_bytes(&mut self, bytes: impl Into<Vec<u8>>) -> Result<Vec<Node>> {
        self.install_font_loader();
        self.install_builder(NodeBuilder::new(self.fonts.clone()));
        self.expander.feed_source(bytes);
        self.expander.run()?;
        self.finish().map(|out| {
            self.shipped = out.shipped;
            out.main
        })
    }

    /// 安装 NodeBuilder 并同步参数镜像（`.fmt` 加载的 \\\\vsize/\\\\tracingpages 等
    /// 不会经 param_changed 推送——新建 builder 的 params 是默认值）。
    fn install_builder(&mut self, builder: NodeBuilder) {
        let p = *self.expander.params_ref();
        if std::env::var("NTEX_DEBUG_TRACINGPAGES").is_ok() {
            eprintln!(
                "[tracingpages] install_builder: misc59={} misc29={} misc55={}",
                p.misc[59], p.misc[29], p.misc[55]
            );
        }
        let mut builder = builder;
        // .fmt 导入的 FontId → cs 名（pass1 定义的 \font\trip 在 pass2 不重跑；
        // import_state 时 sink 尚未安装，须在 install 时补同步）
        builder.font_cs_names = self.expander.font_cs_names_ref().clone();
        // .fmt 导入的当前字体（防 pass2 字符全 nullfont + Missing 警告）
        builder.current_font = FontId(self.fmt_current_font);
        // pass2 NodeBuilder 重建：同步数学间距参数（\\thinmuskip 等 muskip 寄存器——
//  pass1 赋值在 dump 前，pass2 不重跑赋值事件）。muskip_is_mu 同步为全 true：
// muskip 寄存器 0/1/2（= thinmuskip/medmuskip/thickmuskip）的 width/stretch/shrink
// 字段永为 mu 数值（expander 端 scan_glue_mu 路径按 1mu=65536 单位存）。
        builder.muskip_params = self.expander.muskip_registers();
        builder.muskip_is_mu = [true; 3];
        self.expander.set_sink(Box::new(builder));
        if let Some(b) = self
            .expander
            .sink_mut()
            .as_any_mut()
            .downcast_mut::<NodeBuilder>()
        {
            b.sync_params(&p);
        }
    }

    /// 取走终端转录（`\message`/`\show`/`\write16` 累积文本）。
    pub fn take_transcript(&mut self) -> String {
        // finish 已走：转录在 last_transcript；未走（运行中途报错）：从 sink 取
        if !self.last_transcript.is_empty() {
            return std::mem::take(&mut self.last_transcript);
        }
        self.expander
            .sink_mut()
            .as_any_mut()
            .downcast_mut::<NodeBuilder>()
            .map(|b| std::mem::take(&mut b.transcript))
            .unwrap_or_default()
    }

    /// 排版源码并取回 `\shipout` 页面（DVI 输出，M3-5）：
    /// 返回 (页面列表, 字体表快照)。需 [`Self::with_tfm`] 模式（否则字体表为空）。
    /// 启用 M3-5-2 断页：顶层垂直列表经页面构建器自动分页（`\vsize`），
    pub fn typeset_dvi(&mut self, text: &str) -> Result<(Vec<BoxNode>, Vec<FontMetrics>)> {
        self.install_font_loader();
        self.install_builder(NodeBuilder::with_pagination(self.fonts.clone(), true));
        self.expander.run_source(text)?;
        let out = self.finish()?;
        self.shipped = out.shipped.clone();
        Ok((out.shipped, out.fonts))
    }

    /// 排版结束后取回已 shipout 的页面列表（[`Self::finish`] 已执行时有效）。
    pub fn shipped_pages(&self) -> &[BoxNode] {
        &self.shipped
    }

    /// 排版结束后取回字体表快照（[`Self::finish`] 已执行时有效）。
    pub fn fonts_snapshot(&self) -> Vec<FontMetrics> {
        match &self.fonts {
            Fonts::Tfm(table) => table.borrow().clone(),
            Fonts::Fn { .. } => Vec::new(),
        }
    }

    /// TFM 模式：把共享字体表接给 VM 的 `\font` 加载器。
    fn install_font_loader(&mut self) {
        if let Fonts::Tfm(table) = &self.fonts {
            self.expander
                .set_font_loader(Box::new(TfmLoader { table: table.clone() }));
        }
    }

    /// 运行结束收尾：关闭开放段落、校验盒子/组闭合，冲掉残余页面，取回主列表与页面。
    ///
    /// 输出例程（M3-5-3）需要引擎在 token 边界执行，因此校验/冲页都在
    /// sink 仍挂接引擎时进行：close_paragraph / eject_one_page 产出的页面
    /// 经 box255+例程（或直通 shipout），随后 `run_pending_output` 执行例程。
    fn finish(&mut self) -> Result<FinishOutput> {
        // 1) 校验 + 关闭开放段落（可能产出页面 → box255 + pending）。
        //    显式 `\end` 时未闭合组/数学列表按 TeX 语义降级为警告继续
        //    （tex.web final_end：`(\end occurred inside a group at level N)`，
        //    trip.log L7293），仅纯 EOF 缺 `\end` 才报错。
        let ended = self.expander.is_ended();
        {
            let builder = self
                .expander
                .sink_mut()
                .as_any_mut()
                .downcast_mut::<NodeBuilder>()
                .ok_or_else(|| Error::internal("typesetter 安装了 NodeBuilder"))?;
            if builder.pending_box.is_some() {
                if ended {
                    builder.pending_box = None;
                } else {
                    return Err(Error::invalid_input("\\hbox/\\vbox 后缺少组"));
                }
            }
            if builder.shipout_next {
                if ended {
                    builder.shipout_next = false;
                } else {
                    return Err(Error::invalid_input("\\shipout 后缺少盒子"));
                }
            }
            if !builder.groups.is_empty() {
                if ended {
                    // TeX：\end 时组未闭合 → 警告不中断（trip.log L7293）
                    let _ = builder.write16(format!(
                        "(end occurred inside a group at level {})\n",
                        builder.groups.len()
                    ));
                    builder.groups.clear();
                } else {
                    let dbg = builder
                        .groups
                        .iter()
                        .map(|g| format!("{:?}", g.kind))
                        .collect::<Vec<_>>()
                        .join(", ");
                    let m = format!(
                        "组未闭合（缺少 }}）：groups=[{dbg}] pending_box={:?} pending_kind={:?} math={}",
                        builder.pending_box,
                        builder.pending_kind,
                        builder.math.len()
                    );
                    return Err(Error::invalid_input(&m));
                }
            }
            if !builder.math.is_empty() {
                if ended {
                    // TeX：\end 时数学列表未闭合 → 同样警告不中断
                    let _ = builder.write16("(end occurred inside a math list)\n".to_string());
                    builder.math.clear();
                } else {
                    return Err(Error::invalid_input("数学模式未闭合（缺少 $）"));
                }
            }
            // 先收尾进行中的段落（tex.web final_end：\end 前 end_graf 收段折行），
            // 再丢弃未闭合盒子/组的残留列表——顺序不能反：先 pop 会把进行中的
            // 水平段列表一起扔掉（VFS 分章测试 `\input{ch1}...\end` 无页面根因）。
            if builder.mode() == Mode::Horizontal {
                builder.close_paragraph();
            }
            if ended {
                // TeX `\end`：丢弃未闭合盒子/组的内容（tex.web final_end 后
                // 各列表就地废弃），仅保留主垂直列表。
                while builder.lists.len() > 1 {
                    builder.lists.pop();
                    builder.list_modes.pop();
                }
            }
        }
        // 2) 执行 close_paragraph 产出的待执行输出例程
        self.expander.run_pending_output()?;
        // 3) 输入结束按 `\end` 冲掉残余页面（tex.web `its_all_over`），
        //    与输出例程交错：冲一页 → 执行例程 → 再冲。
        loop {
            let ejected = {
                let builder = self
                    .expander
                    .sink_mut()
                    .as_any_mut()
                    .downcast_mut::<NodeBuilder>()
                    .ok_or_else(|| Error::internal("typesetter 安装了 NodeBuilder"))?;
                if builder.pagination {
                    builder.eject_one_page()?
                } else {
                    false
                }
            };
            if !ejected {
                break;
            }
            self.expander.run_pending_output()?;
        }
        // 4) 取走 sink，收集主列表/页面/字体表
        let mut sink = self.expander.take_sink();
        let builder = sink
            .as_any_mut()
            .downcast_mut::<NodeBuilder>()
            .ok_or_else(|| Error::internal("typesetter 安装了 NodeBuilder"))?;
        // 转录留档（finish 后 sink 被 VecSink 替换，take_transcript 读不到 NodeBuilder）
        self.last_transcript = std::mem::take(&mut builder.transcript);
        // 当前字体存档（take_sink 后 export_state 读不到 NodeBuilder）
        self.last_current_font = builder.current_font.0;
        let mut lists = std::mem::take(&mut builder.lists);
        debug_assert_eq!(lists.len(), 1, "收尾后应只剩主列表");
        let shipped = std::mem::take(&mut builder.shipped);
        let fonts = match &self.fonts {
            Fonts::Tfm(table) => table.borrow().clone(),
            Fonts::Fn { .. } => Vec::new(),
        };
        // RFC-3：排版结束收尾 flush 残留延迟写流（TeX \end final_cleanup 语义）
        self.expander.flush_writes()?;
        self.shipped = shipped.clone();
        Ok(FinishOutput {
            main: lists.pop().expect("主列表"),
            shipped,
            fonts,
        })
    }
}

/// `finish` 的返回：主垂直列表 + `\shipout` 页面 + 字体表快照。
struct FinishOutput {
    main: Vec<Node>,
    shipped: Vec<BoxNode>,
    fonts: Vec<FontMetrics>,
}

impl Default for Typesetter {
    fn default() -> Self {
        Self::new()
    }
}
