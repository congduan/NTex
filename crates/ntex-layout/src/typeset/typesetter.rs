
// NodeBuilder 主 impl（含分页/段落方法）在此闭合；数学/TokenSink 分片各自开启新 impl。
/// TFM 字体加载器（M3-4）：`\font` 执行时按名字查找/解析 TFM，追加到共享字体表。
#[derive(Debug)]
struct TfmLoader {
    table: Rc<RefCell<Vec<FontMetrics>>>,
}

/// 按名字装载字体并追加进共享字体表（[`TfmLoader`] 与 char_node 的 CJK 字体
/// 回落共用的装载缝；表内同名字+同缩放去重，幂等）。
pub(super) fn load_font_into_table(
    table: &Rc<RefCell<Vec<FontMetrics>>>,
    name: &str,
    at: Option<i64>,
    scaled: Option<i64>,
) -> Result<u32> {
    if at.is_some() && scaled.is_some() {
        return Err(Error::invalid_input("\\font 的 at 与 scaled 不能同时给出"));
    }
        let mut fm = load_metrics(name)?;
        fm.name = name.to_owned(); // DVI fnt_def 的字体名
        // at：目标尺寸/设计字号；scaled：千分比
        // （OpenType 字体的 design_size_sp 由 ntex_font::build_metrics 置为 10pt
        //   基准，故此处两条字体路径共用同一套缩放逻辑，DVI fnt_def 语义一致。）
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
        let mut table = table.borrow_mut();
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
                char_italic: Vec::new(),
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
                next_larger: Vec::new(),
                font_params: Vec::new(),
                unicode_native: false,
                unicode_chars: Vec::new(),
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

impl FontLoader for TfmLoader {
    fn load(&mut self, name: &str, at: Option<i64>, scaled: Option<i64>) -> Result<u32> {
        load_font_into_table(&self.table, name, at, scaled)
    }

    /// 字体字符度量查询（`\iffontchar`/`\fontchar*`）：字体表与排版器共享。
    /// Unicode 直映字体（OTF）按码位二分，TFM 走 8-bit 槽表（见
    /// [`FontMetrics::char_metrics_opt`]）。
    fn char_metric(&mut self, font: u32, ch: u32) -> Option<(i64, i64, i64)> {
        let table = self.table.borrow();
        let fm = table.get(font as usize)?;
        fm.char_metrics_opt(ch)
    }

    /// 合法字符码上限（`\char`/`\iffontchar`/`\fontchar*` 的校验上界）。
    ///
    /// 8-bit 字体（TFM）→ 255：保持 TeX 语义的 "Bad character code" 硬口径
    /// （`reference/trip/tripin.log` 与 `fixtures/etrip/etrip.log` 均有
    /// `! Bad character code (256).` 参考块，不能放宽）；
    /// Unicode 直映字体（OTF）→ 0x10FFFF：对齐 XeTeX，使 `\char"4E00` 可排汉字。
    fn char_code_limit(&mut self, font: u32) -> u32 {
        let table = self.table.borrow();
        match table.get(font as usize) {
            Some(fm) if fm.unicode_native => ntex_core::font::UNICODE_MAX_CHARCODE,
            _ => 255,
        }
    }

    /// 字体参数查询（em/ex 内部单位：param 5=x_height、6=quad）：
    /// `font_params[param-1]`（已按 scaled 缩放）；nullfont/越界 → None。
    fn font_param(&mut self, font: u32, param: usize) -> Option<i64> {
        if param == 0 {
            return None;
        }
        let table = self.table.borrow();
        let fm = table.get(font as usize)?;
        fm.font_params.get(param - 1).copied()
    }

    /// TFM 声明的 fontdimen 参数个数（tex.web `font_params[f]`）。
    /// nullfont 固定 7（tex.web L10780-10787 初始化：`font_params[null_font]:=7`
    /// ——pdfTeX 实测 `\fontdimen20\nullfont` 报 "Font \nullfont has only 7
    /// fontdimen parameters."）；未装载的槽 → None。
    fn param_count(&mut self, font: u32) -> Option<usize> {
        if font == 0 {
            return Some(7);
        }
        let table = self.table.borrow();
        table
            .get(font as usize)
            .map(|fm| fm.font_params.len())
    }

    /// 最近装载的字体号（tex.web `font_ptr`）：表长-1。表空 → None。
    fn last_font(&mut self) -> Option<u32> {
        let n = self.table.borrow().len();
        u32::try_from(n.checked_sub(1)?).ok()
    }
}

/// 字体名是否显式带 OpenType 文件后缀（`.otf`/`.ttf`/`.ttc`，大小写不敏感）。
///
/// 带后缀时不试 TFM——避免"名字里写死了字体文件"却被同名 TFM 抢走。
fn is_open_type_name(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.ends_with(".otf") || n.ends_with(".ttf") || n.ends_with(".ttc")
}

/// 按名字取字体度量（M9 中文刀 1 的双路径分派）。
///
/// 分派顺序与兼容性契约：
/// 1. **宿主注册的 OpenType 字节**（wasm/Tauri 前端 fetch 注入）最优先；
/// 2. 名字带 OpenType 后缀 → 直接走 OpenType；
/// 3. 否则先按 TFM 找（**原路径**，native 行为逐字节不变）；
/// 4. TFM 找不到才回落 OpenType——中文/西文 OpenType 字体没有 TFM，靠这一步命中；
/// 5. 都没有 → 报既有的 `找不到 TFM 文件：<name>`（保持消息不变，TRIP/ETRIP
///    缺字体路径有硬口径）。
fn load_metrics(name: &str) -> Result<FontMetrics> {
    if let Some(bytes) = crate::registered_otf_bytes(name) {
        let fm = ntex_font::build_metrics(bytes, name)
            .map_err(|e| Error::invalid_input(format!("解析字体 {name}: {e}")))?;
        return Ok(register_otf_metrics(name, fm));
    }
    if is_open_type_name(name) {
        return load_otf_from_fs(name)?
            .ok_or_else(|| Error::invalid_input(format!("找不到字体文件：{name}")));
    }
    // TFM 字节来源（M8-A WASM 骨架线分叉）：
    // ① 宿主注册的 [`crate::TfmSource`]（wasm32 无文件系统，唯一来源；native 可
    //    显式 opt-in）；未注册 → None 回落 ②。
    // ② native 文件系统（`find_tfm` + `std::fs::read`，原路径，native 默认走这里
    //    ——注册表为空时行为与历史版本逐字节一致）；wasm32 下 `std::fs` 不可用。
    let tfm_bytes: Option<Vec<u8>> = match crate::registered_tfm_bytes(name) {
        Some(bytes) => Some(bytes),
        None => {
            #[cfg(not(target_arch = "wasm32"))]
            {
                ntex_font::find_tfm(name).and_then(|path| std::fs::read(&path).ok())
            }
            #[cfg(target_arch = "wasm32")]
            {
                None
            }
        }
    };
    if let Some(bytes) = tfm_bytes {
        return ntex_font::parse_tfm(&bytes)
            .map_err(|e| Error::invalid_input(format!("解析 {name}: {e}")));
    }
    // OpenType 兜底（无 TFM 的字体：中文 Fandol/思源、西文 OTF）
    if let Some(fm) = load_otf_from_fs(name)? {
        return Ok(fm);
    }
    #[cfg(target_arch = "wasm32")]
    {
        return Err(Error::invalid_input(format!(
            "找不到 TFM 字节：{name}（wasm 无文件系统，宿主须 set_tfm_source 注册字体源）"
        )));
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        Err(Error::invalid_input(format!("找不到 TFM 文件：{name}")))
    }
}

/// 文件系统查找并构建 OpenType 度量：`Ok(None)` = 找不到文件（非错误，
/// 由调用方决定回落或报错），`Err` = 找到了但解析失败。
#[cfg(not(target_arch = "wasm32"))]
fn load_otf_from_fs(name: &str) -> Result<Option<FontMetrics>> {
    let Some(path) = ntex_font::find_otf(name) else {
        return Ok(None);
    };
    let bytes = std::fs::read(&path).map_err(|e| Error::io("读取字体", path, e))?;
    let fm = ntex_font::build_metrics(bytes, name)
        .map_err(|e| Error::invalid_input(format!("解析字体 {name}: {e}")))?;
    Ok(Some(register_otf_metrics(name, fm)))
}

/// 排版器现场合成的 OpenType 度量**同步登记**到 `ntex-font` 进程级注册表，
/// 并原样返回，供 `\font` 装载路径继续使用。
///
/// 为什么必须登记：`ntex-pdf`（PDF 写出）取度量只有「TFM 字节 → parse」与
/// 进程级注册表两条路（`read_tfm` / `registered_metrics`），它**不**走本
/// 模块的 OpenType 合成链。无 TFM 的中文字体（Fandol 等）只合成不登记，
/// 则 native `ntex-dvi` → `ntex-pdf` 在写 PDF 时报
/// `找不到 TFM：FandolSong-Regular`（2026-09-17 现场：resume1-plain.tex）。
/// wasm 侧无此问题——`ntex-wasm` 的 `set_otf_font` 注入时已登记同一条缝
/// （见 `ntex-wasm/src/lib.rs`）；本函数补齐 native 的对应动作。
///
/// 返回值语义是"写寄存器"：锁毒化只影响登记（PDF 侧回落报错），
/// 不影响排版本身，故不作错误传播（与 `register_tfm_bytes` 同口径）。
fn register_otf_metrics(name: &str, fm: FontMetrics) -> FontMetrics {
    let _ = ntex_font::register_metrics(name, fm.clone());
    fm
}

#[cfg(target_arch = "wasm32")]
fn load_otf_from_fs(_name: &str) -> Result<Option<FontMetrics>> {
    Ok(None)
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
    /// 与 [`Self::shipped`] 一一对应的各页 `\count0..9` 快照（输出例程刀 5）。
    shipped_counts: Vec<[i64; 10]>,
    /// `.fmt` 恢复的 `\count0..9`（install_builder 播种页号链镜像用；`.fmt` 的
    /// 寄存器不经 count_changed 事件，install 边界是唯一可取点——输出例程刀 5）。
    fmt_page_counts: [i64; 10],
    /// .fmt 导入的当前字体（import_state 时 sink 未装，install_builder 同步）。
    fmt_current_font: u32,
    /// finish 收走的当前字体（take_sink 后 NodeBuilder 不可达，export_state 用）。
    last_current_font: u32,
    /// plain 格式预载开关（格式预载 G2(a)）：排版入口在用户源前先跑内嵌
    /// [`plain_format::PLAIN_TEX`]。默认关——TRIP/latex probe/corpus math 等
    /// INITEX 语义调用方不受影响；`ntex-dvi` 等面向 plain 文档的入口显式打开。
    preload_plain: bool,
    /// 断字模式预载开关：排版入口在用户源前先跑内嵌 [`plain_format::HYPHEN_TEX`]
    /// （US patterns + 例外词表，落 `\language=0`）。给 **LaTeX fmt 路径**用的——
    /// 真 latex.fmt 在格式生成期载入断字表（lthyphen.dtx），而 NTex 的 `.fmt`
    /// 只序列化 core 侧状态（`ntex-core::expand::FmtState`），断字表住在
    /// 排版器（`PatternTrie`）跨不进快照，于是 fmt 恢复后 `\language=0` 无表，
    /// 全文档不断词：窄版心（quotation/abstract，`\parshape` 收窄到
    /// hsize−2×leftmargin）一行溢出 35pt、全篇 Overfull 26 处（GT 0 处）。
    /// plain 预载不需要它——plain.tex:1222 自己 `\input hyphen`。
    ///
    /// 默认关：INITEX/TRIP 语义零影响（真 TRIP 断字表为空，自动补表会改
    /// 断行）。与 [`Self::preload_plain`] 互斥使用（双开会把表登记两遍）。
    preload_hyphen: bool,
    /// 表格宏包层预载开关（LaTeX 路径）：排版入口在用户源前先跑内嵌
    /// [`plain_format::BOOKTABS_COMPAT_TEX`]（booktabs/multirow 最小语义）。
    /// 给**不 \usepackage 却使用 \toprule/\multirow 等宏名的源**一个符合源码
    /// 意图的版面（真 pdflatex 对这类源同样报 Undefined control sequence，
    /// 表线缺失、宏名参数漏成文字）。\providecommand 定义，真宏包载入仍以
    /// 包版为准；plain/INITEX/TRIP 路径不受影响（默认关）。
    preload_compat: bool,
    /// 内嵌格式 VFS 兜底层是否已包（[`Self::use_embedded_format`] 幂等标记）。
    embedded_vfs_installed: bool,
    /// UTF-8 输入默认开关（M9 中文刀 3）：开则排版入口在用户源之前把
    /// `\utfinputmode`（`param::MISC_UTF_INPUT_MODE`）置 1，源文件可直接写
    /// 中文（输入层把 UTF-8 多字节合并成单个 21-bit 字符 token）。
    ///
    /// 做成引擎开关而不是"调用方在源码前拼一行"的理由：拼一行会让用户源
    /// 整体下移，log/转录里的 `l.N` 与编辑器行号错位——而 `l.N` 是 TeX
    /// 定位的第一现场（docs/tooling-trust.md 的仪器可信度纪律）。走
    /// [`Expander::set_misc_int`] 则源文本逐字节不动。
    ///
    /// 默认关：bytes 是引擎既有语义，TRIP/ETRIP/expl3/latex-probe 口径零
    /// 影响（`ntex-wasm` 的 Tauri/浏览器前端显式打开）。
    utf8_input_default: bool,
    /// PDF 模式默认开关（`\pdfoutput`，misc 63）。NTex 最终仍写 DVI，但
    /// workbench 可打开此兼容位，让 `graphics.cfg` 选择 `pdftex.def`，从而
    /// 走引擎实现的 `\pdfximage`/`\pdfrefximage` 图片管线。
    /// 默认关，保持 native DVI/TRIP 口径不变；用户源内显式赋值后写覆盖。
    pdf_output_default: bool,
    /// CJK 字间断点默认开关（`\cjkbreakmode`，misc 66）：做成引擎级默认值
    /// 而非动 `\cjkbreakmode` 的 INITEX 默认——后者保持 0（tex.web 原义：汉字
    /// 之间既无胶水也无断点，plain/TRIP/ETRIP 口径零影响）。前端（CLI
    /// `--cjk-fallback`/`--utf8`、wasm 工作台）显式要中文排版能力时才打开，
    /// 源内 `\cjkbreakmode=0` 仍可关（后写覆盖先写，同 [`Self::utf8_input_default`]）。
    cjk_break_mode_default: bool,
    /// CJK 字体回落名（workbench 档）：`char_node` 里当前字体缺字形且码位
    /// 超过 0xFF 时自动改用它排该字符。默认 None——TRIP/ETRIP/native 语义零
    /// 影响（`ntex-wasm` 的 Tauri 前端显式下发，如 `FandolSong-Regular`）。
    fallback_font: Option<String>,
}

/// INITEX/plain 大写字母 `\sfcode=999`（tex.web §4852 `for k:="A" to "Z" ...
/// sf_code(k):=999`；小写保持 1000，标点默认已在 NodeBuilder 表内）。
/// 缺它则大写后的空格因子不被钳制："LaTeX." 误得句末胶水（+extra_space、
/// stretch×3、shrink÷3）、"TeX," 误得 sf1250 胶水、"TeX}" 后丢 sf999
/// 混合胶水——demo1 首行自然宽 +1.11pt、收缩总 −0.96pt，glue set
/// 0.705→0.809，词间空格 w167106 → w159540（官方对照实测）。
/// 全量与增量两条 install 路径都必须套用（增量镜像见 incremental.rs）。
fn init_sfcodes(builder: &mut NodeBuilder) {
    for k in b'A'..=b'Z' {
        builder.sfcodes[k as usize] = 999;
    }
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
            shipped_counts: Vec::new(),
            fmt_page_counts: [0; 10],
            fmt_current_font: 0,
            last_current_font: 0,
            preload_plain: false,
            preload_hyphen: false,
            preload_compat: false,
            utf8_input_default: false,
            pdf_output_default: false,
            cjk_break_mode_default: false,
            fallback_font: None,
            embedded_vfs_installed: false,
        }
    }

    /// 指定词间空白胶水函数（空格 token → 胶水；仅 fn 指针模式生效）。
    pub fn with_space(mut self, space: SpaceFn) -> Self {
        if let Fonts::Fn { space: s, .. } = &mut self.fonts {
            *s = space;
        }
        self
    }

    /// iniTeX（INITEX / 格式构建态）语义：EXPANDER 换用 tex.web §1273 的初始
    /// catcode 表（LaTeX 兼容铺开：latex.ltx L99 靠 `\{`=12 判别"纯 initex"）。
    /// 默认 plain 风格表保持不变，plain/TRIP 路径不受影响。
    pub fn initex(mut self) -> Self {
        self.expander = self.expander.initex();
        self
    }

    /// 注入 VFS 后端（RFC-3；`\input`/`\write` 等副作用原语的文件接口）。
    pub fn set_vfs(&mut self, vfs: Box<dyn ntex_io::Vfs>) {
        self.expander.set_vfs(vfs);
    }

    /// 设置 TeX 作业名（`\jobname`），用于 aux/toc 等派生文件路径。
    pub fn set_job_name(&mut self, name: impl Into<String>) {
        self.expander.set_job_name(name);
    }

    /// 设置交互模式（0=batch, 1=nonstop, 2=scroll, 3=errorstop）。
    pub fn set_interaction_mode(&mut self, mode: i64) {
        self.expander
            .set_misc_int(ntex_core::param::MISC_INTERACTION_MODE, mode.clamp(0, 3));
    }

    /// 取回 VFS（测试断言写入内容用）。
    pub fn take_vfs(&mut self) -> Box<dyn ntex_io::Vfs> {
        self.expander.take_vfs()
    }

    /// 接入内嵌 plain 格式文件（格式预载 G2(a)）：`\input plain`/`\input hyphen`
    /// 在本地/宿主 VFS 全落空时改读内嵌资源（[`EmbeddedFormatVfs`] 兜底层，
    /// 本地命中优先，不改变既有搜索语义）。幂等：已包过不再包。
    ///
    /// 与 [`Self::set_preload_plain`] 独立——「内嵌文件可被 `\input` 到」是
    /// 分发能力，「启动自动跑 plain.tex」是格式开关；后者隐含前者。
    pub fn use_embedded_format(&mut self) {
        if self.embedded_vfs_installed {
            return;
        }
        let inner = self.expander.take_vfs();
        self.expander
            .set_vfs(Box::new(EmbeddedFormatVfs::new(inner)));
        self.embedded_vfs_installed = true;
    }

    /// plain 格式预载开关（格式预载 G2(a)）：排版入口（[`Self::typeset`] /
    /// [`Self::typeset_bytes`] / [`Self::typeset_dvi`]）在用户源前先跑内嵌
    /// [`PLAIN_TEX`]，等价于源文件首行 `\input plain`。
    ///
    /// 连同 [`Self::use_embedded_format`] 一起打开（plain.tex:1222 的
    /// `\input hyphen` 也走内嵌）。builder 风格 [`Self::plain_format`]。
    pub fn set_preload_plain(&mut self, on: bool) {
        if on {
            self.use_embedded_format();
        }
        self.preload_plain = on;
    }

    /// [`Self::set_preload_plain`]`(true)` 的 builder 形式（链式构造）。
    pub fn plain_format(mut self) -> Self {
        self.set_preload_plain(true);
        self
    }

    /// plain 格式预载是否已开（诊断/测试用）。
    pub fn preload_plain(&self) -> bool {
        self.preload_plain
    }

    /// 断字模式预载开关（LaTeX fmt 路径）：排版入口在用户源前先跑内嵌
    /// [`plain_format::HYPHEN_TEX`]。动机与互斥约束见字段 [`Self::preload_hyphen`]。
    pub fn set_preload_hyphen(&mut self, on: bool) {
        if on {
            self.use_embedded_format();
        }
        self.preload_hyphen = on;
    }

    /// 表格宏包层预载开关（LaTeX 路径）：见字段 [`Self::preload_compat`]。
    pub fn set_preload_compat(&mut self, on: bool) {
        if on {
            self.use_embedded_format();
        }
        self.preload_compat = on;
    }

    /// UTF-8 输入默认开关（M9 中文刀 3）：开则每次排版在用户源之前把
    /// `\utfinputmode` 置 1（详见字段 [`Self::utf8_input_default`]）。
    ///
    /// 用户源里显式的 `\utfinputmode=0` 仍生效——它后写、覆盖本默认。
    /// 关闭（false）会把该参数**复位为 0**，使同一个 Typesetter 实例在
    /// 多次排版之间不残留前一作业的开关。
    pub fn set_utf8_input(&mut self, on: bool) {
        self.utf8_input_default = on;
        self.apply_utf8_input_default();
    }

    /// 设置 workbench 的 PDF 兼容模式。打开后，每次排版入口先置
    /// `\pdfoutput=1`；用户源码仍可显式改回 0。
    pub fn set_pdf_output(&mut self, on: bool) {
        self.pdf_output_default = on;
    }

    /// [`Self::set_utf8_input`] 的 builder 形式（链式构造）。
    pub fn utf8_input(mut self, on: bool) -> Self {
        self.set_utf8_input(on);
        self
    }

    /// UTF-8 输入默认是否已开（诊断/测试用）。
    pub fn utf8_input_on(&self) -> bool {
        self.utf8_input_default
    }

    /// CJK 字体回落（workbench 档）：`char_node` 里当前字体缺字形且码位
    /// 超过 0xFF（utf8 输入才可能，TRIP/ETRIP 的 8-bit 路径零影响）时，自动
    /// 改用该字体排这个字符——源文件不写 `\font\zh=FandolSong-Regular`
    /// 也能排中文（Tauri 工作台「plain 简历中文全 Missing character」现场
    /// 的修复，2026-09-18）。传 `None` 关闭（默认）。
    ///
    /// 名字经 `load_metrics` 解析：宿主注册的 OTF 字节（wasm `set_otf_font`）
    /// 最优先。回落字符的度量/字形/PDF 嵌入走该字体自己的通路。
    pub fn set_fallback_font(&mut self, name: Option<String>) {
        self.fallback_font = name;
    }

    /// 把 [`Self::utf8_input_default`] 落到 `\utfinputmode`（每次排版入口调用）。
    fn apply_utf8_input_default(&mut self) {
        let on = i64::from(self.utf8_input_default);
        self.expander
            .set_misc_int(ntex_core::param::MISC_UTF_INPUT_MODE, on);
    }

    /// CJK 字间断点默认开关（`\cjkbreakmode`）：P0「全角标点段落不断行」的
    /// 落点。引擎默认仍是 0（见字段文档）；只由前端在要中文排版能力时显式
    /// 打开。`close_paragraph` 据此在可断字间（含全角标点的行首/行尾禁则）
    /// 插零宽胶水——不开则中文段落整段单行 Overfull 出页。
    pub fn set_cjk_break_mode(&mut self, on: bool) {
        self.cjk_break_mode_default = on;
    }

    /// 每次排版入口把引擎级默认值落到 expander（与 [`Self::apply_utf8_input_default`]
    /// 同一时机：fmt 载入/预载之前、用户源之前——源内显式赋值后写覆盖）。
    ///
    /// 排版器镜像要单独写：`\cjkbreakmode` 的消费端 `close_paragraph` 读
    /// builder 的 `params.misc` 镜像（paragraph.rs），而镜像只经 param_changed
    /// 事件或 `sync_params` 对齐——`set_misc_int` 不发事件。且 install_builder
    /// 的 `sync_params` 在本调用之前跑，镜像里还是旧值。
    fn apply_layout_defaults(&mut self) {
        self.expander.set_misc_int(
            ntex_core::param::MISC_PDF_OUTPUT,
            i64::from(self.pdf_output_default),
        );
        let on = i64::from(self.cjk_break_mode_default);
        self.expander
            .set_misc_int(ntex_core::param::MISC_CJK_BREAK_MODE, on);
        if let Some(b) = self
            .expander
            .sink_mut()
            .as_any_mut()
            .downcast_mut::<NodeBuilder>()
        {
            b.params.misc[ntex_core::param::MISC_CJK_BREAK_MODE] = on;
        }
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
        // 页号链初值（输出例程刀 5）：fmt 恢复的 \count0..9，install_builder 播种
        let mut fmt_page_counts = [0i64; 10];
        for (i, c) in state.registers.counts.iter().take(10).enumerate() {
            fmt_page_counts[i] = *c;
        }
        self.fmt_page_counts = fmt_page_counts;
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
            shipped_counts: Vec::new(),
            fmt_page_counts: [0; 10],
            fmt_current_font: 0,
            last_current_font: 0,
            preload_plain: false,
            preload_hyphen: false,
            preload_compat: false,
            utf8_input_default: false,
            pdf_output_default: false,
            cjk_break_mode_default: false,
            fallback_font: None,
            embedded_vfs_installed: false,
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
            shipped_counts: Vec::new(),
            fmt_page_counts: [0; 10],
            fmt_current_font: 0,
            last_current_font: 0,
            preload_plain: false,
            preload_hyphen: false,
            preload_compat: false,
            utf8_input_default: false,
            pdf_output_default: false,
            cjk_break_mode_default: false,
            fallback_font: None,
            embedded_vfs_installed: false,
        }
    }

    /// 排版源码，返回主垂直列表节点。
    pub fn typeset(&mut self, text: &str) -> Result<Vec<Node>> {
        self.install_font_loader();
        self.install_builder(NodeBuilder::new(self.fonts.clone()));
        self.apply_utf8_input_default();
        self.apply_layout_defaults();
        self.run_plain_preload()?;
        self.run_hyphen_preload()?;
        self.run_compat_preload()?;
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
        self.apply_utf8_input_default();
        self.apply_layout_defaults();
        self.run_plain_preload()?;
        self.run_hyphen_preload()?;
        self.run_compat_preload()?;
        self.expander.feed_source(bytes);
        self.expander.run()?;
        self.finish().map(|out| {
            self.shipped = out.shipped;
            out.main
        })
    }

    /// plain 格式预载（G2(a)）：开关开着才跑，且只在用户源前跑一次。
    /// 出错即失败——plain.tex 是格式的一部分，格式坏了不该静默带病排版
    /// （G0 教训：静默是最大测量陷阱）。
    fn run_plain_preload(&mut self) -> Result<()> {
        if !self.preload_plain {
            return Ok(());
        }
        self.expander.run_source(plain_format::PLAIN_TEX)
    }

    /// 断字模式预载（[`Self::set_preload_hyphen`]）：开关开着才跑。出错即失败
    /// ——与 [`Self::run_plain_preload`] 同纪律，断字表缺失不该静默带病排版
    /// （plain-format-survey §3.2：跳过 patterns = 「plain 预载了但断词表缺失」
    /// 的新静默偏差）。
    fn run_hyphen_preload(&mut self) -> Result<()> {
        if !self.preload_hyphen {
            return Ok(());
        }
        self.expander.run_source(plain_format::HYPHEN_TEX)
    }

    /// 表格宏包层预载（[`Self::set_preload_compat`]）：开关开着才跑。
    fn run_compat_preload(&mut self) -> Result<()> {
        if !self.preload_compat {
            return Ok(());
        }
        self.expander.run_source(plain_format::BOOKTABS_COMPAT_TEX)
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
        // 页号链播种（输出例程刀 5）：.fmt 恢复的 \count0..9 不经 count_changed
        // 事件，镜像须取 fmt 初值（TRIP：pass1 dump 前 \count4 已到 11）
        builder.page_state.page_counts = self.fmt_page_counts;
        // .fmt 恢复的 \output token 列表在 expander 侧，但新建 NodeBuilder
        // 默认认为输出例程未定义；若不同步，LaTeX 的 \@outputpage 会被绕过。
        builder.page_state.output_defined = self.expander.output_defined();
        // .fmt 导入的当前字体（防 pass2 字符全 nullfont + Missing 警告）
        builder.current_font = FontId(self.fmt_current_font);
        // pass2 NodeBuilder 重建：同步数学间距参数（\\thinmuskip 等 muskip 寄存器——
//  pass1 赋值在 dump 前，pass2 不重跑赋值事件）。muskip_is_mu 同步为全 true：
// muskip 寄存器 0/1/2（= thinmuskip/medmuskip/thickmuskip）的 width/stretch/shrink
// 字段永为 mu 数值（expander 端 scan_glue_mu 路径按 1mu=65536 单位存）。
        builder.math_state.muskip_params = self.expander.muskip_registers();
        builder.math_state.muskip_is_mu = [true; 3];
        init_sfcodes(&mut builder);
        builder.set_fallback_font_name(self.fallback_font.clone());
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
            .map(|b| std::mem::take(&mut b.io_state.transcript))
            .unwrap_or_default()
    }

    /// 排版源码并取回 `\shipout` 页面（DVI 输出，M3-5）：
    /// 返回 (页面列表, 字体表快照)。需 [`Self::with_tfm`] 模式（否则字体表为空）。
    /// 启用 M3-5-2 断页：顶层垂直列表经页面构建器自动分页（`\vsize`），
    pub fn typeset_dvi(&mut self, text: &str) -> Result<(Vec<BoxNode>, Vec<FontMetrics>)> {
        self.install_font_loader();
        self.install_builder(NodeBuilder::with_pagination(self.fonts.clone(), true));
        self.apply_utf8_input_default();
        self.apply_layout_defaults();
        self.run_plain_preload()?;
        self.run_hyphen_preload()?;
        self.run_compat_preload()?;
        self.expander.run_source(text)?;
        let out = self.finish()?;
        self.shipped = out.shipped.clone();
        Ok((out.shipped, out.fonts))
    }

    /// 排版结束后取回已 shipout 的页面列表（[`Self::finish`] 已执行时有效）。
    pub fn shipped_pages(&self) -> &[BoxNode] {
        &self.shipped
    }

    /// 各页面 shipout 边界的 `\count0..9` 快照（与 [`Self::shipped_pages`]
    /// 一一对应；输出例程刀 5 页号链——DVI bop 计数取值源）。
    pub fn shipped_page_counts(&self) -> &[[i64; 10]] {
        &self.shipped_counts
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
            if builder.box_state.pending_box.is_some() {
                if ended {
                    builder.box_state.pending_box = None;
                } else {
                    return Err(Error::invalid_input("\\hbox/\\vbox 后缺少组"));
                }
            }
            if builder.page_state.shipout_next {
                if ended {
                    builder.page_state.shipout_next = false;
                } else {
                    return Err(Error::invalid_input("\\shipout 后缺少盒子"));
                }
            }
            if !builder.groups.is_empty() {
                if ended {
                    // TeX：\end 时组未闭合 → 警告不中断（trip.log L7293）。
                    // 仅显式 \end 豁免：tex.web final_cleanup 收尾。纯 EOF 时
                    // 残留本身就是要报告的错误（M4-7），不能反过来当豁免条件
                    // ——58665be 曾用「数学列表非空」放宽此处，使未闭合数学
                    // 静默吞掉（math_unclosed/left_without_right 回归）。
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
                        builder.box_state.pending_box,
                        builder.box_state.pending_kind,
                        builder.math_state.math.len()
                    );
                    return Err(Error::invalid_input(&m));
                }
            }
            if !builder.math_state.math.is_empty() {
                if ended {
                    // TeX：\end 时数学列表未闭合 → 同样警告不中断。纯 EOF 的
                    // 残留按 M4-7 报错（下方 else），不得因「组已清空」豁免。
                    let _ = builder.write16("(end occurred inside a math list)\n".to_string());
                    builder.math_state.math.clear();
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
                if builder.page_state.pagination {
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
        // 4) 最终列表清理（tex.web final_end：冲页循环内 \output 例程可能重新
        //    打开水平列表（trip.tex `\output{\unvbox255\end\rb}` 等——例程体在
        //    错误恢复后未闭合），此时再收段 + 丢弃残留层，仅留主垂直列表。
        //    TRIP 崩溃根因（2026-09-03）：ended-clean 在冲页前执行，冲页后
        //    残留层未清 → finish 断言 lists.len()==1 违约（畸形输入 panic）。
        {
            let builder = self
                .expander
                .sink_mut()
                .as_any_mut()
                .downcast_mut::<NodeBuilder>()
                .ok_or_else(|| Error::internal("typesetter 安装了 NodeBuilder"))?;
            if builder.mode() == Mode::Horizontal {
                builder.close_paragraph();
            }
            while builder.lists.len() > 1 {
                builder.lists.pop();
                builder.list_modes.pop();
            }
        }
        // 5) 取走 sink，收集主列表/页面/字体表
        let mut sink = self.expander.take_sink();
        let builder = sink
            .as_any_mut()
            .downcast_mut::<NodeBuilder>()
            .ok_or_else(|| Error::internal("typesetter 安装了 NodeBuilder"))?;
        // 转录留档（finish 后 sink 被 VecSink 替换，take_transcript 读不到 NodeBuilder）
        self.last_transcript = std::mem::take(&mut builder.io_state.transcript);
        // 当前字体存档（take_sink 后 export_state 读不到 NodeBuilder）
        self.last_current_font = builder.current_font.0;
        let mut lists = std::mem::take(&mut builder.lists);
        debug_assert_eq!(lists.len(), 1, "收尾后应只剩主列表");
        let shipped = std::mem::take(&mut builder.page_state.shipped);
        let shipped_counts = std::mem::take(&mut builder.page_state.shipped_counts);
        let fonts = match &self.fonts {
            Fonts::Tfm(table) => table.borrow().clone(),
            Fonts::Fn { .. } => Vec::new(),
        };
        // RFC-3：排版结束收尾 flush 残留延迟写流（TeX \end final_cleanup 语义）
        self.expander.flush_writes()?;
        self.shipped = shipped.clone();
        self.shipped_counts = shipped_counts;
        Ok(FinishOutput {
            main: lists.pop().expect("主列表"),
            shipped,
            fonts,
        })
    }
}

/// `finish` 的返回：主垂直列表 + `\shipout` 页面 + 字体表快照。
/// （各页 `\count0..9` 快照随 `Typesetter::shipped_page_counts` 取用，不在此传递。）
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
