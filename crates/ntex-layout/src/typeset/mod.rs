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
use ntex_core::{
    AlignSink, BoxSink, CoreSink, FontLoader, FontSink, IoSink, MathSink, PageSink, Primitive,
    TokenSink,
};
use ntex_font::{FontMetrics, LigKern};

use crate::hyphen::PatternTrie;
use crate::linebreak::{knuth_plass, parshape_line_indent, parshape_line_width};
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

/// 数学原子类别（TeXbook 附录 G：8 类原子；Var 为第 9 类"变量字母"，
/// initex 默认 mathcode 字母 = x+"7100 → class 7）。
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
    /// 变量字母（class 7）：间距按 Ord 查表（TeXbook 附录 G 规则 18 表
    /// 无 Var 行，tex.web `var_noad` 同 `ord_noad` 处理）。
    Var,
}

/// 数学样式（决定字阶与 spacing 表；TeXbook p.140-141）。编码随 tex.web
/// L13568-13572：偶数 = 未压、奇数 = 压（+1）：display=0、text=2、script=4、
/// scriptscript=6；下标迁移一律取压臂（sub_style/denom_style/radicand）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MathStyle {
    Display,
    CrampedDisplay,
    Text,
    CrampedText,
    Script,
    CrampedScript,
    ScriptScript,
    CrampedScriptScript,
}

impl MathStyle {
    /// tex.web 风格码（`cramped=1` 加在未压码上）。
    fn code(self) -> u8 {
        match self {
            MathStyle::Display => 0,
            MathStyle::CrampedDisplay => 1,
            MathStyle::Text => 2,
            MathStyle::CrampedText => 3,
            MathStyle::Script => 4,
            MathStyle::CrampedScript => 5,
            MathStyle::ScriptScript => 6,
            MathStyle::CrampedScriptScript => 7,
        }
    }

    fn from_code(code: u8) -> MathStyle {
        match code {
            0 => MathStyle::Display,
            1 => MathStyle::CrampedDisplay,
            2 => MathStyle::Text,
            3 => MathStyle::CrampedText,
            4 => MathStyle::Script,
            5 => MathStyle::CrampedScript,
            6 => MathStyle::ScriptScript,
            _ => MathStyle::CrampedScriptScript,
        }
    }

    fn is_display(self) -> bool {
        self.code() < 2
    }

    fn is_cramped(self) -> bool {
        self.code() & 1 == 1
    }

    /// 字阶缩放（相对 textfont；TeX 数学三阶字体 text/script/scriptscript，
    /// 10pt 基字阶对应 7pt/5pt——ETRIP 前以比例近似，M4-3 用 fontdimen 精化）。
    fn scale(self) -> (i64, i64) {
        match self.size_kind() {
            0 => (1, 1),
            1 => (7, 10),
            _ => (5, 10),
        }
    }

    /// 上标字段风格（tex.web L13857 `sup_style(#)=2*(# div 4)+4+(# mod 2)`）：
    /// 降一级、压性继承（display→script、script→scriptscript 封顶）。
    fn sup_style(self) -> MathStyle {
        let c = self.code();
        MathStyle::from_code((c / 4) * 2 + 4 + (c % 2))
    }

    /// 下标字段风格（tex.web L13856 `sub_style(#)=2*(# div 4)+5`）：降一级、
    /// 恒压。
    fn sub_style(self) -> MathStyle {
        let c = self.code();
        MathStyle::from_code((c / 4) * 2 + 5)
    }

    /// 分子风格（tex.web L13858 `num_style(#)=#+2-2*(# div 6)`）：降一级、压性
    /// 不变（display→text、text→script），scriptscript 封顶。
    fn numerator_style(self) -> MathStyle {
        let c = self.code();
        MathStyle::from_code(c + 2 - 2 * (c / 6))
    }

    /// 分母风格（tex.web L13859 `denom_style(#)=2*(# div 2)+3-2*(# div 6)`）：
    /// 降一级、恒压。
    fn denominator_style(self) -> MathStyle {
        let c = self.code();
        MathStyle::from_code((c / 2) * 2 + 3 - 2 * (c / 6))
    }

    /// 压性迁移（tex.web L13855 `cramped_style(#)=2*(# div 2)+1`）：同级转压
    /// （\sqrt radicand、\overline/\underline 核）。
    fn cramped(self) -> MathStyle {
        let c = self.code();
        MathStyle::from_code((c / 2) * 2 + 1)
    }

    /// 对应数学字体字阶下标（`math_fonts[fam]` 的 text/script/scriptscript 槽：
    /// tex.web cur_size，风格码 <4（display/text 两态）用 text 槽 0）。
    fn size_kind(self) -> usize {
        let c = self.code();
        if c < 4 {
            0
        } else {
            (c as usize / 2) - 1
        }
    }
}

/// `\sqrt` 原语的默认根号定界符码：plain.tex `\def\sqrt{\radical"270370}`——
/// 按 tex.web scan_delimiter 拆分为 small=(fam 2,'p')=cmsy10 根号、
/// large=(fam 3,'p')=cmex10 根号（27 位码的高半/低半 12 位）。
pub(crate) const SQRT_DELIM_CODE: u32 = 0x27_03_70;

