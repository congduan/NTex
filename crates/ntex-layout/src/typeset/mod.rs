//! 排版器（M3-2）：模式状态机 + token→节点构建。
//!
//! 消费 ntex-core 展开引擎的排版事件（[`TokenSink`]），维护 TeX 模式状态机：
//!
//! - [`Mode::Vertical`]：垂直列表（页面主列表 / vbox 内容）；字符触发段落；
//! - [`Mode::Horizontal`]：段落水平列表；`\par` 或输入结束将其封装为 hbox；
//! - [`Mode::RestrictedHorizontal`]：`\hbox{...}` 内容。
//!
//! 排版原语参数（`\hskip`/`\vskip`/`\kern`/`\penalty`/`\hrule`/`\vrule`）
//! 由 VM 扫描，本层收到结构化结果；`\hbox`/`\vbox`/`\vtop`/`\par`/`\indent`
//! 由本层解释。内部参数（`\parindent`/`\baselineskip`/`\lineskip`/`\lineskiplimit`）
//! 经 `param_changed` 事件镜像，随组作用域快照/恢复。
//!
//! M3-2 范围说明（后续子步补齐）：
//! - 字体度量：M3-4 TFM 前由 [`Typesetter::with_metrics`] 提供 fn 指针（默认全零）；
//!   [`Typesetter::with_tfm`] 启用真实 TFM 度量（`\font\cs=cmr10` + 词间距来自字体参数）；
//! - `\hbox to <glue>` / `\hbox spread <glue>` 规格暂拒。

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use ntex_core::error::{Error, Result};
use ntex_core::expand::Expander;
use ntex_core::param::{ParamKind, ParamValue, Params};
use ntex_core::register::{Glue, REGISTER_COUNT, SP_PER_PT};
use ntex_core::token::Token;
use ntex_core::{FontLoader, Primitive, TokenSink};
use ntex_font::{FontMetrics, LigKern};

use crate::hyphen::PatternTrie;
use crate::linebreak::knuth_plass;
use crate::node::{
    hbox_dimensions, hpack, split_vbox, vbox_dimensions, vpack, BoxKind, BoxNode, FontId, Node,
    GLUE_ORDER_FIL, GLUE_ORDER_FILL,
};
use crate::page::PageBuilder;

/// 模式（TeX 模式状态机的 M3-2 子集 + M4 数学）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// 垂直模式：垂直列表（页面主列表 / vbox 内容）；字符触发段落。
    Vertical,
    /// 水平模式：段落水平列表；`\par` / 输入结束封装为 hbox。
    Horizontal,
    /// 受限水平模式：`\hbox{...}` 内容。
    RestrictedHorizontal,
    /// 数学模式（行内 `$...$`，textstyle）。
    Math,
    /// 显示数学模式（`$$...$$`，displaystyle）。
    DisplayMath,
}

/// 数学原子类别（TeXbook 附录 G：8 类原子）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MathClass {
    Ord,
    Bin,
    Op,
    Rel,
    Open,
    Close,
    Punct,
    Inner,
}

/// 数学样式（决定字阶与 spacing 表；TeXbook p.140-141）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MathStyle {
    Display,
    Text,
    Script,
    ScriptScript,
}

impl MathStyle {
    /// 字阶缩放（相对 textfont；TeX 数学三阶字体 text/script/scriptscript，
    /// 10pt 基字阶对应 7pt/5pt——ETRIP 前以比例近似，M4-3 用 fontdimen 精化）。
    fn scale(self) -> (i64, i64) {
        match self {
            MathStyle::Display | MathStyle::Text => (1, 1),
            MathStyle::Script => (7, 10),
            MathStyle::ScriptScript => (5, 10),
        }
    }

    /// 下一级（脚本的字阶）：Text→Script、Script→ScriptScript、Display→Script。
    fn next(self) -> MathStyle {
        match self {
            MathStyle::Display | MathStyle::Text => MathStyle::Script,
            MathStyle::Script => MathStyle::ScriptScript,
            MathStyle::ScriptScript => MathStyle::ScriptScript,
        }
    }
}

/// 数学字符原子：类 + 族 + 字符码。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MathChar {
    class: MathClass,
    fam: u8,
    charcode: u32,
}

