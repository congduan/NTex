
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
        self.expander.export_state()
    }

    /// 加载展开引擎状态快照（`.fmt` v1）。
    pub fn import_state(&mut self, state: ntex_core::expand::FmtState) {
        self.expander.import_state(state);
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
        }
    }

    /// 排版源码，返回主垂直列表节点。
    pub fn typeset(&mut self, text: &str) -> Result<Vec<Node>> {
        self.install_font_loader();
        self.expander
            .set_sink(Box::new(NodeBuilder::new(self.fonts.clone())));
        self.expander.run_source(text)?;
        self.finish().map(|o| o.main)
    }

    /// 排版字节源码。
    pub fn typeset_bytes(&mut self, bytes: impl Into<Vec<u8>>) -> Result<Vec<Node>> {
        self.install_font_loader();
        self.expander
            .set_sink(Box::new(NodeBuilder::new(self.fonts.clone())));
        self.expander.feed_source(bytes);
        self.expander.run()?;
        self.finish().map(|o| o.main)
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
    /// 输入结束按 `\end` 语义冲页（`\hbox to \hsize{}\vfill\penalty-2^30`）。
    pub fn typeset_dvi(&mut self, text: &str) -> Result<(Vec<BoxNode>, Vec<FontMetrics>)> {
        self.install_font_loader();
        self.expander.set_sink(Box::new(NodeBuilder::with_pagination(
            self.fonts.clone(),
            true,
        )));
        self.expander.run_source(text)?;
        let out = self.finish()?;
        Ok((out.shipped, out.fonts))
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
        // 1) 校验 + 关闭开放段落（可能产出页面 → box255 + pending）
        {
            let builder = self
                .expander
                .sink_mut()
                .as_any_mut()
                .downcast_mut::<NodeBuilder>()
                .ok_or_else(|| Error::internal("typesetter 安装了 NodeBuilder"))?;
            if builder.pending_box.is_some() {
                return Err(Error::invalid_input("\\hbox/\\vbox 后缺少组"));
            }
            if builder.shipout_next {
                return Err(Error::invalid_input("\\shipout 后缺少盒子"));
            }
            if !builder.groups.is_empty() {
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
            if !builder.math.is_empty() {
                return Err(Error::invalid_input("数学模式未闭合（缺少 $）"));
            }
            if builder.mode() == Mode::Horizontal {
                builder.close_paragraph();
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
        let mut lists = std::mem::take(&mut builder.lists);
        debug_assert_eq!(lists.len(), 1, "收尾后应只剩主列表");
        let shipped = std::mem::take(&mut builder.shipped);
        let fonts = match &self.fonts {
            Fonts::Tfm(table) => table.borrow().clone(),
            Fonts::Fn { .. } => Vec::new(),
        };
        // RFC-3：排版结束收尾 flush 残留延迟写流（TeX \end final_cleanup 语义）
        self.expander.flush_writes()?;
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