/// 数学字符原子：类 + 族 + 字符码。
///
/// `limits` 只对 **Op 类**原子有意义（tex.web `op_noad` 的 subtype：
/// 0=normal/displaylimits——display 样式堆叠上下限、1=`\limits` 恒堆叠、
/// 2=`\nolimits` 恒不堆叠，上下标走普通脚本位）。`\int`=`\intop\nolimits`
/// 就靠它把上下标放回积分号右侧。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MathChar {
    class: MathClass,
    fam: u8,
    charcode: u32,
    limits: u8,
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
    /// 根式（`\sqrt`/`\radical`）：radicand 子列表 + 27 位定界符码（tex.web
    /// scan_delimiter L22051 拆分：small=(码>>20&15, 码>>12&255)、large=
    /// (码>>8&15, 码&255)；**0 = null delimiter**（var_delimiter null 分支，
    /// 宽 \nulldelimiterspace）。`\sqrt` 原语取 plain.tex 的
    /// `\def\sqrt{\radical"270370}` 码。
    Radical { base: Vec<MathAtom>, delim: u32 },
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
    /// 数学空格（`\mskip`/`\mkern` 结果；`nonscript`：`\nonscript` 后脚本模式丢弃；
    /// `mu`：mu 单位胶——落 hlist 时按当前 style 的 em/18 换算（tex.web `math_glue`，
    /// 只换算 mu_glue；`\hskip` 等 pt 胶原样落 hlist）。
    MSkip {
        width: i64,
        stretch: i64,
        shrink: i64,
        nonscript: bool,
        mu: bool,
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
/// `Clone`（M5 阶段三）：NodeBuilder 整体克隆（段边界检查点）需要。
#[derive(Debug, Default, Clone)]
struct MathLevel {
    atoms: Vec<MathAtom>,
    /// 本组字段类别（`^`/`_`、`\sqrt`、`\mathbin` 后的 `{...}`）；None = 普通组。
    field: Option<MathFieldKind>,
    /// `\left` 打开本层的定界符（None = 普通层）；`\right` 时收为 Delimited 原子。
    left: Option<Option<u32>>,
    /// 本层待定分式（`\over`/`\atop`；numerator 已收集，atoms 继续收 denominator）。
    fraction: Option<FractionPending>,
    /// 待定分式的定界符（`\overwithdelims` 等；`math_fraction` 时并入 FractionPending）。
    frac_delims: (Option<u32>, Option<u32>),
}

/// `\over`/`\atop` 中间态：numerator 已收集，当前数学层 atoms 继续收集 denominator。
/// `Clone`（M5 阶段三）：NodeBuilder 整体克隆（段边界检查点）需要。
#[derive(Debug, Clone)]
struct FractionPending {
    thickness: Option<i64>,
    num: Vec<MathAtom>,
    /// `\overwithdelims` 系定界符（None/None = 无）。
    delims: (Option<u32>, Option<u32>),
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

/// 待封装盒子种类（`\hbox`/`\vbox`/`\vtop`/`\vcenter` 的下一个组）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingBox {
    HBox,
    VBox,
    VTop,
    /// `\vcenter`：收集通路与 [`PendingBox::VBox`] 同构（tex.web
    /// mmode+vcenter 同进内层竖模式），封装后按数学轴重分 height/depth
    /// （tex.web make_vcenter L14455）。
    VCenter,
}

impl PendingBox {
    fn is_vertical(self) -> bool {
        matches!(self, Self::VBox | Self::VTop | Self::VCenter)
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
    /// `\insert<num>{...}`（tex.web group_code=insert_group=11）：组体在内部
    /// 垂直模式排版，`}` 处 vpack(natural) 后挂 ins_node 到外层列表。
    Insert,
}

/// 对齐组方向（tex.web alignment：\halign 行堆叠 / \valign 列并排）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AlignDir {
    Halign,
    Valign,
}

/// M4-5 对齐排版上下文（tex.web alignment 数据；两遍法——数据行以原始
/// 单元列表攒入 `stream`，对齐组结束时统一列宽再封装，tex.web §784-823）。
#[derive(Debug, Clone, PartialEq)]
struct AlignCtx {
    /// 列边界 tabskip 快照（preamble_end 给出；len = 列数 + 1）。
    tabskips: Vec<ntex_core::Glue>,
    /// 顺序流：数据行（\halign）/数据列（\valign）或 \noalign 材料。
    stream: Vec<AlignItem>,
    /// 当前行/列的单元（align_cell_end 攒入，align_row_end 收行）。
    cur_cells: Vec<AlignCellBox>,
    /// 当前行下一单元的起始列（span 累计）。
    cur_col: usize,
    /// `to <dimen>` 目标宽/高（fin_align 摊派）；None = 自然。
    to: Option<i64>,
    /// `spread <dimen>` 增量（S7 修复，2026-09-08）：目标 = 最宽行自然宽 + spread。
    spread: Option<i64>,
}

/// 对齐流项（行/列与 \noalign 材料的顺序记录）。
#[derive(Debug, Clone, PartialEq)]
enum AlignItem {
    /// 数据行/列（单元按序）。
    Row(Vec<AlignCellBox>),
    /// \noalign 材料（垂直列表节点，原样入流）。
    Material(Vec<Node>),
}

/// 一个对齐单元（可跨列）的原始内容（封装延迟到 fin_align）。
#[derive(Debug, Clone, PartialEq)]
struct AlignCellBox {
    /// 起始列。
    start_col: usize,
    /// 跨列数（\span 合并单元）。
    span_len: u16,
    /// 单元原始列表（\halign = 水平；\valign = 垂直）。
    nodes: Vec<Node>,
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
            GroupKind::Insert => 11,
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
            GroupKind::Insert => "insert group",
        }
    }
}

/// 组上下文（group_begin 压栈，group_end 弹出）。
/// `Clone`（M5 阶段三）：NodeBuilder 整体克隆（段边界检查点）需要。
#[derive(Debug, Clone)]
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
    /// 该 `\setbox` 赋值的 `\global` 旗标（tex.web：global 旗标随赋值走
    /// box_context——每个盒子组各自持有。[`BoxState::setbox_global`] 是
    /// 单槽：`\global\setbox0\vbox{...\setbox\z@\hbox{..}...}` 的内层非
    /// global 赋值会覆写它，封装时外层赋值被误当局部（amsmath `\measure@`）。
    setbox_global: bool,
    /// 进入行号（etrip.tex 行；\tracinggroups 的 `entered at line L`）。
    entered_line: u32,
    /// 组打开时是否数学模式（`\hbox{A}` 数学字段关闭是合法流程；外层组
    /// 关闭时仍处数学模式才是缺 `$`）。
    entered_math: bool,
    /// 组深度（1-based；\tracinggroups 的 `(level N)`）。
    level: u32,
    /// 本组的 `to`/`spread` 规格（group_begin 从 [`BoxState::pending_box_spec`]
    /// 认领——tex.web 中规格随盒子扫描各自持有：嵌套时内层 `\hb@xt@\textwidth`
    /// 的规格不得覆写外层 `\vbox to\headheight` 的，否则 LaTeX `\@outputpage`
    /// 页眉盒高度塌成 0、版心整体上移）。
    spec: Option<(Option<i64>, Option<i64>)>,
    /// 本组的参考点位移（`\raise`/`\lower`/`\moveleft`/`\moveright`——tex.web：
    /// 位移前缀绑定紧随其后的盒子；group_begin 认领到盒子组，防止内层嵌套盒
    /// 的封装把位移抢走——`\@outputpage` 的 `\moveright\@themargin\vbox{...}`
    /// 若被头部内层 `\hb@xt@\textwidth` 盒窃取，版心整体左移 62pt）。
    shift: Option<i64>,
    /// `\insert` 组的插入寄存器号（tex.web `saved(0)`；仅 [`GroupKind::Insert`]）。
    insert_class: usize,
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

    /// 更大变体字符（TFM char_info tag=2；tex.web make_op display 大算符
    /// 放大用，cmex10 char 80→88）。fn 指针占位 / 无变体返回 None。
    fn next_larger(&self, font: FontId, charcode: u32) -> Option<u32> {
        match self {
            Fonts::Fn { .. } => None,
            Fonts::Tfm(table) => table
                .borrow()
                .get(font.0 as usize)
                .and_then(|fm| fm.next_larger.get(charcode as usize).copied().flatten())
                .map(|c| c as u32),
        }
    }
}

/// `Params.misc` 下标：`\tracinglostchars`（与 ntex-core `int_param_index` 对齐）。
const MISC_TRACING_LOSTCHARS: usize = 1;

/// R1b 字段分组：NodeBuilder 的 65 个状态字段按域聚成四个嵌入结构体
/// （[`BoxState`]/[`PageState`]/[`MathState`]/[`IoState`]，对齐 R1 的子 trait 域），
/// 其余列表/组/参数/字体/断字等核心态留在本结构体顶层。方法体零逻辑变化，
/// `self.X` → `self.<域>.X` 纯机械替换。
///
/// 增量快照（[`SideEffects`]）保持逐字段平铺镜像：其字段集是本结构体的
/// **子集**（跨组挑字段），不随分组嵌套——`side_effects()`/`restore_side_effects()`
/// 的字段清单契约不变。
///
/// 节点构建 sink：把 VM 排版事件转成节点列表。
///
/// `Clone`（M5 阶段三）：排版层段边界检查点整体克隆 builder 状态
/// （[`crate::typeset::IncrementalTypesetter`]）；`shipped` 页面在捕获时由
/// 捕获方先摘走（页面不随检查点复制，只记数量），故克隆不含已 shipout 页面。
#[derive(Debug, Clone)]
struct NodeBuilder {
    /// 列表栈（栈顶 = 当前列表；栈底 = 主垂直列表）。
    /// 栈内元素按入栈顺序：盒子内容、段落。`[list_modes]` 与之并行，
    /// 记录每个列表的模式（段落 = Horizontal，hbox 内容 = RestrictedHorizontal，
    /// vbox 内容 / 主列表 = Vertical）。作用域组（非盒子）不压列表。
    lists: Vec<Vec<Node>>,