/// 数学列表原子（M4-1/2：字符/上下标/分式/根式/定界符/类化/样式/空格）。
#[derive(Debug, Clone, PartialEq)]
enum MathAtom {
    /// 字符原子（8 类之一）。
    Char(MathChar),
    /// 上下标：`x^a` / `x_b` / `x_a^b`（字段为子列表）。
    Scripts {
        base: Box<MathAtom>,
        sub: Option<Vec<MathAtom>>,
        sup: Option<Vec<MathAtom>>,
    },
    /// 分式（`\over`/`\atop`/`\above`）：num/den 子列表；thickness 为分式线厚度
    /// （None=字体默认、Some(0)=`\atop` 无线）。
    Fraction {
        num: Vec<MathAtom>,
        den: Vec<MathAtom>,
        thickness: Option<i64>,
    },
    /// 根式（`\sqrt`）：radicand 子列表。
    Radical { base: Vec<MathAtom> },
    /// `\left<delim>...\right<delim>`：定界符为 None 表示空（`.`）。
    Delimited {
        left: Option<u32>,
        body: Vec<MathAtom>,
        right: Option<u32>,
    },
    /// e-TeX `\middle<delim>`（\left...\right 内分隔符；类 Inner）。
    Middle(Option<u32>),
    /// 显式定类字段（`\mathbin{...}` 等）：内容作为一个指定类的原子。
    Classed {
        class: MathClass,
        content: Vec<MathAtom>,
    },
    /// 样式切换（`\displaystyle`/`\textstyle`/`\scriptstyle`/`\scriptscriptstyle`）。
    Style(MathStyle),
    /// 数学空格（`\mskip`/`\mkern` 结果；`nonscript`：`\nonscript` 后脚本模式丢弃）。
    MSkip {
        width: i64,
        stretch: i64,
        shrink: i64,
        nonscript: bool,
    },
    /// 已排版盒子（数学模式内 `\hbox{...}` 产出）。
    Box(BoxNode),
}

/// 数学组字段类别（group_begin 压层时记录；group_end 按类别处理）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MathFieldKind {
    /// 脚本字段（`^`/`_` 后的组）：Some(is_sup)。
    Script(bool),
    /// `\sqrt` 后的组（radicand）。
    Sqrt,
    /// `\mathbin` 等后的组：内容作为指定类原子。
    Class(MathClass),
}

/// 数学列表层级：原子列表 + 是否为特殊字段组。
#[derive(Debug, Default)]
struct MathLevel {
    atoms: Vec<MathAtom>,
    /// 本组字段类别（`^`/`_`、`\sqrt`、`\mathbin` 后的 `{...}`）；None = 普通组。
    field: Option<MathFieldKind>,
    /// `\left` 打开本层的定界符（None = 普通层）；`\right` 时收为 Delimited 原子。
    left: Option<Option<u32>>,
    /// 本层待定分式（`\over`/`\atop`；numerator 已收集，atoms 继续收 denominator）。
    fraction: Option<FractionPending>,
}

/// `\over`/`\atop` 中间态：numerator 已收集，当前数学层 atoms 继续收集 denominator。
#[derive(Debug)]
struct FractionPending {
    thickness: Option<i64>,
    num: Vec<MathAtom>,
}

/// 数学间距码（TeXbook 附录 G 规则 18）：0 无 / 1 thin / 2 med / 3 thick / 4 *（紧排）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SpacingCode {
    None,
    Thin,
    Med,
    Thick,
    Tight,
}

/// 待封装盒子种类（`\hbox`/`\vbox`/`\vtop` 的下一个组）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingBox {
    HBox,
    VBox,
    VTop,
}

impl PendingBox {
    fn is_vertical(self) -> bool {
        matches!(self, Self::VBox | Self::VTop)
    }
}

/// 组种类（TeX group code，tex.web §291 / e-TeX 扩展）。
///
/// ETRIP 用 `\currentgrouptype` 检查的组类型码：simple=1、hbox=2、
/// adjusted hbox=3、vbox=4、vtop=5、align=6、no align=7、math=9、
/// semi simple=14；bottom=0，math shift=15（数学模式由 `$` 进入时）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GroupKind {
    Simple,
    SemiSimple,
    HBox,
    AdjustedHBox,
    VBox,
    VTop,
    Align,
    NoAlign,
    Math,
}

