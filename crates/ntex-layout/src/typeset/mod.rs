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
    hbox_dimensions, hpack, split_vbox, vbox_dimensions, vpack, BoxKind, BoxNode, FontId,
    LeadersKind, Node, GLUE_ORDER_FIL, GLUE_ORDER_FILL,
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
    /// `\underline`：内容子列表（底线渲染）。
    Underline { base: Vec<MathAtom> },
    /// `\overline`：内容子列表（顶线渲染）。
    Overline { base: Vec<MathAtom> },
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
    /// 重音符原子（`\accent`/`\mathaccent`）：accent 字段 + nucleus 字段
    /// （tex.web math_ac：重音符在前、被重音内容在后）。
    Accent {
        accent: Vec<MathAtom>,
        nucleus: Vec<MathAtom>,
    },
    /// 样式切换（`\displaystyle`/`\textstyle`/`\scriptstyle`/`\scriptscriptstyle`）。
    Style(MathStyle),
    /// 数学断行点（数学模式 `\penalty`；M4-1——tex.web math list 的 penalty 节点）。
    Penalty { penalty: i64 },
    /// 数学规则原子（数学模式 `\vrule`；M4-1）。
    Rule { width: i64, height: i64, depth: i64 },
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
    /// `\radical<delim>` 后的组（radicand；带定界符号，TRIP）。
    Radical(u32),
    /// `\mathbin` 等后的组：内容作为指定类原子。
    Class(MathClass),
    /// `\accent`/`\mathaccent` 后的组：内容为 nucleus（被重音）字段。
    Accent,
    /// `\underline` 后的组：内容为底线字段。
    Underline,
    /// `\overline` 后的组：内容为顶线字段。
    Overline,
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
    /// 输出例程的隐式组（tex.web group_code=output_group；ETRIP L396 检查）。
    Output,
    Math,
    /// `\left`/`\middle` 打开的数学定界组（tex.web group_code=math_left_group=16；
    /// `\right` 时闭合。ETRIP L356-358 `\left.\1 16` 检查）。
    MathLeft,
}