    list_modes: Vec<Mode>,

    /// 组上下文栈（与列表栈独立：作用域组只压 ctx）。
    groups: Vec<GroupCtx>,

    /// 内部参数镜像（随 `param_changed` 事件更新，组作用域快照/恢复）。
    params: Params,

    /// e-TeX 惩罚数组镜像（随 `penalty_array_changed` 事件更新；kind 0-3：
    /// interline/club/widow/displaywidow）。折行时行间惩罚按索引取值，超出用末值。
    penalty_arrays: [Vec<i64>; 4],

    /// `\parshape` 镜像（随 `set_parshape` 推送；`[(indent, width)]`，sp）。
    /// 折行按行号取宽、行盒 `shift` 取缩进（tex.web §16742/§17425）；
    /// LaTeX `\list` 的两侧缩进赖此。组作用域随 `parshape_stack` 恢复。
    parshape: Vec<(i64, i64)>,
    /// 组开始时的 `\parshape` 快照（group_end 恢复；`\parshape` 是组局部量）。
    parshape_stack: Vec<Vec<(i64, i64)>>,

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

    /// M4-5 对齐排版栈（嵌套对齐：\noalign 组或单元内的 \halign/\valign）。
    /// 每项 = (方向, 两遍法上下文；见 [`AlignCtx`])。
    align_stack: Vec<(AlignDir, AlignCtx)>,

    /// 最近一次 \par 的源码行号（折行警告 `at lines a--b` 的结束行）。
    last_par_line: i64,

    /// 字体度量来源（M3-4：fn 指针占位或 TFM 字体表）。
    fonts: Fonts,

    /// `\font<cs>=<name>` 登记的 FontId → cs 名（showbox 字体标识显示
    /// `.\trip 1`；fmt 导入恢复 + `font_defined` 事件更新）。
    font_cs_names: Vec<Option<String>>,

    /// 当前字体（TFM 模式由 `font_selected` 事件更新；fn 指针模式恒为 FontId(0)）。
    current_font: FontId,

    /// CJK 字体回落（workbench 档，宿主经 [`Typesetter::set_fallback_font`]
    /// 下发；默认 None——native/TRIP 语义零影响）：`char_node` 里当前字体
    /// 缺字形且码位 > 0xFF（utf8 输入才可能）时，自动改用该字体排这个字符
    /// （XeTeX/办公排版 per-char fallback 惯例；源文件不写 `\font\zh` 也能排中文）。
    fallback_font: Option<String>,

    /// 回落字体装载缓存：(按当前字体有效字号算出的 at_sp, FontId)。同字号复用，
    /// 字号变则按新字号重载（字体表按名字+缩放去重，\Large 混排只多一两份）。
    fallback_loaded: Option<(i64, FontId)>,

    /// 断字模式表（M4-6）：`\patterns{...}` 解析后的 Liang trie。
    patterns: PatternTrie,

    /// ETRIP 冲刺：断字异常词表（`\hyphenation{...}`）：小写字母 + 允许断点
    /// （0 = 词首、len = 词尾）。断字时优先于模式表。
    hyph_exceptions: Vec<(Vec<u8>, Vec<usize>)>,

    /// M5 阶段三：主列表录制（增量段贡献缓存）。`Some` 时把进入**主列表**
    /// （`lists.len() == 1`）的节点原样录下——编辑段后的增量重放把缓存节点流
    /// 重新注入主列表，行盒免重排、页面装配（断页）照常重跑。
    /// 只追加、不参与任何语义分支；`append`/`push_node` 两个入列路径挂钩。
    record_main: Option<Vec<Node>>,

    /// BoxState 域（见 [`BoxState`]）。
    box_state: BoxState,

    /// PageState 域（见 [`PageState`]）。
    page_state: PageState,

    /// MathState 域（见 [`MathState`]）。
    math_state: MathState,

    /// IoState 域（见 [`IoState`]）。
    io_state: IoState,
}

/// 盒域状态（R1b 分组；对齐 R1 的 [`BoxSink`] 域）：盒封装待定（`\hbox{`、
/// `\raise`/`\moveleft`、`\leaders`、`to`/`spread` 规格）、盒寄存器文件与组级
/// 保存日志、`\setbox` 目标、`\lastbox` 暂存。字段可服务多个域，按主归属划组。
#[derive(Debug, Clone)]
struct BoxState {
    /// 等待下一个组的盒子种类。
    pending_box: Option<PendingBox>,

    /// 等待下一个组的显式种类（`\begingroup`/`\valign`/`\noalign`/`\insert`；
    /// 优先于 pending_box）。
    pending_kind: Option<GroupKind>,

    /// `\insert` 组的插入寄存器号（tex.web `saved(0)`；insert_begin 置、
    /// group_begin 认领进 [`GroupCtx::insert_class`]）。
    pending_insert_class: usize,

    /// `\\raise`/`\\lower`：下一个封装盒子的参考点位移（sp）。
    pending_shift: Option<i64>,

    /// `\\moveleft`/`\\moveright`：下一个封装盒子的水平位移（sp）。
    pending_hshift: Option<i64>,

    /// `\leaders`/`\cleaders`/`\xleaders`：已见引导符、等待盒子。
    pending_leaders: Option<LeadersKind>,

    /// 引导符盒子已就位（`\leaders\hbox{...}` 封装完成 / `\leaders\hrule`）、等待胶水。
    leaders_box: Option<(LeadersKind, Node)>,

    /// 盒子寄存器（M3-5-3）：`\box<n>` 读写（box255 为待输出例程页面队列，见
    /// [`Self::pending_pages`]，不占此表）。
    ///
    /// `Rc`（M5 阶段五）：寄存器文件 32768 槽整份克隆是段级增量热路径的大头
    /// （边界检查点克隆 / 副作用字段对齐每段一次）——`Rc` 共享 + 写时复制
    /// （[`Self::boxes_mut`]）让克隆与比较退化为引用计数 / 指针相等，语义不变
    /// （共享期间无人可写）。
    boxes: std::rc::Rc<Vec<Option<BoxNode>>>,

    /// `\setbox` 组作用域变更日志（TeX 寄存器组级保存）：(组级, 下标, 旧值)。
    /// 组结束回滚本组及更深组内的盒子设置（trip L317 组内 `\setbox22=\lastbox`
    /// → L318 `}` 后参考 `restoring \box22=void`）。
    box_saves: Vec<(usize, usize, Option<BoxNode>)>,

    /// ETRIP 冲刺：`\setbox<n>=<box>` 目标寄存器（下一个封装盒子存入该槽）。
    setbox_target: Option<usize>,