impl GroupKind {
    fn code(self) -> i64 {
        match self {
            GroupKind::Simple => 1,
            GroupKind::SemiSimple => 14,
            GroupKind::HBox => 2,
            GroupKind::AdjustedHBox => 3,
            GroupKind::VBox => 4,
            GroupKind::VTop => 5,
            GroupKind::Align => 6,
            GroupKind::NoAlign => 7,
            GroupKind::Math => 9,
        }
    }
}

/// 组上下文（group_begin 压栈，group_end 弹出）。
#[derive(Debug)]
struct GroupCtx {
    /// 组种类（`\currentgrouptype` 查询用）。
    kind: GroupKind,
    /// 本组是否为盒子内容（`\hbox`/`\vbox`/`\vtop` 紧邻的组；对齐组复用 vbox）。
    box_kind: Option<PendingBox>,
    /// 本组是否为 `\shipout` 的目标（封装的盒子作为页面而非追加）。
    /// 随组传递：`\shipout\vbox{...\box255...}` 内层盒子不被 shipout。
    shipout: bool,
}

/// 字符度量函数：`(width, height, depth)`，单位 sp。
pub type MetricsFn = fn(FontId, u32) -> (i64, i64, i64);

/// 词间空白胶水函数（空格 token → 胶水；M3-4 TFM 前由调用方提供）。
pub type SpaceFn = fn(FontId) -> Glue;

/// 字体度量来源：fn 指针占位（M3-4 前 / ntex-pdf 临时切片）或 TFM 字体表（M3-4）。
#[derive(Debug, Clone)]
enum Fonts {
    /// fn 指针占位：字符维度/词间距由调用方提供（`with_metrics`/`with_space`）。
    Fn { metrics: MetricsFn, space: SpaceFn },
    /// TFM 字体表（`with_tfm`）：FontId → 度量；`\font` 加载时追加。
    /// `Rc<RefCell>` 让加载器（[`TfmLoader`]）与节点构建器共享同一张表。
    Tfm(Rc<RefCell<Vec<FontMetrics>>>),
}

impl Fonts {
    fn metrics(&self, font: FontId, charcode: u32) -> (i64, i64, i64) {
        match self {
            Fonts::Fn { metrics, .. } => (metrics)(font, charcode),
            Fonts::Tfm(table) => table
                .borrow()
                .get(font.0 as usize)
                .map(|fm| fm.char_metrics(charcode))
                .unwrap_or((0, 0, 0)),
        }
    }

    fn space(&self, font: FontId) -> Glue {
        match self {
            Fonts::Fn { space, .. } => (space)(font),
            Fonts::Tfm(table) => table
                .borrow()
                .get(font.0 as usize)
                .map(|fm| fm.space_glue())
                .unwrap_or(Glue::ZERO),
        }
    }

    /// `\sfcode≥2000` 空格追加的 extra_space（cmr10 = 72818 sp；M3-4 词间距）。
    fn extra_space(&self, font: FontId) -> i64 {
        match self {
            Fonts::Fn { .. } => 0,
            Fonts::Tfm(table) => table
                .borrow()
                .get(font.0 as usize)
                .map(|fm| fm.extra_space)
                .unwrap_or(0),
        }
    }

    /// 查左字符的 lig/kern 程序（TFM 模式；fn 指针占位无程序）。
    fn lig_kern(&self, font: FontId, left: u8, right: u8) -> Option<LigKern> {
        match self {
            Fonts::Fn { .. } => None,
            Fonts::Tfm(table) => table
                .borrow()
                .get(font.0 as usize)
                .and_then(|fm| fm.apply_lig_kern(left, right)),
        }
    }

    /// 数学 em（quad = fontdimen 6；数学间距 1em 基准）。fn 指针占位返回 0。
    fn quad(&self, font: FontId) -> i64 {
        match self {
            Fonts::Fn { .. } => 0,
            Fonts::Tfm(table) => table
                .borrow()
                .get(font.0 as usize)
                .map(|fm| fm.quad)
                .unwrap_or(0),
        }
    }

    /// x 高度（fontdimen 5；数学上标提升基准）。fn 指针占位返回 0。
    fn x_height(&self, font: FontId) -> i64 {
        match self {
            Fonts::Fn { .. } => 0,
            Fonts::Tfm(table) => table
                .borrow()
                .get(font.0 as usize)
                .map(|fm| fm.x_height)
                .unwrap_or(0),
        }
    }