/// 对齐组方向（tex.web alignment：\halign 行堆叠 / \valign 列并排）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AlignDir {
    Halign,
    Valign,
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
            GroupKind::Output => 8,
            GroupKind::Math => 9,
            GroupKind::MathLeft => 16,
        }
    }

    /// tex.web group_names（\\tracinggroups 显示 `{entering <名> (level N)...}`）。
    fn group_name(self) -> &'static str {
        match self {
            GroupKind::Simple => "simple group",
            GroupKind::SemiSimple => "semi simple group",
            GroupKind::HBox => "hbox group",
            GroupKind::AdjustedHBox => "adjusted hbox group",
            GroupKind::VBox => "vbox group",
            GroupKind::VTop => "vtop group",
            GroupKind::Align => "align group",
            GroupKind::NoAlign => "no align group",
            GroupKind::Output => "output group",
            GroupKind::Math => "math group",
            GroupKind::MathLeft => "math left group",
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
    /// 本组是否为 `\leaders` 家族的引导盒子（封装结果挂起等胶水，不入当前列表）。
    leaders: Option<LeadersKind>,
    /// `\setbox<n>=` 的 RHS 盒子组目标寄存器：仅认领**最外层**盒子组
    /// （tex.web scan_box box_end 语义），内层嵌套 \hbox 组不得消费
    /// （否则 `\setbox0=\vbox{\hbox{...}}` 的 target 被内层盒抢走）。
    setbox: Option<usize>,
    /// 进入行号（etrip.tex 行；\tracinggroups 的 `entered at line L`）。
    entered_line: u32,
    /// 组打开时是否数学模式（`\hbox{A}` 数学字段关闭是合法流程；外层组
    /// 关闭时仍处数学模式才是缺 `$`）。
    entered_math: bool,
    /// 组深度（1-based；\tracinggroups 的 `(level N)`）。
    level: u32,
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

    /// 字体外部名（showbox 字符显示 `.\trip 1`；fn 指针占位无名字回退 `\font`）。
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

    /// 字符是否在字体中定义（tex.web `char_exists`；fn 指针占位一律视为存在）。
    fn char_exists(&self, font: FontId, charcode: u32) -> bool {
        match self {
            Fonts::Fn { .. } => true,
            Fonts::Tfm(table) => table
                .borrow()
                .get(font.0 as usize)
                .is_some_and(|fm| fm.char_exists(charcode)),
        }
    }

    /// 字体是否已加载（TFM 模式查表；fn 指针占位视为已加载）。
    fn loaded(&self, font: FontId) -> bool {
        match self {
            Fonts::Fn { .. } => true,
            Fonts::Tfm(table) => table.borrow().get(font.0 as usize).is_some(),
        }
    }

    /// 字体外部名（`\tracinglostchars` 警告用；tex.web slow_print(font_name[f])）。
    fn font_name(&self, font: FontId) -> String {
        match self {
            Fonts::Fn { .. } => String::new(),
            Fonts::Tfm(table) => table
                .borrow()
                .get(font.0 as usize)
                .map(|fm| fm.name.clone())
                .unwrap_or_default(),
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

/// `Params.misc` 下标：`\tracinglostchars`（与 ntex-core `int_param_index` 对齐）。
const MISC_TRACING_LOSTCHARS: usize = 1;

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
    /// `\\raise`/`\\lower`：下一个封装盒子的参考点位移（sp）。
    pending_shift: Option<i64>,
    /// `\\moveleft`/`\\moveright`：下一个封装盒子的水平位移（sp）。
    pending_hshift: Option<i64>,
    /// `\leaders`/`\cleaders`/`\xleaders`：已见引导符、等待盒子。
    pending_leaders: Option<LeadersKind>,
    /// 引导符盒子已就位（`\leaders\hbox{...}` 封装完成 / `\leaders\hrule`）、等待胶水。
    leaders_box: Option<(LeadersKind, Node)>,
    /// 诊断：累计追加节点数 + 最近追加节点（layout 侧死循环/OOM 定位用）。
    nodes_appended: u64,
    last_appended: String,
    /// 内部参数镜像（随 `param_changed` 事件更新，组作用域快照/恢复）。
    params: Params,
    /// e-TeX 惩罚数组镜像（随 `penalty_array_changed` 事件更新；kind 0-3：
    /// interline/club/widow/displaywidow）。折行时行间惩罚按索引取值，超出用末值。
    penalty_arrays: [Vec<i64>; 4],
    /// 组开始时的参数快照（group_end 恢复）。
    param_stack: Vec<Params>,
    /// `\sfcode` 表（随 `sfcode_changed` 事件更新；plain 默认 .,?!=3000、:=2000、
    /// ;=1500、,=1250，其余 1000）。
    sfcodes: [u32; 256],
    /// 组开始时的当前字体（group_end 恢复——TeX 字体选择**组作用域**：
    /// `{\bf bold} normal` 组内选择组外恢复；etrip L125 `\nullfont` 在组内
    /// 选择后 L129 `\endgroup` 须恢复 `\trip`，否则后续段落全 nullfont）。
    font_stack: Vec<FontId>,
    /// 当前 spacefactor（tex.web `space_factor`；段落/\hbox 开始 = 1000，
    /// 随字符 sfcode 更新，控制词间空格胶水）。
    space_factor: i64,
    /// `\noindent`：下一个段落不缩进。
    noindent_next: bool,
    /// 对齐组方向（\halign：行堆叠 vbox；\valign：列并排 hbox）。None = 非对齐组。
    align_dir: Option<AlignDir>,
    /// 对齐组已封装的列/行盒（\cr 分隔）。
    align_columns: Vec<Node>,
    /// 最近一次 \par 的源码行号（折行警告 `at lines a--b` 的结束行）。
    last_par_line: i64,
    /// 字体度量来源（M3-4：fn 指针占位或 TFM 字体表）。
    fonts: Fonts,
    /// `\font<cs>=<name>` 登记的 FontId → cs 名（showbox 字体标识显示
    /// `.\trip 1`；fmt 导入恢复 + `font_defined` 事件更新）。
    font_cs_names: Vec<Option<String>>,
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
    /// `\setbox` 组作用域变更日志（TeX 寄存器组级保存）：(组级, 下标, 旧值)。
    /// 组结束回滚本组及更深组内的盒子设置（trip L317 组内 `\setbox22=\lastbox`
    /// → L318 `}` 后参考 `restoring \box22=void`）。
    box_saves: Vec<(usize, usize, Option<BoxNode>)>,
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
    /// `\radical<delim>`：等待 radicand 字段（带定界符号；TRIP）。
    radical_pending: Option<u32>,
    /// `\mathbin` 等：等待字段（下一个原子或组），应用指定类。
    class_pending: Option<MathClass>,
    /// `\accent`/`\mathaccent`：重音符字段的 <15-bit number> 已扫描，等待 nucleus 字段。
    pub(super) accent_pending: bool,
    /// `\underline`：等待字段（组开收为 Underline 原子）。
    pub(super) underline_pending: bool,
    /// `\overline`：等待字段（组开收为 Overline 原子）。
    pub(super) overline_pending: bool,
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
    /// `\setbox` 的 `\global` 前缀（\global\setbox 不随组回滚——tex.web 语义）。
    setbox_global: bool,
    /// ETRIP 冲刺：盒子规格（`\hbox to/spread <dimen>`）：(to, spread)，随下一个盒子组生效。
    pending_box_spec: Option<(Option<i64>, Option<i64>)>,
    /// M4-4 显示数学：本次公式用短间距（前一段末行短于 `\displaywidth`）。
    display_short: bool,
    /// M4-4 显示数学：公式刚闭合，后续文字续排（不开新段：无 parskip/缩进）。
    after_display: bool,
    /// ETRIP 冲刺：数学间距参数（\\thinmuskip/\\medmuskip/\\thickmuskip =
    /// muskip 寄存器 0/1/2 的 mu glue；`muskip_param` 事件更新，
    /// 默认 thin=3mu/med=4mu±2mu∓4mu/thick=5mu±5mu）。
    /// 数学间距与排版事件 muskip_param 均属 mu 上下文路径；`width`/`stretch`/`shrink`
    /// 字段以 mu 单位存（1mu=1pt 数值=N×65536），math_to_hlist 内部按当前 style em/18 转 sp。
    muskip_params: [ntex_core::Glue; 3],
    /// 与 muskip_params 一一对应的 mu 单位标记：`\thinmuskip`/`\medmuskip`/`\thickmuskip`
    /// 永远绑 muskip 寄存器 0/1/2，全部按 mu 数值存；保留 `[bool;3]` 而非硬编码 `[true;3]`
    /// 是为未来承接 `\muskipdef` cs 绑到 muskip 时的同源同步。
    muskip_is_mu: [bool; 3],
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
    /// tex.web scan_math：待定数学字段（`\mathord`/`\mathaccent`/`\sqrt`/`\radical`/
    /// `^`/`_` 后）遇**非字符、非 `{` 组**的 token → othercases 分支 `back_input;
    /// scan_left_brace` 报 "Missing { inserted" 并隐含空字段恢复（TRIP L272
    /// `\mathord\radical`、L375 `^\leaders`、L396 `\accent\x\vfill`）。
    /// 调用点：所有非字符非组的数学事件入口（primitive/math_radical/math_sqrt/
    /// math_class/math_accent/fill_glue）。字符原子走 math_push_atom、组走
    /// group_begin——它们消费待定字段，不经过此检查（合法字段）。
    fn check_math_field_break(&mut self) -> Result<()> {
        if matches!(self.mode(), Mode::Math | Mode::DisplayMath)
            && (self.class_pending.is_some()
                || self.accent_pending
                || self.radical_pending.is_some()
                || self.sqrt_pending
                || self.pending_script.is_some())
        {
            self.report_error("Missing { inserted.");
            // scan_left_brace 隐含 `{`：TeX 报错后 cur_tok={ 开 math_group
            // （隐含空字段组）——**必须真的开组**，否则后续 `}` 越界关外层组
            // → Missing $ + close_math（TRIP l.272 `\mathord\radical"161` 缺 {，
            //    l.278 的 }}} 第 3 个 `}` 靠它配对；l.280 eqno 数学丢根因）。
            // 待定字段清空后开隐含组（Math 组：push math 层，由后续 `}` pop）。
            self.class_pending = None;
            self.accent_pending = false;
            self.radical_pending = None;
            self.sqrt_pending = false;
            self.pending_script = None;
            self.group_begin(0)?;
        }
        Ok(())
    }

    fn new(fonts: Fonts) -> Self {
        Self::with_pagination(fonts, false)
    }

    /// 全量同步参数镜像（`.fmt` 加载后：expander 的 params 已恢复，排版器需对齐）。
    pub fn sync_params(&mut self, p: &Params) {
        self.params = *p;
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
            pending_hshift: None,
            params: Params::default(),
            penalty_arrays: Default::default(),
            param_stack: Vec::new(),
            sfcodes,
            font_stack: Vec::new(),
            space_factor: 1000,
            noindent_next: false,
            font_cs_names: Vec::new(),
            align_dir: None,
            align_columns: Vec::new(),
            last_par_line: 0,
            current_font: FontId(0),
            shipout_next: false,
            shipped: Vec::new(),
            pagination,
            page: PageBuilder::new(),
            boxes: vec![None; REGISTER_COUNT],
            box_saves: Vec::new(),
            output_defined: false,
            pending_pages: VecDeque::new(),
            write_flush_pending: false,
            math: Vec::new(),
            math_style: MathStyle::Text,
            pending_script: None,
            sqrt_pending: false,
            radical_pending: None,
            class_pending: None,
            accent_pending: false,
            underline_pending: false,
            overline_pending: false,
            nonscript_pending: false,
            math_fonts: vec![[None; 3]; 16],
            patterns: PatternTrie::default(),
            hyph_exceptions: Vec::new(),
            setbox_target: None,
            setbox_global: false,
            pending_box_spec: None,
            display_short: false,
            after_display: false,
            // 默认数学间距（TeXbook p.170）：thin=3mu、med=4mu±2mu∓4mu、thick=5mu±5mu
            // ——按 mu 数值存（1mu=1pt 数值=N×65536）；math_to_hlist 内部按当前 style em/18 转 sp。
            muskip_params: [
                ntex_core::Glue::new(3 * SP_PER_PT, 0, 0),
                ntex_core::Glue::new(4 * SP_PER_PT, 2 * SP_PER_PT, 4 * SP_PER_PT),
                ntex_core::Glue::new(5 * SP_PER_PT, 5 * SP_PER_PT, 0),
            ],
            muskip_is_mu: [true; 3],
            pending_leaders: None,
            leaders_box: None,
            nodes_appended: 0,
            last_appended: String::new(),
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
        match self.list_modes.last() {
            Some(m) => *m,
            // 输入可达路径不得 panic：模式栈异常为空时退回垂直模式
            // （TRIP 错误恢复曾触发：`\par` 于数学模式恢复时 close_math/close_paragraph
            // 叠加弹出；防御回退 + 后续校准弹栈配对）
            None => {
                eprintln!("[ntex] mode() 栈空，回退 Vertical（错误恢复弹栈失衡）");
                Mode::Vertical
            }
        }
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

    /// TeX `\unhbox`/`\unvbox` 不可拆盒的错误恢复：写转录并继续
    /// （TRIP L396 `\unhbox234`——void 盒）。
    fn unbox_error_continue(&mut self) {
        let mut msg = "! Incompatible list can't be unboxed.\n".to_string();
        if matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
            msg.push_str("And I can't open any boxes in math mode.\n");
        }
        let _ = self.write16(msg);
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
        // 诊断：节点增长监控（TRIP L338 `\halign` 内挂死 = 列表无限增长 OOM；
        // 死循环在 layout 侧不经 process_one，VM 看门狗不计数）。
        self.nodes_appended += 1;
        self.last_appended = format!("{node:?}");
        if self.nodes_appended % 200_000 == 0 {
            eprintln!(
                "[layout-watchdog] nodes={} mode={:?} lists={} top_len={} groups={} last={:?}",
                self.nodes_appended,
                self.mode(),
                self.lists.len(),
                self.lists.last().map_or(0, |l| l.len()),
                self.groups.len(),
                self.last_appended
            );
        }
        // tex.web box_end（L20894 `shift_amount(cur_box):=box_context`）：
        // \raise/\lower/\moveleft/\moveright 的位移值对**下一个进入列表的盒子**
        // 生效（\box/\copy/\hbox 等所有 make_box 路径）。参照 append_to_vlist/
        // hlist_out 的语义：shift 沿盒子进入当前列表时写入，非包装完成时。
        let mut node = node;
        if let Node::Box(b) = &mut node {
            if let Some(v) = self.pending_shift.take() {
                b.shift = v;
            }
            // 水平位移（\moveleft/\moveright）：tex.web 同一 box_context 机制，
            // hlist 中 vbox 的 shift 也可表示水平偏移（vlist_out L12590
            // `cur_h:=left_edge+shift_amount(p)`）——同一字段按列表方向解释。
            if let Some(v) = self.pending_hshift.take() {
                b.shift = v;
            }
        }
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
            // \tracingpages：断页追踪输出到转录（tex.web begin_diagnostic → log）
            if let Some(t) = self.page.take_trace() {
                let _ = self.write16(t);
            }
        }
    }

    /// u8（0-7）→ 数学类别（tex.web math_char/scan_math 的类编号）。
    fn class_of(class: u8) -> MathClass {
        match class {
            0 => MathClass::Ord,
            1 => MathClass::Bin,
            2 => MathClass::Op,
            3 => MathClass::Rel,
            4 => MathClass::Open,
            5 => MathClass::Close,
            6 => MathClass::Punct,
            _ => MathClass::Inner,
        }
    }

    /// 存入盒子寄存器（TeX 寄存器组级保存）：组内记录旧值，组结束回滚。
    /// 返回旧值（`\box` 取走语义：读旧值 + 清空由调用方按返回值使用）。
    fn store_box(&mut self, idx: usize, value: Option<BoxNode>) -> Option<BoxNode> {
        let old = self.boxes.get(idx).cloned().flatten();
        if !self.groups.is_empty() && !self.setbox_global {
            self.box_saves.push((self.groups.len(), idx, old.clone()));
        }
        if let Some(slot) = self.boxes.get_mut(idx) {
            *slot = value;
        }
        old
    }

    fn package_box(
        &mut self,
        kind: PendingBox,
        ship: bool,
        leaders: Option<LeadersKind>,
        setbox: Option<usize>,
        boxmaxdepth: i64,
    ) {
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
                Node::Box(vpack(children, target, boxmaxdepth))
            }
            PendingBox::VTop => {
                // tex.web L21083-21087 Readjust：\vtop 的高度取首项高度（首项为
                // box/rule 时），depth 相应调整——不修改 shift_amount。
                let natural = vbox_dimensions(&children);
                let target = match spec {
                    Some((Some(to), _)) => to,
                    Some((_, Some(spread))) => natural.height + natural.depth + spread,
                    _ => natural.height + natural.depth,
                };
                let mut b = vpack(children, target, boxmaxdepth);
                let first_h = match b.children.first() {
                    // type(p)<=rule_node：box/rule 节点取 height，其余（glue 等）为 0
                    Some(Node::Box(inner)) => inner.height,
                    Some(Node::Rule { height, .. }) => *height,
                    _ => 0,
                };
                b.depth = b.depth - first_h + b.height;
                b.height = first_h;
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
        // `\moveleft`/`\moveright`：水平位移（TRIP 冲刺暂不落节点，取走即清）
        let _hshift = self.pending_hshift.take();
        // ETRIP 冲刺：`\setbox<n>=<box>` —— 封装结果存入寄存器（不入当前列表）。
        // 仅最外层 RHS 盒子组（group_begin 认领进 GroupCtx）持有目标；内层嵌套盒
        // （`\setbox0=\vbox{\hbox{...}}` 的 \hbox）不消费。
        if let Some(idx) = setbox {
            if let Node::Box(b) = node {
                self.store_box(idx, Some(b));
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
        // `\leaders` 引导盒子：封装结果挂起，等 \hskip/\vskip 胶水组成 Leader 节点。
        if let Some(ld) = leaders {
            self.leaders_box = Some((ld, node));
            return;
        }
        self.push_box(node);
    }

    /// 追加盒子到当前列表；垂直列表中前驱为盒子时插入 interline glue
    /// （tex.web `append_to_vlist`：d = \baselineskip − (depth 前 + height 新)，
    /// d < \lineskiplimit 用 \lineskip，否则用宽度调整为 d 的 \baselineskip）。
    fn push_node(&mut self, node: Node) {
        self.nodes_appended += 1;
        self.last_appended = format!("{node:?}");
        self.lists.last_mut().expect("列表栈非空").push(node);
    }

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
                // 行间惩罚节点（折行插入的 interline penalty）不阻断行间胶水：
                // Box → Penalty → Box 场景仍按"前驱是 Box"插 baselineskip glue
                match self
                    .lists
                    .last()
                    .and_then(|l| l.iter().rev().find(|n| !matches!(n, Node::Penalty { .. })))
                {
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
                    name: None,
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
    /// 字体中未定义的字符：tex.web `new_character` 返回 null（不建节点）并调
    /// `char_warning` 发 "Missing character" 警告（`\tracinglostchars>0` 时）。
    fn char_node(&mut self, tok: Token) -> Option<Node> {
        let charcode = tok.charcode()?;
        if !self.fonts.char_exists(self.current_font, charcode) {
            // 字体未加载（`ont` 失败后的悬空当前字体）：NTex 现状是不产生
            // 度量也不产生节点；tex.web 此时 font_name[f] 亦无定义，不发警告。
            if self.fonts.loaded(self.current_font) {
                self.char_warning(charcode);
            }
            return None;
        }
        let (w, h, d) = self.fonts.metrics(self.current_font, charcode);
        Some(Node::Char {
            font: self.current_font,
            charcode,
            width: w,
            height: h,
            depth: d,
        })
    }

    /// tex.web char_warning：`\tracinglostchars>0` 时报告缺失字符。
    /// 字符按 TeX 的不可打印渲染（`k<" "` 或 `k>"~"` → `^^` + 偏移/十六进制）。
    fn char_warning(&mut self, charcode: u32) {
        if self.params.misc[MISC_TRACING_LOSTCHARS] <= 0 {
            return;
        }
        let printable = (32..=126).contains(&charcode);
        let shown = if printable {
            char::from_u32(charcode).unwrap_or('?').to_string()
        } else if charcode < 64 {
            format!("^^{}", char::from_u32(charcode + 64).unwrap_or('?'))
        } else if charcode < 128 {
            format!("^^{}", char::from_u32(charcode - 64).unwrap_or('?'))
        } else {
            format!("^^{:x}{:x}", charcode / 16, charcode % 16)
        };
        let _ = self.write16(format!(
            "Missing character: There is no {shown} in font {}!",
            self.fonts.font_name(self.current_font)
        ));
    }

    /// TeX box_end leader 分支报错：引导盒子后缺少 `\hskip`/`\vskip`（tex.web L20927），
    /// 报错并丢弃引导盒子。
    fn report_leaders_misplaced(&mut self) {
        let _ = self.write16(
            "! Leaders not followed by proper glue.\n\
             You should say `\\leaders <box or rule><hskip or vskip>'.\n\
             I found the <box or rule>, but there's no suitable\n\
             <hskip or vskip>, so I'm ignoring these leaders.\n"
                .to_string(),
        );
        self.leaders_box = None;
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
            name: None,
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
            // 先取出前驱 (font, charcode)，避免借用冲突；尾节点为连字节点时
            // 以结果字符继续匹配（ffi 的第二次 f+i 匹配）
            let prev = match self.lists.last().and_then(|l| l.last()) {
                Some(Node::Char {
                    font: pf,
                    charcode: pc,
                    ..
                }) if *pf == font => Some((*pf, *pc)),
                Some(Node::Ligature {
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
                            // 尾节点替换为连字节点（tex.web ligature_node：
                            // lastnodetype=7；参考 showbox `.\trip r (ligature u|)`）。
                            // 连续连字（ffi）时尾 Ligature 继续参与匹配，components 累积。
                            let (w, h, d) = self.fonts.metrics(font, result as u32);
                            let last = self.lists.last_mut().and_then(|l| l.last_mut());
                            let replaced = match last {
                                Some(Node::Char {
                                    font: cf,
                                    charcode: rc,
                                    ..
                                }) => {
                                    let comps = vec![*rc as u8, charcode as u8];
                                    Some(Node::Ligature {
                                        font: *cf,
                                        charcode: result as u32,
                                        width: w,
                                        height: h,
                                        depth: d,
                                        components: comps,
                                    })
                                }
                                Some(Node::Ligature {
                                    font: cf,
                                    components: comps,
                                    ..
                                }) => {
                                    comps.push(charcode as u8);
                                    Some(Node::Ligature {
                                        font: *cf,
                                        charcode: result as u32,
                                        width: w,
                                        height: h,
                                        depth: d,
                                        components: std::mem::take(comps),
                                    })
                                }
                                _ => None,
                            };
                            if let Some(node) = replaced {
                                if let Some(l) = self.lists.last_mut() {
                                    *l.last_mut().expect("上面已检查非空") = node;
                                }
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

// \showbox 格式化（自由函数，迁自 sink.rs；依赖 mod.rs 已 use 的类型）
include!("sink_showbox.rs");

include!("tests.rs");