    /// `\setbox` 的 `\global` 前缀（\global\setbox 不随组回滚——tex.web 语义）。
    setbox_global: bool,

    /// ETRIP 冲刺：盒子规格（`\hbox to/spread <dimen>`）：(to, spread)，随下一个盒子组生效。
    pending_box_spec: Option<(Option<i64>, Option<i64>)>,

    /// ETRIP 第二波：`\lastbox` 摘下的盒子（TeX 语义：供下一个 `\box`/`\copy` 使用）。
    lastbox_hold: Option<BoxNode>,
}

/// 页域状态（R1b 分组；对齐 R1 的 [`PageSink`] 域）：页面构建器与自动分页开关、
/// shipout 产出（页面队列与 `\count0..9` 计数快照，两表恒同长）、`\output`
/// 例程事件态、e-TeX marks 族（tex.web 断页轮转）。
#[derive(Debug, Clone)]
struct PageState {
    /// M3-5-2 断页：启用自动分页（`typeset_dvi` 打开；旧 `typeset` 保持切片行为）。
    pagination: bool,

    /// 页面构建器（`pagination` 时把顶层垂直列表拆成页面）。
    page: PageBuilder,

    /// `\shipout`：下一个封装盒子作为页面（DVI shipout，M3-5）。
    shipout_next: bool,

    /// 已 \\shipout 的页面（按顺序）。
    shipped: Vec<BoxNode>,

    /// 各页面 shipout 边界的 `\count0..9` 快照（与 [`Self::shipped`] 一一对应；
    /// 输出例程刀 5：DVI bop 的 10 计数字取值源，由调用方 `write_dvi_with_counts`
    /// 消费）。
    shipped_counts: Vec<[i64; 10]>,

    /// `\count0..9` 镜像（输出例程刀 5 页号链）：tex.web `ship_out` L12694-12699
    /// 在 shipout 边界**直接读 count(j)** 打页标签——引擎经
    /// [`PageSink::count_changed`] 赋值即推送，本侧无从反查寄存器文件。
    page_counts: [i64; 10],

    /// `\\tracingoutput` 转录计数（tex.web ship_out 页号末段：每次 shipout +1；
    /// 页标签自刀 5 起改读 [`Self::page_counts`] 镜像，此计数保留为 shipout 次数
    /// 诊断量并进 M5 副作用快照）。
    ship_seq: u32,

    /// `\output` 例程是否已定义（true：fire_up 改道 box255 + 待执行）。
    output_defined: bool,

    /// 待输出例程处理的页面队列（M3-5-3）：`\output` 定义时 fire_up 产出的页面
    /// 排队，`\box255` 逐页取出，例程反复运行直至队列清空。队列而非单槽——
    /// `close_paragraph` 一次推入多行可能连续产出多页，逐页交错执行例程。
    pending_pages: VecDeque<BoxNode>,

    /// RFC-3：页面真正输出（`\shipout` 边界）时置位，通知引擎 flush 延迟写流。
    write_flush_pending: bool,

    /// 输出例程刀 1：页面真正 shipout 过（自上次 [`PageSink::take_page_shipped`]
    /// 查询以来）——`dead_cycles` 清零依据（tex.web ship_out `dead_cycles:=0`）。
    page_shipped: bool,

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
}

/// 数学域状态（R1b 分组；对齐 R1 的 [`MathSink`] 域）：数学列表栈、当前样式与
/// 待定字段（`^`/`_`、`\sqrt`、`\radical`、类、重音、`\underline`/`\overline`）、
/// 数学字体族、显示数学装配、muskip 参数镜像。
#[derive(Debug, Clone)]
struct MathState {
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

    /// `\eqno`/`\leqno`（tex.web start_eq_no）：Some(leqno) 表示其后数学材料
    /// 是公式编号（独立 mlist）；close_math 按编号侧与公式同线装配。
    eqno_side: Option<bool>,

    /// `\eqno` 事件时切出去的公式原子（tex.web after_math 的公式 mlist p）。
    eqno_formula: Option<Vec<MathAtom>>,

    /// 数学字体族表（M4-3）：16 族 × 3 阶（text/script/scriptscript）。
    /// `\textfont<fam>=<cs>` 等原语分配；字符按族+字阶选字体。
    math_fonts: Vec<[Option<FontId>; 3]>,

    /// M4-4 显示数学：tex.web `pre_display_size`（进入显示时由上一段末行算出：
    /// 2em + 末行可见材料自然宽；空段落 = -max_dimen）。`close_math` 退出时与
    /// `d+s = half(\displaywidth-公式宽)+\displayindent` 比较，裁决长/短 display skip。
    predisplay_size: i64,

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
}

/// I/O 与诊断域状态（R1b 分组；对齐 R1 的 [`IoSink`] 域）：终端转录累积、追加
/// 计数与最近追加节点（layout 侧死循环/OOM 定位共用同一诊断通道）。
#[derive(Debug, Clone)]
struct IoState {
    /// ETRIP 冲刺：终端转录累积（`\message`/`\show`/`\write16`）。
    transcript: String,

    /// 诊断：累计追加节点数 + 最近追加节点（layout 侧死循环/OOM 定位用）。
    nodes_appended: u64,

    last_appended: String,
}