    /// 字体参数（fontdimen，M4-3）：`idx` 为 TeX 参数号（1 起）；无则 0。
    fn font_param(&self, font: FontId, idx: usize) -> i64 {
        match self {
            Fonts::Fn { .. } => 0,
            Fonts::Tfm(table) => table
                .borrow()
                .get(font.0 as usize)
                .and_then(|fm| fm.font_params.get(idx.saturating_sub(1)).copied())
                .unwrap_or(0),
        }
    }
}

/// 节点构建 sink：把 VM 排版事件转成节点列表。
#[derive(Debug)]
struct NodeBuilder {
    /// 列表栈（栈顶 = 当前列表；栈底 = 主垂直列表）。
    /// 栈内元素按入栈顺序：盒子内容、段落。`[list_modes]` 与之并行，
    /// 记录每个列表的模式（段落 = Horizontal，hbox 内容 = RestrictedHorizontal，
    /// vbox 内容 / 主列表 = Vertical）。作用域组（非盒子）不压列表。
    lists: Vec<Vec<Node>>,
    list_modes: Vec<Mode>,
    /// 组上下文栈（与列表栈独立：作用域组只压 ctx）。
    groups: Vec<GroupCtx>,
    /// 等待下一个组的盒子种类。
    pending_box: Option<PendingBox>,
    /// 等待下一个组的显式种类（`\begingroup`/`\valign`/`\noalign`；优先于 pending_box）。
    pending_kind: Option<GroupKind>,
    /// `\raise`/`\lower`：下一个封装盒子的参考点位移（sp）。
    pending_shift: Option<i64>,
    /// 内部参数镜像（随 `param_changed` 事件更新，组作用域快照/恢复）。
    params: Params,
    /// 组开始时的参数快照（group_end 恢复）。
    param_stack: Vec<Params>,
    /// `\sfcode` 表（随 `sfcode_changed` 事件更新；plain 默认 .,?!=3000、:=2000、
    /// ;=1500、,=1250，其余 1000）。
    sfcodes: [u32; 256],
    /// 当前 spacefactor（tex.web `space_factor`；段落/\hbox 开始 = 1000，
    /// 随字符 sfcode 更新，控制词间空格胶水）。
    space_factor: i64,
    /// `\noindent`：下一个段落不缩进。
    noindent_next: bool,
    /// 字体度量来源（M3-4：fn 指针占位或 TFM 字体表）。
    fonts: Fonts,
    /// 当前字体（TFM 模式由 `font_selected` 事件更新；fn 指针模式恒为 FontId(0)）。
    current_font: FontId,
    /// `\shipout`：下一个封装盒子作为页面（DVI shipout，M3-5）。
    shipout_next: bool,
    /// 已 \shipout 的页面（按顺序）。
    shipped: Vec<BoxNode>,
    /// M3-5-2 断页：启用自动分页（`typeset_dvi` 打开；旧 `typeset` 保持切片行为）。
    pagination: bool,
    /// 页面构建器（`pagination` 时把顶层垂直列表拆成页面）。
    page: PageBuilder,
    /// 盒子寄存器（M3-5-3）：`\box<n>` 读写（box255 为待输出例程页面队列，见
    /// [`Self::pending_pages`]，不占此表）。
    boxes: Vec<Option<BoxNode>>,
    /// `\output` 例程是否已定义（true：fire_up 改道 box255 + 待执行）。
    output_defined: bool,
    /// 待输出例程处理的页面队列（M3-5-3）：`\output` 定义时 fire_up 产出的页面
    /// 排队，`\box255` 逐页取出，例程反复运行直至队列清空。队列而非单槽——
    /// `close_paragraph` 一次推入多行可能连续产出多页，逐页交错执行例程。
    pending_pages: VecDeque<BoxNode>,
    /// RFC-3：页面真正输出（`\shipout` 边界）时置位，通知引擎 flush 延迟写流。
    write_flush_pending: bool,
    /// 数学列表栈（M4-1）：数学模式期间一层；`{...}` 数学组/脚本字段压层。
    math: Vec<MathLevel>,
    /// 当前数学样式（进入 Math=Text、DisplayMath=Display；`\displaystyle` 等修改）。
    math_style: MathStyle,
    /// 待挂载的脚本方向（`^`=Some(true)、`_`=Some(false)）：等待下一个原子/组。
    pending_script: Option<bool>,
    /// `\sqrt`：等待 radicand 字段（下一个原子或组）。
    sqrt_pending: bool,
    /// `\mathbin` 等：等待字段（下一个原子或组），应用指定类。
    class_pending: Option<MathClass>,
    /// `\nonscript`：下一个数学空格在脚本模式丢弃。
    nonscript_pending: bool,
    /// 数学字体族表（M4-3）：16 族 × 3 阶（text/script/scriptscript）。
    /// `\textfont<fam>=<cs>` 等原语分配；字符按族+字阶选字体。
    math_fonts: Vec<[Option<FontId>; 3]>,
    /// 断字模式表（M4-6）：`\patterns{...}` 解析后的 Liang trie。
    patterns: PatternTrie,
    /// ETRIP 冲刺：断字异常词表（`\hyphenation{...}`）：小写字母 + 允许断点
    /// （0 = 词首、len = 词尾）。断字时优先于模式表。
    hyph_exceptions: Vec<(Vec<u8>, Vec<usize>)>,
    /// ETRIP 冲刺：`\setbox<n>=<box>` 目标寄存器（下一个封装盒子存入该槽）。
    setbox_target: Option<usize>,
    /// ETRIP 冲刺：盒子规格（`\hbox to/spread <dimen>`）：(to, spread)，随下一个盒子组生效。
    pending_box_spec: Option<(Option<i64>, Option<i64>)>,
    /// M4-4 显示数学：本次公式用短间距（前一段末行短于 `\displaywidth`）。
    display_short: bool,
    /// M4-4 显示数学：公式刚闭合，后续文字续排（不开新段：无 parskip/缩进）。
    after_display: bool,
    /// ETRIP 冲刺：终端转录累积（`\message`/`\show`/`\write16`）。
    transcript: String,
    /// ETRIP 冲刺：e-TeX marks 族状态（断页轮转）。
    /// `\topmarks<c>`：继承自上一页 botmarks<c>（初始空）。
    marks_top: std::collections::HashMap<i64, String>,
    /// 当前页第一个出现的 marks<c>（断页新页开始时清空）。
    marks_first: std::collections::HashMap<i64, String>,
    /// 当前页最后一个出现的 marks<c>（断页新页开始时保留继承值，后续新 marks 覆盖）。
    marks_bot: std::collections::HashMap<i64, String>,
    /// \vsplit 产生的拆分 marks（暂未实现 vsplit 全语义；留空）。
    marks_split_top: std::collections::HashMap<i64, String>,
    marks_split_first: std::collections::HashMap<i64, String>,
    marks_split_bot: std::collections::HashMap<i64, String>,
    /// ETRIP 第二波：`\lastbox` 摘下的盒子（TeX 语义：供下一个 `\box`/`\copy` 使用）。
    lastbox_hold: Option<BoxNode>,
}

/// 断字候选字符：ASCII 字母（catcode 11 的近似；ligature/非字母不参与断字 run）。
fn is_alpha(charcode: u32) -> bool {
    charcode < 128 && (charcode as u8).is_ascii_alphabetic()
}

impl NodeBuilder {
    fn new(fonts: Fonts) -> Self {
        Self::with_pagination(fonts, false)
    }

    /// 创建构建器；`pagination` 打开 M3-5-2 断页（自动分页 + parskip + 收尾冲页）。
    fn with_pagination(fonts: Fonts, pagination: bool) -> Self {
        // plain 格式 \sfcode 默认（TeXbook p.75）：.,?! = 3000、: = 2000、; = 1500、, = 1250
        let mut sfcodes = [1000u32; 256];
        for (c, v) in [
            (b'.', 3000),
            (b'?', 3000),
            (b'!', 3000),
            (b':', 2000),
            (b';', 1500),
            (b',', 1250),
        ] {
            sfcodes[c as usize] = v;
        }
        Self {
            lists: vec![Vec::new()],
            list_modes: vec![Mode::Vertical],
            groups: Vec::new(),
            pending_box: None,
            pending_kind: None,
            pending_shift: None,
            params: Params::default(),
            param_stack: Vec::new(),
            sfcodes,
            space_factor: 1000,
            noindent_next: false,
            current_font: FontId(0),
            shipout_next: false,
            shipped: Vec::new(),
            pagination,
            page: PageBuilder::new(),
            boxes: vec![None; REGISTER_COUNT],
            output_defined: false,
            pending_pages: VecDeque::new(),
            write_flush_pending: false,
            math: Vec::new(),
            math_style: MathStyle::Text,
            pending_script: None,
            sqrt_pending: false,
            class_pending: None,
            nonscript_pending: false,
            math_fonts: vec![[None; 3]; 16],
            patterns: PatternTrie::default(),
            hyph_exceptions: Vec::new(),
            setbox_target: None,
            pending_box_spec: None,
            display_short: false,
            after_display: false,
            transcript: String::new(),
            fonts,
            marks_top: std::collections::HashMap::new(),
            marks_first: std::collections::HashMap::new(),
            marks_bot: std::collections::HashMap::new(),
            marks_split_top: std::collections::HashMap::new(),
            marks_split_first: std::collections::HashMap::new(),
            marks_split_bot: std::collections::HashMap::new(),
            lastbox_hold: None,
        }
    }