/// 盒子寄存器 255（tex.web `box(255)`）：页面构建器完成页的投递寄存器——
/// tex.web fire_up @<Break the current page at node |p|, put it in box~255...@>
/// 直接 `box(255):=vpackage(link(page_head),best_size,exactly,page_max_depth)`
/// （同层裸写，例程负责消费，例程结束检查 @<Ensure that box 255 is empty after
/// output@>）。NTex 输出例程延迟到 token 边界执行（tex.web 在 fire_up 内同步
/// 执行且 `build_page` 在 output_active 期间停摆，任一时刻至多一页在飞），
/// 一帧内连续断页的多页排队 [`NodeBuilder::pending_pages`]——**队列即寄存器
/// 255 的物理存储，队首即寄存器内容**（tex.web 每次 fire_up 覆写 box(255)
/// ≙ NTex 每页 push_back，后页排在队首之后）。全部寄存器访问路径（读/取/存/
/// 判型/维度/拆分/showbox）经 [`NodeBuilder::box_view`]/[`NodeBuilder::take_box_at`]
/// /[`NodeBuilder::write_box`] 统一路由，255 与普通寄存器同语义——latex.ltx
/// `\@cclv=\chardef 255`（L325）正是普通寄存器号：`\box`×3、`\unvbox`×2、
/// `\vsplit to\z@`×1、`\setbox`×1（L20914 存回）+ `\ifvoid/\ifvbox`。
const PAGE_BOX: usize = 255;

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
            && (self.math_state.class_pending.is_some()
                || self.math_state.accent_pending
                || self.math_state.radical_pending.is_some()
                || self.math_state.sqrt_pending
                || self.math_state.pending_script.is_some())
        {
            self.report_error("Missing { inserted.");
            // scan_left_brace 隐含 `{`：TeX 报错后 cur_tok={ 开 math_group
            // （隐含空字段组）——**必须真的开组**，否则后续 `}` 越界关外层组
            // → Missing $ + close_math（TRIP l.272 `\mathord\radical"161` 缺 {，
            //    l.278 的 }}} 第 3 个 `}` 靠它配对；l.280 eqno 数学丢根因）。
            // 待定字段清空后开隐含组（Math 组：push math 层，由后续 `}` pop）。
            self.math_state.class_pending = None;
            self.math_state.accent_pending = false;
            self.math_state.radical_pending = None;
            self.math_state.sqrt_pending = false;
            self.math_state.pending_script = None;
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
            params: Params::default(),
            penalty_arrays: Default::default(),
            parshape: Vec::new(),
            parshape_stack: Vec::new(),
            param_stack: Vec::new(),
            sfcodes,
            font_stack: Vec::new(),
            space_factor: 1000,
            noindent_next: false,
            font_cs_names: Vec::new(),
            align_stack: Vec::new(),
            last_par_line: 0,
            current_font: FontId(0),
            fallback_font: None,
            fallback_loaded: None,
            fonts,
            patterns: PatternTrie::default(),
            hyph_exceptions: Vec::new(),
            record_main: None,
            box_state: BoxState {
                pending_box: None,
                pending_kind: None,
                pending_insert_class: 0,
                pending_shift: None,
                pending_hshift: None,
                pending_leaders: None,
                leaders_box: None,
                boxes: std::rc::Rc::new(vec![None; REGISTER_COUNT]),
                box_saves: Vec::new(),
                setbox_target: None,
                setbox_global: false,
                pending_box_spec: None,
                lastbox_hold: None,
            },
            page_state: PageState {
                pagination,
                page: PageBuilder::new(),
                shipout_next: false,
                shipped: Vec::new(),
                shipped_counts: Vec::new(),
                page_counts: [0; 10],
                ship_seq: 0,
                output_defined: false,
                pending_pages: VecDeque::new(),
                write_flush_pending: false,
                page_shipped: false,
                marks_top: std::collections::HashMap::new(),
                marks_first: std::collections::HashMap::new(),
                marks_bot: std::collections::HashMap::new(),
                marks_split_top: std::collections::HashMap::new(),
                marks_split_first: std::collections::HashMap::new(),
                marks_split_bot: std::collections::HashMap::new(),
            },
            math_state: MathState {
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
                predisplay_size: 0,
                after_display: false,
                eqno_side: None,
                eqno_formula: None,
                // 默认数学间距（TeXbook p.170）：thin=3mu、med=4mu±2mu∓4mu、thick=5mu±5mu
                // ——按 mu 数值存（1mu=1pt 数值=N×65536）；math_to_hlist 内部按当前 style em/18 转 sp。
                muskip_params: [
                    ntex_core::Glue::new(3 * SP_PER_PT, 0, 0),
                    ntex_core::Glue::new(4 * SP_PER_PT, 2 * SP_PER_PT, 4 * SP_PER_PT),
                    ntex_core::Glue::new(5 * SP_PER_PT, 5 * SP_PER_PT, 0),
                ],
                muskip_is_mu: [true; 3],
            },
            io_state: IoState {
                transcript: String::new(),
                nodes_appended: 0,
                last_appended: String::new(),
            },
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
        self.page_state.marks_top = self.page_state.marks_bot.clone();
        // marks_first：新页第一个 marks 清空
        self.page_state.marks_first.clear();
        // marks_bot：初始等于继承的 marks_top（若无新 marks 则 bot==top）
        self.page_state.marks_bot = self.page_state.marks_top.clone();
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

    fn append(&mut self, node: Node) {
        // 诊断：节点增长监控（TRIP L338 `\halign` 内挂死 = 列表无限增长 OOM；
        // 死循环在 layout 侧不经 process_one，VM 看门狗不计数）。
        self.io_state.nodes_appended += 1;
        self.io_state.last_appended = format!("{node:?}");
        if self.io_state.nodes_appended % 200_000 == 0 {
            eprintln!(
                "[layout-watchdog] nodes={} mode={:?} lists={} top_len={} groups={} last={:?}",
                self.io_state.nodes_appended,
                self.mode(),
                self.lists.len(),
                self.lists.last().map_or(0, |l| l.len()),
                self.groups.len(),
                self.io_state.last_appended
            );
        }
        // tex.web box_end（L20894 `shift_amount(cur_box):=box_context`）：
        // \raise/\lower/\moveleft/\moveright 的位移值对**下一个进入列表的盒子**
        // 生效（\box/\copy/\hbox 等所有 make_box 路径）。参照 append_to_vlist/
        // hlist_out 的语义：shift 沿盒子进入当前列表时写入，非包装完成时。
        let mut node = node;
        if let Node::Box(b) = &mut node {
            if let Some(v) = self.box_state.pending_shift.take() {
                b.shift = v;
            }
            // 水平位移（\moveleft/\moveright）：tex.web 同一 box_context 机制，
            // hlist 中 vbox 的 shift 也可表示水平偏移（vlist_out L12590
            // `cur_h:=left_edge+shift_amount(p)`）——同一字段按列表方向解释。
            if let Some(v) = self.box_state.pending_hshift.take() {
                b.shift = v;
            }
        }
        self.record_main_node(&node);
        self.lists.last_mut().expect("列表栈非空").push(node);
        // M3-5-2：顶层垂直模式追加后运行页面构建器（TeX build_page 的触发点）。
        // 增量（feed_one）：每产出一页即暂停——若定义了输出例程，让引擎在 token
        // 边界执行例程（ship box255）后再继续；未定义时页面直通 shipped。
        if self.page_state.pagination && self.mode() == Mode::Vertical && self.lists.len() == 1 {
            if let Some(p) = self
                .page_state
                .page
                .feed_one(&mut self.lists[0], &self.params)
            {
                self.accept_page(p);
                // ETRIP 冲刺：断页 marks 轮转（top = 旧 bot，first 清空，bot 保留继承）
                self.rotate_marks();
            }
            // \tracingpages：断页追踪输出到转录（tex.web begin_diagnostic → log）
            if let Some(t) = self.page_state.page.take_trace() {
                let _ = self.write16(t);
            }
        }
    }

    /// u8（0-7）→ 数学类别（TeXbook 附录 B \mathcode 类编号：
    /// 0=Ord 1=Op 2=Bin 3=Rel 4=Open 5=Close 6=Punct 7=Var。
    /// plain.tex 铁证：\sum="1350 → class 1=Op（大算符）、`+`="202B →
    /// class 2=Bin。此前 1/2 互换（\sum 误 Bin、+ 误 Op）、7 误 Inner
    /// （Inner 由原子类型 Fraction/Delimited/`\mathinner` 给出，非 mathcode
    /// 编码；Var 间距按 Ord 查 Rule 18 表）。
    fn class_of(class: u8) -> MathClass {
        match class {
            1 => MathClass::Op,
            2 => MathClass::Bin,
            3 => MathClass::Rel,
            4 => MathClass::Open,
            5 => MathClass::Close,
            6 => MathClass::Punct,
            7 => MathClass::Var,
            _ => MathClass::Ord,
        }
    }

    /// `\mathord`=0/`\mathbin`=1/`\mathop`=2/`\mathrel`=3/`\mathopen`=4/
    /// `\mathclose`=5/`\mathpunct`=6/`\mathinner`=7（tex.web math_comp 命令
    /// 编号）→ 数学类别。与 [class_of]（mathcode 类字段编号）是**不同体系**：
    /// Bin/Op 编号互换（mathcode 1=Op/2=Bin），且命令 7=Inner 而 mathcode 7=Var。
    /// 混用会让 `\mathbin` 得 Op 类（间距误 thinmuskip）。
    fn class_of_cmd(n: u8) -> MathClass {
        match n {
            1 => MathClass::Bin,
            2 => MathClass::Op,
            7 => MathClass::Inner,
            _ => Self::class_of(n),
        }
    }

    /// 盒子寄存器可写视图（写时复制：`Rc` 共享时先拆出独享副本——段级增量
    /// 的边界检查点/副作用对齐克隆与活 builder 共享同一份寄存器文件，写入
    /// 前必须解共享，语义与整份深克隆一致）。
    fn boxes_mut(&mut self) -> &mut Vec<Option<BoxNode>> {
        std::rc::Rc::make_mut(&mut self.box_state.boxes)
    }

    /// 读盒子寄存器（255 → 待输出页队首，见 [PAGE_BOX]）。
    fn box_view(&self, idx: usize) -> Option<&BoxNode> {
        if idx == PAGE_BOX {
            self.page_state.pending_pages.front()
        } else {
            self.box_state.boxes.get(idx).and_then(|s| s.as_ref())
        }
    }

    /// 取走盒子寄存器内容（`\box`/`\unhbox`/`\unvbox`/`\vsplit` 的取走语义；
    /// tex.web 对应 `box(cur_val):=null`（begin_box）/`box(n):=null`（unpackage）
    /// /`box(n):=vpack(...)`（vsplit）的**同层裸写**——不入 save stack，组结束
    /// 不回滚。255 → 弹队首：页被例程消费后不再复活（若入组级日志，例程组的
    /// 回滚会把已消费页塞回队列导致重复输出）。
    fn take_box_at(&mut self, idx: usize) -> Option<BoxNode> {
        if idx == PAGE_BOX {
            self.page_state.pending_pages.pop_front()
        } else {
            self.boxes_mut().get_mut(idx).and_then(|s| s.take())
        }
    }

    /// 写盒子寄存器（`\setbox` 的写半边与组结束回滚共用）。255 → 替换队首，
    /// 队列空时压入成为待输出页（tex.web：例程 `\setbox\@cclv\vbox{\box\@cclv\vfil}`
    /// L20914 存回 255，页即被改写；`None` = 清空寄存器）。
    fn write_box(&mut self, idx: usize, value: Option<BoxNode>) {
        if idx == PAGE_BOX {
            match value {
                Some(b) if !self.page_state.pending_pages.is_empty() => {
                    self.page_state.pending_pages[0] = b
                }
                Some(b) => self.page_state.pending_pages.push_front(b),
                None => {
                    self.page_state.pending_pages.pop_front();
                }
            }
        } else if let Some(slot) = self.boxes_mut().get_mut(idx) {
            *slot = value;
        }
    }

    /// 排版副作用字段快照（M5 阶段五，`typeset::incremental` 的复用判定用）：
    /// 只拷贝随执行演化、不进节点流/页面产出的小字段（盒子寄存器文件 `Rc`
    /// 共享，克隆是引用计数）——整份 [`Self::clone`] 含页面构建器的当前页节点，
    /// 只留给段边界回滚点。
    ///
    /// **字段清单与 [`Self::restore_side_effects`] 必须一致，新增字段两处同步。**
    fn side_effects(&self) -> SideEffects {
        SideEffects {
            pending_box: self.box_state.pending_box,
            pending_kind: self.box_state.pending_kind,
            pending_shift: self.box_state.pending_shift,
            pending_hshift: self.box_state.pending_hshift,
            pending_leaders: self.box_state.pending_leaders,
            leaders_box: self.box_state.leaders_box.clone(),
            params: self.params,
            penalty_arrays: self.penalty_arrays.clone(),
            param_stack: self.param_stack.clone(),
            sfcodes: self.sfcodes,
            font_stack: self.font_stack.clone(),
            space_factor: self.space_factor,
            noindent_next: self.noindent_next,
            align_stack: self.align_stack.clone(),
            last_par_line: self.last_par_line,
            font_cs_names: self.font_cs_names.clone(),
            current_font: self.current_font,
            shipout_next: self.page_state.shipout_next,
            ship_seq: self.page_state.ship_seq,
            page_counts: self.page_state.page_counts,
            boxes: BoxFile(std::rc::Rc::clone(&self.box_state.boxes)),
            box_saves: self.box_state.box_saves.clone(),
            output_defined: self.page_state.output_defined,
            pending_pages: self.page_state.pending_pages.clone(),
            write_flush_pending: self.page_state.write_flush_pending,
            page_shipped: self.page_state.page_shipped,
            math_style: self.math_state.math_style,
            pending_script: self.math_state.pending_script,
            sqrt_pending: self.math_state.sqrt_pending,
            radical_pending: self.math_state.radical_pending,
            class_pending: self.math_state.class_pending,
            accent_pending: self.math_state.accent_pending,
            underline_pending: self.math_state.underline_pending,
            overline_pending: self.math_state.overline_pending,
            nonscript_pending: self.math_state.nonscript_pending,
            math_fonts: self.math_state.math_fonts.clone(),
            patterns: self.patterns.clone(),
            hyph_exceptions: self.hyph_exceptions.clone(),
            setbox_target: self.box_state.setbox_target,
            setbox_global: self.box_state.setbox_global,
            pending_box_spec: self.box_state.pending_box_spec,
            predisplay_size: self.math_state.predisplay_size,
            after_display: self.math_state.after_display,
            muskip_params: self.math_state.muskip_params,
            muskip_is_mu: self.math_state.muskip_is_mu,
            marks_top: self.page_state.marks_top.clone(),
            marks_first: self.page_state.marks_first.clone(),
            marks_bot: self.page_state.marks_bot.clone(),
            marks_split_top: self.page_state.marks_split_top.clone(),
            marks_split_first: self.page_state.marks_split_first.clone(),
            marks_split_bot: self.page_state.marks_split_bot.clone(),
            lastbox_hold: self.box_state.lastbox_hold.clone(),
        }
    }

    /// 排版副作用字段推进（缓存段复用时）：该段未执行，其排版副作用（盒子寄存器、
    /// marks、参数镜像、当前字体、断字表等）不会发生——对齐到快照里的"执行后"
    /// 样子；主列表 / 页面构建器 / shipped 保留注入重算的结果（页面断点随编辑
    /// 后移，这正是增量要的效果）。
    ///
    /// **字段清单与 [`Self::side_effects`] 必须一致，新增字段两处同步。**
    fn restore_side_effects(&mut self, s: &SideEffects) {
        self.box_state.pending_box = s.pending_box;
        self.box_state.pending_kind = s.pending_kind;
        self.box_state.pending_shift = s.pending_shift;
        self.box_state.pending_hshift = s.pending_hshift;
        self.box_state.pending_leaders = s.pending_leaders;
        self.box_state.leaders_box = s.leaders_box.clone();
        self.params = s.params;
        self.penalty_arrays = s.penalty_arrays.clone();
        self.param_stack = s.param_stack.clone();
        self.sfcodes = s.sfcodes;
        self.font_stack = s.font_stack.clone();
        self.space_factor = s.space_factor;
        self.noindent_next = s.noindent_next;
        self.align_stack = s.align_stack.clone();
        self.last_par_line = s.last_par_line;
        self.font_cs_names = s.font_cs_names.clone();
        self.current_font = s.current_font;
        self.page_state.shipout_next = s.shipout_next;
        self.page_state.ship_seq = s.ship_seq;
        self.page_state.page_counts = s.page_counts;
        self.box_state.boxes = std::rc::Rc::clone(&s.boxes.0);
        self.box_state.box_saves = s.box_saves.clone();
        self.page_state.output_defined = s.output_defined;
        self.page_state.pending_pages = s.pending_pages.clone();
        self.page_state.write_flush_pending = s.write_flush_pending;
        self.page_state.page_shipped = s.page_shipped;
        self.math_state.math_style = s.math_style;
        self.math_state.pending_script = s.pending_script;
        self.math_state.sqrt_pending = s.sqrt_pending;
        self.math_state.radical_pending = s.radical_pending;
        self.math_state.class_pending = s.class_pending;
        self.math_state.accent_pending = s.accent_pending;
        self.math_state.underline_pending = s.underline_pending;
        self.math_state.overline_pending = s.overline_pending;
        self.math_state.nonscript_pending = s.nonscript_pending;
        self.math_state.math_fonts = s.math_fonts.clone();
        self.patterns = s.patterns.clone();
        self.hyph_exceptions = s.hyph_exceptions.clone();
        self.box_state.setbox_target = s.setbox_target;
        self.box_state.setbox_global = s.setbox_global;
        self.box_state.pending_box_spec = s.pending_box_spec;
        self.math_state.predisplay_size = s.predisplay_size;
        self.math_state.after_display = s.after_display;
        self.math_state.muskip_params = s.muskip_params;
        self.math_state.muskip_is_mu = s.muskip_is_mu;
        self.page_state.marks_top = s.marks_top.clone();
        self.page_state.marks_first = s.marks_first.clone();
        self.page_state.marks_bot = s.marks_bot.clone();
        self.page_state.marks_split_top = s.marks_split_top.clone();
        self.page_state.marks_split_first = s.marks_split_first.clone();
        self.page_state.marks_split_bot = s.marks_split_bot.clone();
        self.box_state.lastbox_hold = s.lastbox_hold.clone();
    }

    /// 存入盒子寄存器（TeX 寄存器组级保存）：组内记录旧值，组结束回滚。
    /// 返回旧值（`\box` 取走语义：读旧值 + 清空由调用方按返回值使用）。
    fn store_box(&mut self, idx: usize, value: Option<BoxNode>) -> Option<BoxNode> {
        let old = self.box_view(idx).cloned();
        if !self.groups.is_empty() && !self.box_state.setbox_global {
            self.box_state
                .box_saves
                .push((self.groups.len(), idx, old.clone()));
        }
        self.write_box(idx, value);
        old
    }

    #[allow(clippy::type_complexity)]
    fn package_box(
        &mut self,
        kind: PendingBox,
        ship: bool,
        leaders: Option<LeadersKind>,
        // `\setbox` 认领对：目标寄存器 + 本赋值的 `\global` 旗标（tex.web
        // box_context：旗标随赋值走，二者一体认领）。
        setbox: Option<(usize, bool)>,
        shift: Option<i64>,
        boxmaxdepth: i64,
    ) {
        let children = self.lists.pop().expect("盒子列表");
        self.list_modes.pop();
        // ETRIP 冲刺：\hbox/\vbox to/spread 规格（目标宽/高）
        let spec = self.box_state.pending_box_spec.take();
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
            PendingBox::VCenter => {
                let natural = vbox_dimensions(&children);
                let target = match spec {
                    Some((Some(to), _)) => to,
                    Some((_, Some(spread))) => natural.height + natural.depth + spread,
                    _ => natural.height + natural.depth,
                };
                let mut b = vpack(children, target, boxmaxdepth);
                // tex.web make_vcenter（L14455-14463）：delta=h+d；
                // height:=axis_height(cur_size)+half(delta)；depth:=delta−height
                // （可负；外层 hpack 的 max 从 0 起算故不致下探）。轴高取
                // **封装现场**的数学样式（fam 2 当前字阶 fontdimen 22）；
                // 非数学模式（TeX 本应报错）退化为 vbox 不动。
                if matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
                    let delta = b.height + b.depth;
                    let axis = self.axis_height(self.math_state.math_style);
                    // tex.web half（L2168）= ceil 除法：算术右移 + 奇数进位
                    b.height = axis + ((delta >> 1) + (delta & 1));
                    b.depth = delta - b.height;
                }
                Node::Box(b)
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
        // 参考点位移（group_begin 认领的 `\raise`/`\lower`/`\moveleft`/
        // `\moveright`）：同一 shift_amount 字段（tex.web：横列表=竖位移、
        // 竖列表=水平位移，由使用现场解释）——LaTeX `\@outputpage` 的
        // `\moveright\@themargin\vbox{...}`（版心定位 62pt）依赖此臂。
        let mut node = node;
        if let Some(shift) = shift {
            if let Node::Box(b) = &mut node {
                b.shift = shift;
            }
        }
        // ETRIP 冲刺：`\setbox<n>=<box>` —— 封装结果存入寄存器（不入当前列表）。
        // 仅最外层 RHS 盒子组（group_begin 认领进 GroupCtx）持有目标；内层嵌套盒
        // （`\setbox0=\vbox{\hbox{...}}` 的 \hbox）不消费。
        if let Some((idx, setbox_global)) = setbox {
            // 本赋值的 \global 旗标随组认领（见 GroupCtx::setbox_global）——
            // 封装时写回单槽，防止嵌套赋值（amsmath `\measure@` 内层
            // `\setboxz@h`）覆写后外层赋值按错误作用域入寄存器。
            self.box_state.setbox_global = setbox_global;
            if let Node::Box(b) = node {
                self.store_box(idx, Some(b));
            }
            return;
        }
        if ship {
            if let Node::Box(b) = node {
                self.trace_shipout(&b);
                self.push_shipped(b);
                self.page_state.write_flush_pending = true;
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
            self.box_state.leaders_box = Some((ld, node));
            return;
        }
        self.push_box(node);
    }

    /// 追加盒子到当前列表；垂直列表中前驱为盒子时插入 interline glue
    /// （tex.web `append_to_vlist`：d = \baselineskip − (depth 前 + height 新)，
    /// d < \lineskiplimit 用 \lineskip，否则用宽度调整为 d 的 \baselineskip）。
    fn push_node(&mut self, node: Node) {
        self.io_state.nodes_appended += 1;
        self.io_state.last_appended = format!("{node:?}");
        self.record_main_node(&node);
        self.lists.last_mut().expect("列表栈非空").push(node);
    }

    /// M5 阶段三录制钩子（[`Self::record_main`]）：只读/追加型，无语义分支。
    /// `push_node`（close_paragraph 的行间惩罚）不经过 `append`，两处都挂。
    fn record_main_node(&mut self, node: &Node) {
        if self.lists.len() == 1 {
            if let Some(rec) = &mut self.record_main {
                rec.push(node.clone());
            }
        }
    }

    /// 垂直列表中不阻断行间胶水的节点。显式 glue/kern 会重置 TeX 的
    /// `prev_depth` 关系；否则 section afterskip、float textfloatsep 等显式
    /// 间距后还会再补一段 baselineskip。
    fn push_box_transparent(n: &Node) -> bool {
        matches!(
            n,
            Node::Penalty { .. }
                | Node::Mark { .. }
                | Node::Ins { .. }
                | Node::Adjust { .. }
                | Node::Whatsit { .. }
        )
    }

    fn previous_interline_depth(&self, cross_glue: bool) -> Option<i64> {
        let list = self.lists.last()?;
        let mut saw_blocking = false;
        for n in list.iter().rev() {
            match n {
                Node::Box(prev) => return Some(prev.depth),
                Node::Rule { depth, .. } => return Some(*depth),
                _ if cross_glue && !matches!(n, Node::Box(_) | Node::Rule { .. }) => {}
                _ if Self::push_box_transparent(n) => {}
                _ => {
                    saw_blocking = true;
                    break;
                }
            }
        }
        if !saw_blocking
            && self.page_state.pagination
            && self.lists.len() == 1
            && self.page_state.page.prev_depth() > crate::page::IGNORE_DEPTH
        {
            Some(self.page_state.page.prev_depth())
        } else {
            None
        }
    }

    fn push_box(&mut self, node: Node) {
        self.push_box_inner(node, false);
    }

    fn push_box_crossing_glue(&mut self, node: Node) {
        self.push_box_inner(node, true);
    }

    fn push_box_inner(&mut self, node: Node, cross_glue: bool) {
        if self.mode() == Mode::Vertical {
            let display_followup = self.math_state.after_display;
            if let Some(prev_depth) = self.previous_interline_depth(cross_glue || display_followup)
            {
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
            if display_followup {
                self.math_state.after_display = false;
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
    /// CJK 回落例外：码位 > 0xFF 且宿主注册了回落字体时自动改用回落字体
    /// （见下方判据处说明），不发警告、照建节点。
    fn char_node(&mut self, tok: Token) -> Option<Node> {
        let charcode = tok.charcode()?;
        if !self.fonts.char_exists(self.current_font, charcode) {
            // CJK 字体回落（utf8 档）：码位 > 0xFF 只可能来自 utf8 输入
            // （8-bit/TRIP 路径字符上限 255，语义零影响）；当前 8-bit TFM
            // 字体没有该字形、宿主又注册了回落字体（如 FandolSong）→ 该
            // 字符改用回落字体排。源文件不写 `\font\zh=FandolSong-Regular`
            // 也能排中文——Tauri 工作台「plain 简历中文全 Missing character」
            // 现场的修复（2026-09-18）。回落字体也没有该字形 → 走原警告路径。
            if charcode > 0xFF {
                if let Some(fb) = self.ensure_fallback_font() {
                    if self.fonts.char_exists(fb, charcode) {
                        let (w, h, d) = self.fonts.metrics(fb, charcode);
                        return Some(Node::Char {
                            font: fb,
                            charcode,
                            width: w,
                            height: h,
                            depth: d,
                        });
                    }
                }
            }
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

    /// 取回落字体的 FontId（懒装载 + 字号缓存）：按**当前字体的有效字号**
    /// 装载（`at` = design_size×scale/2^20），`\Large` 上下文里的中文随西文
    /// 一起放大。仅 TFM 表模式可用（fn 指针占位模式无表可挂）；装载失败
    /// （回落字体名无效/字节坏）返回 None，char_node 走原 Missing 路径。
    fn ensure_fallback_font(&mut self) -> Option<FontId> {
        let name = self.fallback_font.clone()?;
        let Fonts::Tfm(table) = &self.fonts else {
            return None;
        };
        // 当前字体有效字号（sp）：design_size_sp × scale（2^20 定点）÷ 2^20。
        // 当前字体未装载（悬空/字号 0）→ 按 10pt 设计字号装载。
        let at_sp = table
            .borrow()
            .get(self.current_font.0 as usize)
            .map(|fm| (fm.design_size_sp as i128 * fm.scale as i128 / (1 << 20)) as i64)
            .filter(|&at| at > 0)
            .unwrap_or(10 * SP_PER_PT);
        if let Some((cached_at, id)) = self.fallback_loaded {
            if cached_at == at_sp {
                return Some(id);
            }
        }
        let id = load_font_into_table(table, &name, Some(at_sp), None).ok()?;
        self.fallback_loaded = Some((at_sp, FontId(id)));
        Some(FontId(id))
    }

    /// [`Typesetter::set_fallback_font`] 的 builder 侧落点（install_builder 同步）。
    pub(crate) fn set_fallback_font_name(&mut self, name: Option<String>) {
        self.fallback_font = name;
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
        self.box_state.leaders_box = None;
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

    /// `\␣` 的词间空白（tex.web `append_normal_space`，L20332）：`\spaceskip`
    /// 非 zero_glue → 参数胶水；否则当前字体 font_glue 原样。
    /// 与 [`Self::append_space_glue`] 的关键差异：**无 spacefactor 分支**——
    /// `hmode+ex_space` 是 `goto append_normal_space`（L20054），绕过
    /// `app_space` 的 sf 调整，`\xspaceskip` 也不参与（GT：`\sfcode`A=2000
    /// 后 `A\ B` 仍是 `glue 3.33333 plus 1.66666 minus 1.11111`，而 `A B`
    /// 是 `plus 1.66498 minus 1.11221`）。
    fn append_normal_space(&mut self) {
        let (w, s, k, name) = self.normal_space_glue();
        self.append(Node::Glue {
            name,
            width: w,
            stretch: s,
            shrink: k,
            stretch_order: 0,
            shrink_order: 0,
        });
    }

    /// [`Self::append_normal_space`] 的胶水值 + showbox 来源名：水平列表直落
    /// [`Node::Glue`]（tex.web `new_param_glue` 带 `\spaceskip` 名），数学表包成
    /// `MSkip` 原子（无名字段，落 hlist 时同为 pt 胶水）。
    fn normal_space_glue(&self) -> (i64, i64, i64, Option<&'static str>) {
        let sk = self.params.spaceskip;
        if sk != ntex_core::Glue::ZERO {
            (sk.width, sk.stretch, sk.shrink, Some("spaceskip"))
        } else {
            let g = self.fonts.space(self.current_font);
            (g.width, g.stretch, g.shrink, None)
        }
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

// 格式预载 G2(a)：内嵌 plain.tex/hyphen.tex 资源 + `\input` 兜底 VFS 层
pub mod plain_format;
pub use plain_format::{EmbeddedFormatVfs, HYPHEN_TEX, PLAIN_TEX};

// M8-A WASM 骨架线：TFM 字节源缝（宿主注入；默认空 → native 走文件系统不变）
include!("wasm_fonts.rs");

// 方法分片（include! 嵌入，原 impl 按域拆分）
include!("paragraph.rs");
include!("paging.rs");
include!("math.rs");
include!("sink.rs");

// \showbox 格式化（自由函数，迁自 sink.rs；依赖 mod.rs 已 use 的类型）
include!("sink_showbox.rs");

// M5 阶段三：端到端增量排版（段贡献缓存 + 页面装配重跑；layout 层段边界检查点）
include!("incremental.rs");

// LaTeX 源特征检测（格式自动检测；ntex-dvi / ntex-studio 共用同一实现）
include!("latex_detect.rs");

include!("incremental_tests.rs");

include!("tests.rs");