    fn mode(&self) -> Mode {
        *self.list_modes.last().expect("列表栈非空")
    }

    /// ETRIP 冲刺：断页后 marks 轮转（fire_up 产出页后立即调用）。
    /// 语义（TeX）：
    ///   marks_top = 旧 marks_bot（上一页 bot 变成新页 top 继承值）；
    ///   marks_first 清空（新页第一个 marks 尚未出现）；
    ///   marks_bot = marks_top（新页无新 marks 时 bot==继承的 top）。
    fn rotate_marks(&mut self) {
        // marks_top：承接上一页 botmarks（继承）
        self.marks_top = self.marks_bot.clone();
        // marks_first：新页第一个 marks 清空
        self.marks_first.clear();
        // marks_bot：初始等于继承的 marks_top（若无新 marks 则 bot==top）
        self.marks_bot = self.marks_top.clone();
    }

    /// ETRIP 第二波：取盒子寄存器内容（`copy=false` 取出置 void；`copy=true` 复制保留）。
    /// `\unhbox`/`\unvbox`/`\unhcopy`/`\unvcopy` 共用；void 盒子报错。
    fn take_or_clone_box(&mut self, idx: usize, copy: bool) -> Result<BoxNode> {
        if copy {
            self.boxes
                .get(idx)
                .and_then(|s| s.as_ref())
                .cloned()
                .ok_or_else(|| Error::invalid_input(format!("盒子 {idx} 为空（void）")))
        } else {
            self.boxes
                .get_mut(idx)
                .and_then(|s| s.take())
                .ok_or_else(|| Error::invalid_input(format!("盒子 {idx} 为空（void）")))
        }
    }

    fn append(&mut self, node: Node) {
        self.lists.last_mut().expect("列表栈非空").push(node);
        // M3-5-2：顶层垂直模式追加后运行页面构建器（TeX build_page 的触发点）。
        // 增量（feed_one）：每产出一页即暂停——若定义了输出例程，让引擎在 token
        // 边界执行例程（ship box255）后再继续；未定义时页面直通 shipped。
        if self.pagination && self.mode() == Mode::Vertical && self.lists.len() == 1 {
            if let Some(p) = self.page.feed_one(&mut self.lists[0], &self.params) {
                self.accept_page(p);
                // ETRIP 冲刺：断页 marks 轮转（top = 旧 bot，first 清空，bot 保留继承）
                self.rotate_marks();
            }
        }
    }

    /// 接收一个已产出的页面（M3-5-3）：定义了输出例程 → 进入待处理队列
    /// （`\box255` 逐页取出）；否则直接进 shipped（与 M3-5-2 默认行为一致）。
    fn package_box(&mut self, kind: PendingBox, ship: bool) {
        let children = self.lists.pop().expect("盒子列表");
        self.list_modes.pop();
        // ETRIP 冲刺：\hbox/\vbox to/spread 规格（目标宽/高）
        let spec = self.pending_box_spec.take();
        let node = match kind {
            PendingBox::HBox => {
                let natural = hbox_dimensions(&children).width;
                let target = match spec {
                    Some((Some(to), _)) => to,
                    Some((_, Some(spread))) => natural + spread,
                    _ => natural,
                };
                Node::Box(hpack(&children, target))
            }
            PendingBox::VBox => {
                let natural = vbox_dimensions(&children);
                let target = match spec {
                    Some((Some(to), _)) => to,
                    Some((_, Some(spread))) => natural.height + natural.depth + spread,
                    _ => natural.height + natural.depth,
                };
                Node::Box(vpack(children, target))
            }
            PendingBox::VTop => {
                // \vtop：维度同 vbox，参考点移到首行基线（shift 待 M3-5 对 DVI 校准）。
                let natural = vbox_dimensions(&children);
                let target = match spec {
                    Some((Some(to), _)) => to,
                    Some((_, Some(spread))) => natural.height + natural.depth + spread,
                    _ => natural.height + natural.depth,
                };
                let mut b = vpack(children, target);
                b.shift = b.height;
                Node::Box(b)
            }
        };
        // `\raise`/`\lower`：封装结果应用参考点位移（\raise 向上为正）
        let mut node = node;
        if let Some(shift) = self.pending_shift.take() {
            if let Node::Box(b) = &mut node {
                b.shift = shift;
            }
        }
        // ETRIP 冲刺：`\setbox<n>=<box>` —— 封装结果存入寄存器（不入当前列表）
        if let Some(idx) = self.setbox_target.take() {
            if let Node::Box(b) = node {
                self.boxes[idx] = Some(b);
            }
            return;
        }
        if ship {
            if let Node::Box(b) = node {
                self.shipped.push(b);
                self.write_flush_pending = true;
            }
            return;
        }
        // 数学模式内 `\hbox{...}`：结果盒子转数学原子（TeX 数学盒子）。
        if matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
            let atom = match node {
                Node::Box(b) => MathAtom::Box(b),
                other => unreachable!("数学模式盒子封装必产出 Box 节点：{other:?}"),
            };
            self.math_push_atom(atom).expect("数学模式盒子挂载");
            return;
        }
        self.push_box(node);
    }

    /// 追加盒子到当前列表；垂直列表中前驱为盒子时插入 interline glue
    /// （tex.web `append_to_vlist`：d = \baselineskip − (depth 前 + height 新)，
    /// d < \lineskiplimit 用 \lineskip，否则用宽度调整为 d 的 \baselineskip）。
    fn push_box(&mut self, node: Node) {
        if self.mode() == Mode::Vertical {
            // 分页模式下顶层前驱盒子的深度/类型：页面构建器里的盒子，或
            // 断页后仍在贡献列表中的残余盒子（未入页，interline glue 的依据）。
            let (prev_is_box, prev_depth) = if self.pagination && self.lists.len() == 1 {
                match self.lists[0]
                    .iter()
                    .rev()
                    .find(|n| matches!(n, Node::Box(_) | Node::Rule { .. }))
                {
                    Some(Node::Box(prev)) => (true, prev.depth),
                    Some(Node::Rule { depth, .. }) => (true, *depth),
                    _ => (
                        self.page.prev_depth() > crate::page::IGNORE_DEPTH,
                        self.page.prev_depth(),
                    ),
                }
            } else {
                match self.lists.last().and_then(|l| l.last()) {
                    Some(Node::Box(prev)) => (true, prev.depth),
                    _ => (false, 0),
                }
            };
            if prev_is_box {
                let height = match &node {
                    Node::Box(b) => b.height,
                    _ => 0,
                };
                let d = self.params.baselineskip.width - (prev_depth + height);
                let g = if d < self.params.lineskiplimit {
                    self.params.lineskip
                } else {
                    let mut g = self.params.baselineskip;
                    g.width = d;
                    g
                };
                self.append(Node::Glue {
                    width: g.width,
                    stretch: g.stretch,
                    shrink: g.shrink,
                    stretch_order: 0,
                    shrink_order: 0,
                });
            }
        }
        self.append(node);
    }

    /// 插入段落缩进（TeX `new_graf`）：\parindent>0 空盒，<0 kern，=0 无；
    /// `\noindent` 抑制。
    fn insert_indent(&mut self) {
        if self.noindent_next {
            self.noindent_next = false;
            return;
        }
        let ind = self.params.parindent;
        if ind > 0 {
            self.append(Node::Box(BoxNode {
                kind: BoxKind::HBox,
                width: ind,
                height: 0,
                depth: 0,
                shift: 0,
                children: Vec::new(),
            }));
        } else if ind < 0 {
            self.append(Node::Kern { width: ind });
        }
    }

    /// 字符 token → Char 节点；非字符（控制序列等）返回 None。
    fn char_node(&self, tok: Token) -> Option<Node> {
        let charcode = tok.charcode()?;
        let (w, h, d) = self.fonts.metrics(self.current_font, charcode);
        Some(Node::Char {
            font: self.current_font,
            charcode,
            width: w,
            height: h,
            depth: d,
        })
    }

    /// 词间空白胶水（tex.web `append_normal_space` / `app_space`）：
    /// spacefactor = 1000 → 字体空格原样；否则按 `app_space` 调整——sf≥2000
    /// 宽度加 extra_space；stretch ×= sf/1000；shrink ×= 1000/sf（xn_over_d 舍入）。
    fn append_space_glue(&mut self) {
        let g = self.fonts.space(self.current_font);
        let (width, stretch, shrink) = if self.space_factor == 1000 {
            (g.width, g.stretch, g.shrink)
        } else {
            let sf = self.space_factor;
            let width = if sf >= 2000 {
                g.width + self.fonts.extra_space(self.current_font)
            } else {
                g.width
            };
            (
                width,
                xn_over_d(g.stretch, sf, 1000),
                xn_over_d(g.shrink, 1000, sf),
            )
        };
        self.append(Node::Glue {
            width,
            stretch,
            shrink,
            stretch_order: 0,
            shrink_order: 0,
        });
    }

    /// 水平模式追加字符（tex.web main_loop 子集）：
    /// 1. 更新 spacefactor（adjust_space_factor，对被连字消费的字符也执行）；
    /// 2. 与列表尾同字体字符查 lig/kern 程序——kern 在其前插入 kern 节点；
    ///    lig 把尾字符替换为结果字符并丢弃当前字符（ffi/ffl 由结果字符的
    ///    程序在下一字符到来时自然连续匹配）。
    fn append_char(&mut self, node: Node) {
        if let Node::Char {
            font,
            charcode,
            width,
            height,
            depth,
        } = node
        {
            self.adjust_space_factor(charcode);
            // 先取出前驱 (font, charcode)，避免借用冲突
            let prev = match self.lists.last().and_then(|l| l.last()) {
                Some(Node::Char {
                    font: pf,
                    charcode: pc,
                    ..
                }) if *pf == font => Some((*pf, *pc)),
                _ => None,
            };
            let mut drop_cur = false;
            if let Some((pf, pc)) = prev {
                if let Some(action) = self.fonts.lig_kern(pf, pc as u8, charcode as u8) {
                    match action {
                        LigKern::Kern(kern) => self.append(Node::Kern { width: kern }),
                        LigKern::Lig(result) => {
                            // 尾字符替换为结果字符（fi/fl/ff 等），当前字符丢弃
                            let (w, h, d) = self.fonts.metrics(font, result as u32);
                            if let Some(Node::Char {
                                charcode: rc,
                                width: rw,
                                height: rh,
                                depth: rd,
                                ..
                            }) = self.lists.last_mut().and_then(|l| l.last_mut())
                            {
                                *rc = result as u32;
                                *rw = w;
                                *rh = h;
                                *rd = d;
                            }
                            drop_cur = true;
                        }
                    }
                }
            }
            if !drop_cur {
                self.append(Node::Char {
                    font,
                    charcode,
                    width,
                    height,
                    depth,
                });
            }
        } else {
            self.append(node);
        }
    }

    /// tex.web `adjust_space_factor`：sfcode=0 不变；=1000 → 1000；<1000 且 >0
    /// → 取该值；>1000 且当前 <1000 → 1000；否则取该值。
    fn adjust_space_factor(&mut self, charcode: u32) {
        let s = self.sfcodes.get(charcode as usize).copied().unwrap_or(1000) as i64;
        if s == 1000 {
            self.space_factor = 1000;
        } else if s < 1000 {
            if s > 0 {
                self.space_factor = s;
            }
        } else if self.space_factor < 1000 {
            self.space_factor = 1000;
        } else {
            self.space_factor = s;
        }
    }
}

// ---------- Typesetter 段（TfmLoader + Typesetter + FinishOutput） ----------
include!("typesetter.rs");

// 方法分片（include! 嵌入，原 impl 按域拆分）
include!("paragraph.rs");
include!("paging.rs");
include!("math.rs");
include!("sink.rs");

include!("tests.rs");
