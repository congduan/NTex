# 已知简化实现与技术债清单（KNOWN SIMPLIFICATIONS）

> 维护规则：**新增任何"简化/no-op/暂不"实现时，必须在此登记**（文件:行）。
> 修复某项后，从清单移除并注明 commit。本清单是"踩坑前先查表"的索引，
> 防止同一领域反复探路（如数学状态机、对齐组）。

## 1. 数学原语（存在性测试/消费参数——最高风险区）

| 位置 | 现状 | 影响 | 状态 |
|---|---|---|---|
| `expand/builtins.rs:296` | 数学原语批量"存在性测试" | 参数被吃掉，数学列表节点缺失 | ⚠️ 部分已修（mkern/mskip mu 上下文 3a2cb63；fraction 族 721646c；其余逐项排查） |
| `typeset/math.rs:131` | math_char 字符 fam 恒 0（\mathcode/\fam 未解析） | \scriptfont undefined 检查不可行（直接检查 127 次爆炸，已回退） | ✅ 经验证主体非债 + \fam 臂已补（2026-09-23）：\mathcode 表早已接通（expand/mod.rs 数学字符分派查 mathcodes → `sink.math_char_full`，layout 侧拆 class/fam/char；DVI 字体名级对照 NT≡GT：`$a+1x\alpha$` 两引擎只落 cmmi10+cmr10 同字形同字体）。实缺口是 tex.web scan_math L21906 的 **\fam 无条件替换臂**：class≥var_code(0x7000) 且 cur_fam∈0..15 → fam:=cur_fam（plain 的 \rm/\it 靠它改道字族）；已在 expand/mod.rs 数学字符分派 + `\mathchar`（expand/primitive_math.rs，tex.web math_char_num 同流到尾部）接入 `mathcode_apply_fam`（misc[46]），并按 init_math L21721 在进公式时复位 \fam=-1。GT 对照（DVI 字体名级）：`$\rm x\it y$` → x=cmr10/y=cmti10 与 pdftex 一致（修复前两者都落 cmmi10）；`$\fam0 x$ $y$` 次公式 x 回落 fam1（复位生效）。回归测试 `math_fam_substitution_replaces_var_class_family`（ntex-layout）。 |
| `expand/primitive.rs:753` | 数学原语"简化实现消费参数" | 同上 | 逐项排查中 |
| `eqtb/primitive.rs:559` | 同 753 | 同上 | 同上 |
| `expand/primitive.rs:780-800` | **fraction 原语族（\abovewithdelims/\above/\atopwithdelims/\overwithdelims）此前只扫参数不挂 sink.math_fraction** | 分子分母混收当前层，trip l.276 数学状态崩 | ✅ 已修（721646c；同层嵌套歧义改恢复式，参考 l.257 Ambiguous） |
| `expand/primitive.rs:726` / `eqtb/primitive.rs:382` | `\vcenter` 简化按 vbox | d 组待做（收集不执行） | ✅ 已修（按数学轴居中：expander 保留 `Primitive::VCenter` 身份（expand/primitive_math.rs VCenter 臂），排版层 `PendingBox::VCenter` 复用 vbox 组收集（tex.web mmode+vcenter L22112 同进内层竖模式），封装时 make_vcenter（tex.web L14455）重分 ht/dp：`height=axis+half(h+d)`，轴高=现场数学样式 fam2 fontdimen 22（plain= cmsy10 param22=2.5pt；cmex10 仅 13 参数无此项）；非数学模式退化为 vbox 不动。GT 对照（etex DVI rule-y 差分法）：`\vcenter{\hbox{\vrule h20pt d4pt}}` → 14.5+9.5 一致；`\vbox` 对照 20+4 不动；`\vcenter{\hbox{$x$}}` 内字上移 0.347pt 一致。回归测试 `vcenter_splits_height_depth_on_math_axis`/`vcenter_axis_fallback_splits_at_center`） |
| `expand/primitive_math.rs:188` | `\eqno/\leqno` 显示数学内 no-op；非数学报错+pretend 已补（942346e） | 显示公式编号不落节点 | ✅ 已修（2026-09-23）：tex.web start_eq_no/after_math/finish_display 装配接入——core 侧显示数学（mode 6）`\eqno/\leqno` 发 `sink.math_eqno(leqno)`（expand/primitive_math.rs EqNo/LeqNo 臂；行内 mode 3 沿旧径 no-op，tex.web 应报 illegal，收窄风险后补）；layout 侧 math_eqno 事件把当前层公式原子切去 `MathState::eqno_formula`、后续材料当编号 mlist，`close_math`（typeset/math.rs）按 tex.web L22521-22632 装配：e=编号盒自然宽，d=half(z−w) 且 d<2e 时左移 half(z−w−e)，行=公式+kern(z−w−e−d)+编号盒（leqno 反序且 d:=0），hpack(natural) 后 shift=s+d（编号右缘=displaywidth 右缘/左缘=displayindent），长/短 skip 裁决补 `or l` 臂；不做 Squeeze（挤窄）臂。GT 对照（pdftex）：`$$x+y\eqno(1)$$` 编号右缘落版心右缘、公式照 half(z−w) 居中，`$$z\leqno(2)$$` 编号落左缘，两侧一致。回归测试 `math_eqno_number_right_flushed_at_displaywidth`（ntex-layout，含 leqno 反序断言） |
| `typeset/sink.rs:1082` | 数学模式 `\penalty` 忽略 | 数学断行点缺失（M4-1） | ✅ 已修（sink penalty 数学分支转 `MathAtom::Penalty` → `Node::Penalty`，tests_math `math_penalty_kept_in_formula`/`math_penalty_inline_between_chars`） |
| `typeset/sink.rs:1097` | 数学模式 `\vrule` 忽略 | 规则原子缺失 | ✅ 已修（sink rule 数学分支转 `MathAtom::Rule` → `Node::Rule`，tests_math `math_vrule_kept_in_formula`/`math_vrule_dimensions`） |
| `typeset/math.rs:495` | 分式节点 M4-2 简化（垂直堆叠） | 分式线/字号精化未做（M4-3 fontdimen） | ✅ 已修（2026-09-26 验证）：`fraction_nodes` 已按 tex.web make_fraction 使用分子/分母字阶降级、fam3 fontdimen8 default rule thickness、fam2 fontdimen8..12 间距；`\over` 生成 rule、`\atop` 无线。回归测试 `tests_math::math_fraction_*` / `math_script_box_width_includes_scriptspace`。 |
| `typeset/math.rs:540` | 根式节点字形（原 M4-2 横线） | 定界符码已驱动 small/large 单字形变体（cmsy10/cmex10）；字阶内 char list 逐级放大与 extensible 拼接（cmex10 根号段）仍缺（M4-3） | ⚠️ 部分已修（见 sink.rs:623 行） |
| `typeset/sink.rs:623` | `\radical` 定界符号不参与渲染 | `\radical"161` 等只出 radicand | ✅ 已修（`MathAtom::Radical{delim}` 携 27 位定界符码，radical_delimiter 按 tex.web var_delimiter 拆 small/large 变体取字形、码 0=null 分支宽 \nulldelimiterspace；`\sqrt` 原语补 plain.tex 默认码 0x270370。tests_math `math_radical_*` 三测；LaTeX `\sqrt{x}` 端到端 DVI 实证 cmsy10 0x70 根号 + cmmi10 'x'） |
| `typeset/sink.rs:116` | 非数学模式样式错误（原"简化忽略"） | TeX 报错缺失 | ✅ 已修（942346e math_mode_error；TRIP/ETRIP 未触发） |

## 2. 数学状态机（本轮 l.260-285 暴露；见 MATH-STATE-MACHINE.md）

- `math_shift` 嵌套语义：数学模式内 `$$` 已修（3a2cb63），`\scriptfont 未定义检查` 缺失（trip L260 `$\x` 报错对齐参考）
- 数学内 `$`/`$$` 的 display 探测消费规则（expander mod.rs:1490）
- 数学模式组结束 `Missing $ inserted` 检查（sink.rs group_end，entered_math + box 组排除）
- 数学原语 no-op 的读值语义（`\splitdiscards` 等，9b0bc69 已修）

## 3. 对齐组（halign/valign）

> **2026-09-08 更新**：对齐状态机已由 `ntex-core/src/expand/align.rs`（M4-5）+
> `ntex-layout/typeset/sink.rs` align_fin（两遍列宽定稿）承担，本节早期
> "\cr/\span/\crcr/\omit 无操作"条目已被状态机取代（raw 拦截 + dispatcher
> 误位报错）。剩余简化见 archive/halign-survey.md §3.2（S1–S7）——其中 S1（everycr
> 只存不注入）、S2/S7（to/spread 摊派）已于 2026-09-08 修复（4b912f9）。

| 位置 | 现状 | 状态 |
|---|---|---|
| `expand/primitive.rs:680` | `\cr` 无操作（对齐组按盒子处理） | ⚠️ 已被 align.rs 状态机取代（raw 拦截做 Insert v_j；dispatcher 误位报 Misplaced \cr） |
| `expand/primitive.rs:707` | `\span` 列合并无操作 | ⚠️ 已被 align.rs 取代（preamble 展开一次 + body 分隔符）；span 宽度摊派见 align_fin |
| `expand/primitive.rs:805` | `\crcr` 与 `\-` 简化 no-op | ✅ 已修（2026-09-26）：\crcr 已被 align.rs 取代（行边界冗余忽略 + raw 结束符）；`\-` 现在按当前字体 hyphenchar 发 `Discretionary{pre,post,replace}`（pre 为连字符、post/replace 空）。回归测试 `explicit_discretionary_hyphen_inserts_breakpoints`。 |
| `expand/primitive.rs:1356` / `eqtb/primitive.rs:504` | `\omit` 简化为 no-op | ⚠️ 已被 align.rs 取代（列首 \omit 判定，单元 V 模板置空） |
| `typeset/sink.rs:488` | 组类型 7 不另开列表（沿用对齐组列表） | 待做 |
| `expand/align.rs`（S4） | preamble 深度记账的 cs 形态超集。**原登记定性有误**（2026-09-27 探针修正）：tex.web preamble **确有** brace 深度机制——u/v 段终结条件是 `(cur_cmd in tab_mark..car_ret) and (align_state=-1000000)`（L15433/L15450），align_state 由**字符 token** 专路加减（get_next `mid_line+left_brace` L7335 / get_token 的 `t<cs_token_flag` 分支 L7492）；"整体移除深度跟踪"会偏离 GT。真偏离=cs 形态（`\let\bgroup={`，`EqSlot::Char`）也调 `brace_depth`；已收窄到字符 token（分类仍走 cmd_class 的 cur_cmd 语义）。GT 对拍：`\halign{\bgroup&#\egroup\cr}` 首错 "Missing # inserted in alignment preamble" 两侧一致 | ✅ 已修（2026-09-27） |
| `expand/align.rs`（S5） | 多列越界。**原登记定性有误**：tex.web 不是"越界内容并入末列"，而是 fin_col 的 `<If the preamble list has been traversed…>` 臂报 "Extra alignment tab has been changed to \cr" 后 `extra_info:=cr_code` → **按 \cr 收行**，余下 token 由 align_peek 起新行首列（pdftex 实测 `\halign{#&#\cr a&b&c&d\cr}`：1 条报错、两行 a,b/c,d）。此前 NTex 只报错却钳末列（单行 4 单元且 c/d 宽 0）；已改 align_col_will_overflow 分流 + align_extra_tab_as_cr（`\span` 越界同臂） | ✅ 已修（2026-09-27） |
| `expand/align.rs`（S6） | ①行中 raw `}`（align_state 0→-1）：tex.web raw 判据要求 `=0`（L7264），-1 落 handle_right_brace 的 align_group 臂（L21654）报 "Missing \cr inserted" 并插 frozen \cr，插入前须还原 align_state（back_input L7028 对 brace 逆调；否则插入的 \cr 卡在 raw 判据外 → Misplaced \cr → 死循环）；对齐闭括号唯一裁决位=行边界 align_peek。此前 align_state<0 直接 align_finish → sink 状态失配 0 页 0 错（GT 1 页 1 错）。2026-09-27 复验：`\setbox0=\vbox{\halign{#\cr a}}\showbox0`（nonstop）pdfTeX/NTex 均为 1 条 `Missing \cr inserted`，box 树同形（`\vbox(4.30554+0)x5.0000…`，行盒含 tabskip/`a`/tabskip；仅 1sp 舍入差）。②模式合法性路由（`abs(mode)+cur_cmd` 大 case）：行内数学 \halign 报 "You can't use `\halign' in math mode."、外层水平 \halign 插 \par 重入（head_for_vmode）、受限水平 \halign off_save 插配对 `}`、外层垂直 \valign 开段重入（GT trip.log 1082 `{horizontal mode: \valign}` 同形）、数学 \valign 报 Missing $（不插）。**遗留**：显示数学 \halign 的 `cur_group=math_shift_group` 判定（现放行）、内层垂直 \valign 开段（GT m6 实证 tex.web 也开段）、interwoven fatal（get_preamble_token 的 endv 判据 L15437 + do_endv/fin_col 的 align_state<500000 L15605）、EOF 组回收链（GT `Missing } inserted`→`Missing \cr inserted`；NTex 走 "You can't use \par in restricted horizontal mode"，有报错非静默但诊断不同） | ✅ S6 raw `}` 已复验清零（2026-09-27；模式路由遗留另列见描述） |
| `typeset/sink.rs`（S3） | `\valign` 行高/深度数学整体跳过 | ✅ 已修（2026-09-27）：`align_fin` 的 Valign 分支改成 Halign 列宽数学的对偶：第一遍量各行最大纵向占据（含 span 摊派），第二遍每个 `\cr` 分隔项打成 column vbox，列内保留首/中/尾 `\tabskip` glue，cell vbox 统一到行高且宽度取列宽，列盒直接插入外层水平列表、不再额外包 hbox。GT 探针 `\setbox0=\hbox{\valign{#\vfil&#\vfil\cr \hrule h5pt d2pt w2pt&\hrule h10pt d4pt w4pt\cr}}\showbox0`：pdfTeX/NTex 均为 `\hbox(21+0)x4` → `\vbox(21+0)x4` → glue / `\vbox(7+0)x4` / glue / `\vbox(14+0)x4` / glue（NTex showbox 尚不打印 `(\tabskip)` 名称，尺寸同）。回归锁 `valign_packs_columns_with_row_extents_and_tabskip`。 |
| `typeset/sink.rs:1483` | 数据行边界（此前 no-op 导致列内容混入 vbox，已修） | ✅ 已修 |

## 4. 内部量单独出现（no-op 语义——已系统性修完，勿回退）

- `expand/primitive.rs:55, 355, 1031, 1033, 1059, 1368`：内部整数/只读整数/胶水分量查询单独出现一律 no-op
  （TeX 主循环不读值；参考 trip `{\tracingstats}` 追踪后无操作；9b0bc69 统一）
- `expand/scan.rs:1212`：`\pagegoal` 等排版状态参数暂按 0 读（expander 无排版状态）
- **反例登记（易回退，勿再犯）**：`expand/primitive_param.rs` 的"内部量单独出现 → no-op"
  判定**必须含 `EqSlot::Char`**（`\chardef` 定义的 cs，如 plain 的 `\bffam=6`）。
  2026-09-11 修：此前漏掉该臂 → `\fam\bffam`（**每个 `\bf`**！）被判成"单独出现
  no-op"，`\bffam` 回流主循环后被当**字符**排版——每处 `\bf` 往盒里多插一个字符码 6
  的节点（cmr10 char 6 宽 7.22222pt；换 Unicode 字体后直接报 `Missing character:
  There is no ^^F`）。回归锁 `expand/tests_insert_alloc.rs`
  `chardef_cs_is_a_parameter_value_not_a_character`。
  判据来源：tex.web `scan_int` 的 internal-integer 臂含 `\chardef`'d cs。

## 5. 字体/连字/断字

| 位置 | 现状 | 状态 |
|---|---|---|
| `catcode.rs` / `input.rs` | `\utfinputmode=1`（M9 中文刀 2）已通 UTF-8 直写；**默认 bytes 模式逐字节语义零改动**（TRIP/ETRIP 已对照）。>255 码位的 `\catcode` 覆盖表（刀 4）已落地 | 见下行遗留 |
| `expand/scan.rs` `\catcode` 处理器 | **>255 码位赋值的预读时序**：`\catcode"XXXX=13` 的数值扫描为确认数字结束而预读下一个 token，该 token 用**赋值前**的 catcode 切分——紧跟的 `\def<该字>{…}` 会把 `\def` 与该字粘成一个控制字（`\def中` → `! Undefined control sequence. \def中`），随后该字恒未定义（2026-09-17 实测；`1234567` 行号亦随之错位到 `l.15` 一类越界值）。**规避**：`\catcode` 行与 `\def` 行之间放一个 ASCII token（`\relax`／空行／任何 ASCII 语句）。与 TeX 的差别在**预读深度**：tex.web `scan_int` 只 back_input 一个 token，控制字的扫描留到赋值之后 | 待做（刀 6） |
| 折行 | **CJK 汉字字间断点已通**（M9 中文刀 5，2026-09-17）：`\cjkbreakmode=1` 时段落关闭阶段在可断字间插零宽可拉伸胶水（`0pt plus 0.5pt minus 0.05pt`，XeTeX inter-character skip 同款），断点 + 两端对齐同时到位；开/闭标点禁则按 **gap** 判定（能同时看两侧，故行首禁则与行尾禁则都落地）。**中西文交界断点亦已通**（同日补）：CJK ↔ ASCII 字母/数字之间给断点（拉丁词/数字**整体不拆**，断点只落交界），这是 `\XeTeXlinebreaklocale "zh"` 的等价行为——缺它则一串西文与前汉字之间的**唯一**断点距离可达数十 pt，折行器只能超宽出页。默认关（断点会改折行结果，TRIP/ETRIP 必须零影响）。遗留：不限 CJK 符号挤压（标点宽度不压缩） | ✅ 已修（④ 主项）；标点挤压待做 |
| `ntex-font/otf.rs` `build_metrics` | OTF 度量只有 advance/height/depth（hmtx+bbox），italic correction 恒 0；无 kerning/连字/HarfBuzz 整形 | 待做（M9 ②） |
| `ntex-backend` CJK 渲染 | 字形经 cmap 直查（`unicode_native` 字体 codepoint→glyph）；缺字形逐字回落方框；无 CJK 字体链 fallback | 待做 |
| `ntex-font/tfm.rs:191` | 保留左/右字符的连字（罕见）暂不支持 | ✅ 已修（2026-09-27）：`LigKern::Lig` 携带 tex.web op=`4a+2b+c` 的 `replacement/keep_left/keep_right/skip`，TFM 解析层不再把 b/c 非零连字丢弃；排版层按保留位决定替换前驱、在前驱后插入 ligature、或保留当前右字符。CM 回归锁仍覆盖 `fi/fl/ff`、````、`--`、`!``/`?``、en-dash→em-dash 等既有 lig/kern；新增 synthetic retained-lig 单测覆盖 keep-left/right 形态。 |
| `ntex-wasm/src/lib.rs` `EMBEDDED_TFMS` | 内嵌 CM TFM 原仅 14 件，而 `set_preload_plain(true)` 的 plain 字体块引用 47 件 → Tauri/WASM 下 34 行 `! Font cmr9 not loadable: Metric (TFM) file not found.` | ✅ 已修（2026-09-11：补齐至 48 件 + `embedded_tfms_cover_plain_preload_set` 回归锁防清单再漂移；清单来源见 `crates/ntex-wasm/fonts/README.md`） |
| `ntex-wasm/src/lib.rs` `EmbeddedTfmSource::otf_bytes` | 此前只实现 `tfm_bytes`，`otf_bytes` 落 trait 默认 `None` → wasm/Tauri 下 OpenType 字体**拿不到排版度量**，中文只能逐字节报 `Missing character: There is no ^^e5 …` | ✅ 已修（2026-09-11：新增进程级 `OTF_METRICS` 表 + `set_otf_font(tex_name, bytes)` 导出，一次调用同写「排版度量 + 渲染轮廓」两侧） |
| `ntex-tauri/ui/fonts/FandolSong-Regular.otf` | 中文只有 **Regular 一款**：`\bf` 只切换拉丁字面（cmbx），汉字仍出 Regular 字形 | 待做（M9 中文刀 4：FandolSong-Bold / FandolHei / FandolKai 子集化 + `\bf` 家族按字体名映射） |
| `scripts/make-cjk-subset.py` | 子集化档位 `sym`/`l1`/`full` 目前只按 GB2312 表 + `EXTRA_PUNCT` 取字符；非 GB2312 汉字（生僻字/异体字）仍缺字形→渲染方框 | 待做（按需并入 `full` 档的 `EXTRA` 码位表，或改走 `--text-file` 按实际文稿取字） |
| `expand/primitive.rs:893` | `\varunit` 字体单位 no-op | ✅ 无单独场景（TRIP 仅 dimen 上下文 `20\varunit`） |
| `expand/save.rs:656` | `\the\font` 简化（expander 无排版状态） | 待做 |
| `hyphen.rs:10,47` | 词界限制 `.` 暂不参与断点过滤（真实表里 `.ach4`/`5hand.` 这类词首/词尾受限模式被当任意位置模式，断点全集偏大——论文轮实测 `complex` 断成 `comp-lex`、`co-mplex`，真 TeX 只给 `com-plex`） | 待做 |
| `typeset/typesetter.rs` `preload_hyphen` | **LaTeX 断字表只在 `ntex-dvi` CLI 补种**：`.fmt` 只序列化 core 侧状态（`ntex-core::expand::FmtState`），断字表（`PatternTrie`）住排版器跨不进快照 → `ntex-studio`（`engine.rs` 直读 `latex.fmt`）与 `ntex-wasm`（`set_latex_mode`）恢复 fmt 后 `\language=0` 仍无表、LaTeX 作业不断词。CLI 侧已修（`set_preload_hyphen`，排版入口在用户源前跑内嵌 `hyphen.tex`；真 latex.fmt 由 lthyphen.dtx 在格式生成期载表，NTex 的 ltxinit.tex 只有 `\input latex.ltx`） | 待做（studio/wasm 两条 LaTeX 通路接同一开关；根治 = fmt 快照携带 PatternTrie，跨 crate 边界，另立题） |
| `typeset/paragraph.rs:153` | **`\lefthyphenmin`/`\righthyphenmin` 已接入断点过滤**（2026-09-17 修）：按 tex.web §927 `norm_min`（`<=0`→1、`>=63`→63）钳制 `misc[MISC_LEFT_HYPHEN_MIN]`/`MISC_RIGHT_HYPHEN_MIN`，再按 §924 `found:` 只保留 `l_hyf <= j <= hn - r_hyf` 的断点；`hn < l_hyf + r_hyf` 时整词不尝试（tex.web `goto done1`）。**两条路统一过滤**：模式表与 `\hyphenation` 异常词表都过同一道筛（词首/词尾断点随之被清）。现场：resume 正文 `Python/Java` 原断成 `J-`/`ava`，现不再断 | ✅ 已修（回归锁见下行） |
| `typeset/tests.rs` | 断字最小宽回归锁：`hyphen_minima_filter_pattern_breaks`（l/r 三档 A/B 对照，含 `hn < l+r` 整词跳过）+ `hyphen_minima_filter_exception_breaks`（`\hyphenation{a-b-c-d-e}` / `ab-cde` 逐位断言）；两条均在「去掉过滤」时失败（已实测），是**真锁**而非顺带通过 | ✅ 已锁 |
| `expand/primitive.rs:1492` | 断字表单语言全局（sink 不分语言；无 lccode 二次比较） | 待做 |
| `crates/ntex-tauri/ui/`（字体资产 + 名单） | 工作台真字形覆盖面：名单只覆盖手挑的少数 CM 尺寸 → LaTeX 点名的 `cmr8`（`\LaTeX` 徽标的 A、`\sf@size` 脚本档）、`cmmi12`（12pt 数学）等落灰方框（2026-09-25 用户截图两连） | ✅ 已修（2026-09-25）：按 `ot1cm*.fd`/`om*.fd` 声明的名字矩阵补 33 个 LM OTF（`ui/fonts` 共 49 件）、`GLYPH_FONTS` 按声明名重写（70 名）、`lm_text_style` 补 `lmromanslant`/`lmromancaps`/`lmmono*-italic\|slant\|caps`/`lmromanunsl`/`bolditalic`/`lmsans*-oblique\|bold`/`lmsansdemicond` 与数学粗体（`cmmib`/`cmbsy`）及大字号族（`icmmi`/`icmsy`/`icmex`）。**遗留**：①`crates/ntex-wasm/www/index.html` 是另一份更小的内联名单 + 独立 `www/fonts/`（**未同步**——浏览器工作台（B/C 档）同类现场仍会方框；要同步就是 33 个文件再复制一份，待与它合并或改共享目录）；②`line10`/`lcircle10`（picture 环境画图字体）LM 无 OTF，画图命令的预览仍是方框；③`cmvtt10`/`cmvtti10`（变宽打字机）LM 无对应，按等宽近似；④LM 无独立 `icmmi8`/`icmsy8`/`icmex10` 文件，按 `latinmodern-math.otf` 近似；⑤EC 的 **T1** 族（`ecrm*` 等）与其余 TS1 族仍未随包（同上条） |
| `ntex-backend/src/glyphs.rs` | 字形通道编码覆盖：EC「TC」**TS1** 族（`tcrm*` 等）此前既无槽位表也无族→LM 文件映射 → LaTeX 的 `\text…` 文本符号与 `\thanks` 脚注标记（`\textasteriskcentered` = TS1 居中标星）在预览里落方框；且 `assets/tfm` 不带这批度量，工作台（wasm 无 TeX Live 回落）直接 `Font … not loadable` | ✅ 已修（2026-09-24）：① `ts1_to_unicode`（槽位锚定 `ts1enc.def`，含 `\char42` 居中标星臂）；② EC 四位数尺寸名归一到 LM 光学尺寸档（`lm_optical_size`，`tcrm1095`→`lmroman10-regular.otf`）；③ `assets/tfm` 随包 `tcrm/tcti/tcbx/tcss/tctt` × 全档度量（EC 包，CTAN other-free）；④ studio 认领表 `lm_tex_name_candidates` 与工作台前端 `GLYPH_FONTS` 双侧同源（`ui_font_manifest_matches_rust_claim` 钉住）；⑤ PDF 侧复用同一 OTF + slot 表（不再要求 TS1 PFB）。**遗留**：TS1 斜体族 `tcsl`/`tcui`/`tcbl`/`tcsi`/`tcso`/`tcst`/`tcit`/`tcvt`/`tcvi` 未随包（LM 无对应 OTF 名，命中即方框）；EC 的 **T1** 族（`ecrm*`/`ecbx*`…）是 Cork 编码，本层不认（按 OT1 语义走，字形可能错位）——`\usepackage[T1]{fontenc}` 文档的准确渲染待做；TS1 旧式数字（0x30..0x39）按同位 ASCII 近似（LM 无旧式字形）；`\textborn`/`\textdied`/`\textleaf`/`\textbigcircle`/`\textlangle` 等 LM 无同形件的槽位仍方框 |

## 6. 其他

| 位置 | 现状 | 状态 |
|---|---|---|
| `expand/macros.rs:175,323` / `save.rs` | `\outer` 限制语义 | ⚠️ 部分已修（a4c2aeb）：展开上下文禁止（宏体/实参——scan_depth>0）+ \def 体跳过 + cs 名 ^^ 转义；Runaway 块/对齐模板场景待错误恢复链统一 |
| `expand/mod.rs` / `expand/scan.rs` / `expand/cond.rs` | M1-13 错误恢复主干：Undefined control sequence 当 `\relax` 继续、多余 `}`/`\fi`/`\else`/`\or` 忽略继续、Missing `{`/number/`endcsname` 等扫描点插入或取默认值继续；`ntex-dvi --interaction=nonstopmode` 已接到 `\interactionmode`。2026-09-27 探针（article + `\undefinedcs` + `\badmacro{x}`）与本机 `pdflatex -interaction=nonstopmode` 均为 2 页、2 条 Undefined，NTex DVI 含错误前后文本。 | ⚠️ 部分达成：主循环仍将文件/I/O/fmt/栈与步数上限等 fatal 冒泡；write/message/special 构串阶段 undefined cs 仍按下行登记静默；错误去重目前依赖各恢复点消费/插入 token，尚无统一“同 token 位同类”集合。 |
| `expand/io.rs:304` | `\write18` shell 转义拒绝（Error） | ✅ 设计如此（RFC-3 副作用隔离；TRIP/ETRIP 不触发） |
| `expand/io.rs` `expand_to_string` | write 构串展开阶段对 undefined cs **不报** `Undefined control sequence`（静默跳过；pdfTeX 报错后恢复丢弃，输出面恰好一致；`\message`/`\show`/`\special` 共享此函数） | 待做（expl3 载入期大量暂未定义 cs 依赖静默推进） |
| `typeset/sink.rs:1519,1525` / `mod.rs:841` | `\moveleft/\moveright` 位移不落节点（取走即清） | 待做 |
| `typeset/sink.rs:1101` | 非引导上下文未定宽度简化落 0 | 待做 |
| `typeset/sink.rs:1439` | `\showlists` 简化转录（诊断用） | 待做 |
| `expand/primitive.rs:2684` | `\showifs` 简化格式 | ✅ 诊断原语（ETRIP l.651 被错误交互打断，无直接比对场景） |
| `ntex-trip/harness.rs:8` | ETRIP 终端输出经 dvitype 比对暂不纳入 | 待做 |
| `expand/macros.rs:844` | 宏不复制宏体（M1 简化） | 待做 |
| `expand/scan.rs:32` | 十进制扫描 M1 简化版 | 待做 |
| `expand/primitive_io.rs` `\output` 存储 | preview L380 `\let\output\pr@output`（委托给 toks17）后，引擎侧 `output_toks` 仍持旧 token 列表——委托关系不回灌引擎存储，页出货走的是 toks17 里的例程但 `\shipout` 判定/页装配仍看旧例程 → preview 文档载入成功但 0 页出（`\shipout 未触发`）。tex.web 无此分裂（`\output` 就是 toks255） | 待做（`\let` 写观测面回灌 output_toks，或引擎侧改挂 toks255 槽） |
| `expand/io.rs` `\read` | 算法宏包链（algorithm.sty/algpseudocode）依赖的 `\read` 流未打开时报 `非法输入：\read 流未打开` 致命，pdfTeX 语义是未打开流读终端/报错后可恢复 | 待做（Calculate-Legendre l.10 现场停步） |
| `ntex-dvi`/`ntex-format` 生产路径 panic 审计 | 全量核查（`unwrap()/expect()/panic!/unreachable!/todo!` + 输入索引）：生产代码仅 ntex-dvi lib.rs `ctm.last_mut().expect("缩放栈底恒在")` 一处（不可达：栈底构造时播种 (1,1)，pop 有 `len>1` 守卫，符合「不可能位 expect+不变量说明」口径）；任务书原列的 15 处 unwrap 全在 `#[cfg(test)] mod tests` 内（`input.rs` 已随分片重构更名）。加固：ntex-format 解码期预分配/名字长度改按 `PREALLOC_CAP`(64 KiB) 钳制——损坏 .fmt 的 u32 长度域填 2^32-1 原会容量溢出 panic/OOM abort，现走 io::Error 优雅失败（循环计数不变，合法文件零影响；DVI 产物逐字节一致）。`production_code_has_no_unwrap` 守护测试固化两 crate 的「生产禁 unwrap」门禁 | ✅ 已修（2026-09-23 技术债第一批项 4；`load_huge_count_fails_fast_not_oom`/`load_oversized_name_length_rejected`/`load_huge_eqtb_slots_fails_fast` 三测） |

---

## 7. demo1 数学/输出一致性战果（2026-09-07 登记）

demo1 六刀 + 输出例程刀 2/3/5 的修复登记；全部已提交，留作回归排查索引（踩坑前先查表）。

| 位置 | 事项 | 状态 |
|---|---|---|
| `typeset/typesetter.rs`（tex.web §4852） | 大写字母 `\sfcode=999` 初始化（原 INITEX 初表清零、须 plain 赋值一类） | ✅ 已修（e0fb4fd） |
| `typeset/math.rs` + `ntex-font/tfm.rs` | 斜体修正 kern 全链（tex.web §759；无下标才落、cmmi10 E=37773 截断） | ✅ 已修（0236876） |
| `typeset/math.rs:1069` | mu→sp 整数截断 `mu_to_sp`（`cur_mu=em/18` 截断不可省——demo1 对照 5×36408，精确除法差 4sp） | ✅ 已修（70a8492 缩放 + cf048af 截断语义） |
| `typeset/math.rs` / `mod.rs` | display 公式盒 interline glue（tex.web 垂直列表 + append_to_vlist，非"水平嵌 hbox"） | ✅ 已修（2fa7e4b） |
| 同上（退出时长短 skip 裁决） | pre_display_size 长短 skip 裁决 | ✅ 已修（2fa7e4b） |
| `ntex-dvi/lib.rs` | 页号链 count0 页标签（原 `[0.0.0.0.N]` 硬编码） | ✅ 已修（375b390） |
| `typeset/mod.rs` / `sink.rs` | box255 寄存器化统一访问面（box_view/take_box_at/write_box 三访问面） | ✅ 已修（c5d02b9） |
| `typeset/paging.rs` / `node.rs` | insert 结构化 token 体保留 | ⚠️ 部分已修（577ed3c）：体已保留，**体排版仍挂账**（脚注仍不可用，见 plan.md §5 P1） |
| `ntex-dvi` transcript 通道 + `--input-path` | 诊断转录（stderr 默认开）+ `\input` 搜索路径（SearchPathVfs）——补"undefined cs 静默跳过"盲区 | ✅ 已修（4ec6a84） |
| `typeset/paging.rs` / `node.rs` | insert `\newinsert` 分配器 + `\count/\dimen/\skip` 三联寄存器（`\footins=\insert254` 与真 plain 一致） | ✅ 已修（3ef1f67，输出例程刀 4；体排版仍挂账，见上行） |

### 7.bis 格式预载（G 线）登记（详见 archive/plain-format-survey.md §5.bis.g）

> ⚠ **文档改版（2026-09-19）**：`latex-feasibility.md` / `plain-format-survey.md`
> 等勘察全文已归档到 `docs/archive/`（见 `docs/archive/README.md`），
> 当前进度与剩余项一律看 `plan.md`。本文件中对旧节号的引用（如「§36」「§15」）
> 请去 `docs/archive/` 下同名文件 grep。

| 位置 | 现状 | 影响 | 状态 |
|---|---|---|---|
| `ntex-layout/typeset/plain_format.rs` `EmbeddedFormatVfs`/`set_preload_plain` | 各渲染端无 `\input plain` 契约 → plain 宏全缺、样例产空页 | ✅ **已修**（G4 接线：ntex-backend/ntex-wasm/ntex-mcp 三端统一 `use_embedded_format()` + `set_preload_plain(true)`，与 ntex-dvi 同路径）|
| 预载路径 | 预载后空文档经 `\plainoutput` 收尾冲出一页空页 | corpus 内宏/格式文件（无 `\bye`）页数与真 TeX（0 页）不一致 | 待做（survey #1） |
| `\lccode/\uccode` 初表 | 预载前初表全 0，未按 INITEX 初值（`a..z`/`A..Z` = 自身）播种 | 预载失败回落时 `\lowercase` 行为偏差 | 待做（survey #2） |
| `expand/macros.rs` 实参扫描 | `\char`/`\number`/`\romannumeral` 在**实参位置**取不到数字时**不报** `! Missing number, treated as zero.`（pdfTeX 报）| 错误报告面缺失（排版结果不变，影响诊断保真与 TRIP 口径）| 待做（archive/latex-feasibility.md §A1.octies；回归锁 `tests_scan.rs` `arg_scan_does_not_expand`）|
| `expand/primitive.rs` `meaning_text` / `save.rs` `slot_display` | 宏的 `\meaning` **只渲染 `#n` 而丢参数文本定界符**（pdfTeX `\meaning\if@` = `macro:if->`，NTex 误报 `macro:->`）——曾把 §38/§39 的根因误判为 `\uppercase` 语义问题 | `\meaning` 输出失真；误导诊断（非功能缺陷） | ✅ 已修（2026-09-11，改渲染 `params.text`，tex.web `print_meaning` 的 `token_show(参数文本)`；测试 `tests_scan.rs` `uppercase_param_text`） |
| `expand/scan.rs` | `scan_glue` 胶水上下文不认 dimen 寄存器别名（`\skip_const:Nn \c_zero_skip{\c_zero_dim}` 致命） | latex.ltx l.13899 停点 | ✅ 已修（226c177，`RegKind::Dimen` 臂） |
| `scripts/corpus-probe.py` | 产物路径按仓库根找（`REPO/<stem>.dvi`），但 ntex-dvi/ntex-pdf 默认输出**保留源文件目录**（`<输入去扩展名>.dvi/.pdf`）→ 全库误判 FAIL | KPI 假阴性（曾把实际成功的 `plain/*.tex` 全判 0/8） | ✅ 已修（2026-09-11，路径按 `tex.parent` 取 + 清理同步） |
| `scripts/corpus-probe.py` | 只判产物存在性，不判内容 → 宏/格式文件的**空页**也计 PASS | KPI 假阳性（`plain/plain.tex` ink=7 计 PASS） | ✅ 已修（2026-09-11，非白像素判据 `MIN_INK=200`，三级 FAIL/EMPTY/PASS） |
| `expand/scan.rs` | `scan_int` 数字上下文不认 dimendef'd cs（`\rm`=`\fam\z@\tenrm` 的 `\z@` Missing number） | plain 预载 l.1237 停点 | ✅ 已修（04d2023，plain 预载全通） |
| `\advance/\multiply/\divide` | 目标集合不含 page 参数寄存器（`\advance\vsize`…类死点） | letterformat.tex FAIL | ✅ 已修（3af87dd，tex.web do_register_command） |

---

## 8. 架构债（2026-09-07 架构评估登记）

结构性债务，非语义简化；与逐项修复分开追踪。括号内为 R3 当日实测，供治理时校准。

| 对象 | 问题 | 状态 |
|---|---|---|
| `ntex-core/src/sink.rs:49` `TokenSink` | 单接口百级方法（评估口径 116；实测 trait 内 107 个 fn），VM↔排版耦合面过宽 | R1 治理中 |
| `ntex-layout/src/typeset/mod.rs:511` `NodeBuilder` | 上帝对象（评估口径 51 字段；实测 62，511-679 行），状态难回滚/难并行 | 待 R1 后的 R1b |
| `ntex-layout/src/typeset/tests.rs`（2606 行）、`ntex-core/src/expand/tests.rs`（4300 行） | 巨型测试文件，定位与并行编辑困难 | R2 治理中 |
| `ntex-dvi/src/lib.rs:37-44` `write_dvi` | 旧驱动仍用合成 counter（count0=页序号，刀 5 残留）——换装 `write_dvi_with_counts` 即可 | 待做 |

---

## 9. ntex-pkg 宏包管理（M9，2026-09-19 新建）

> 定位：`crates/ntex-pkg/src/`（plan.md §6.2 第 8/9 条落地）。
> **解析层已对照真实 `texlive.tlpdb` 验收**（2024basic，346 记录），路径选择以 `kpsewhich` 为
> oracle **9/9 一致**（article.cls / latex.ltx / expl3-code.tex / geometry.sty / amsmath.sty /
> cmr10.tfm / plain.tex / size10.clo / ot1cmr.fd）。下表为**取料层/收敛层的刻意简化与未实现插口**。

| 位置 | 现状 | 影响 | 状态 |
|---|---|---|---|
| `tlpdb.rs:143` | `relocated` 字段解析并保留；**`RELOC/` 前缀映射已实现**（`normalize_path` 剥除、`install_rel_path` → `texmf-dist/`），但**包级 relocation 目标未实现**（不读库里的 relocation 表，一律按 `texmf-dist/` 落） | 带自定义 relocation 前缀的包（如 tlpkg 自举包）落点可能不符 | 部分已做（2026-09-19 修 `RELOC/`；剩余待做） |
| `tlpdb.rs:62` `host_arch` | 只做四类平台映射（macOS 统一 `universal-darwin`、Linux aarch64/x86_64、Windows 统一 `windows`）；未知平台返回空串 | 未知平台的 `.ARCH` 依赖落入「未解析」并由调用方显式报告——**不静默猜测** | 设计如此 |
| `tlpdb.rs:495` `tds_rank` | 固定 TDS 层级序（`tex/latex` → `tex/generic` → `tex` → `fonts` → …），非真实 kpathsea 的 `TEXINPUTS`/`texmf.cnf` 路径序；**已修「先 `normalize_path` 再判档」**（TL2026 的 `RELOC/` 前缀曾使全部路径落兜底档 → 稳定版与 `latex-base-dev` 平票取到预测试版） | **仅当同一 basename 有多个不同层级提供者**时才可能与真实 TeX 选路不一致 | 路径序待做（前缀归一 ✅，回归锁 `tds_rank_normalizes_before_ranking`） |
| `tlpdb.rs`（`bin_files` 臂） | `binfiles` 解析保留 basename，**未保留架构子路径** | bin 类资产定位不可靠（当前主线不使用） | 待做 |
| `source.rs:244` `UnimplementedSource` | **仅剩 CTAN / 离线归档两源** `fetch_container` 返回 `SourceNotImplemented`（**不静默降级**）；② tlnet 已实现（`tlnet.rs`） | 无 CTAN 回落取料、无内网离线取料能力 | 待做（§6.2 第 9 条刻意的插口） |
| `source.rs:167` `LocalTexLiveSource::probe` | `ntex-io::Vfs` **无 `exists`**，探针只能对每个文件 `read`（逐文件 O(文件数) 次读） | 大包 completeness 探针有读放大 | 待做（Vfs 补 `exists` 后收敛） |
| `cache.rs:38` `sha512_hex` | ✅ **已修（2026-09-19）**：接入 `sha2` 0.10 做**真实 SHA-512 字节哈希**（NIST 空串/`abc` 向量为回归锁），② tlnet 取料层据此校验容器 | —— | ✅ |
| `cache.rs:38` `package_dir` | `<root>/<name>@<revision>` 的**逐包布局**，与 `tlnet::materialize` 铺出的 **TDS 树布局**（`<root>/texmf-dist/**` + `<root>/tlpkg/texlive.tlpdb`）不是同一套 | 两个"缓存根"语义并存，`local` 子命令打印的路径只是"包应当落哪"的提示 | 待做（统一缓存布局 + GC/复用策略） |
| `tlnet.rs:76` `HostContainerIo` | 宿主 `curl` + `tar` 当**边缘**（HTTP/TLS/xz 全在外部进程），不引 HTTP 客户端/xz 依赖；缺命令时报可判读错误 | 依赖宿主有 `curl`/`tar`（macOS/Linux 恒有）；**无重试、无断点续传、无并发** | 设计如此（与 `ntex-dvi` 把 `kpsewhich` 当最后一层搜索链同一手法） |
| `tlnet.rs:247` `fetch_verified` | 只做 SHA-512 **容器**校验；**无 GPG detached 验签**（TLS 由 `curl` 提供） | 能防"容器与 TLPDB 对不上"，不能防"TLPDB 本身被替换" | 待做（GPG 公钥环未接入；§6.2 第 9 条列的 GPG 签名项） |
| `tlnet.rs:313` `TlnetSource::fetch_container` | `PackageSource` trait 只给包名，拼不出含 revision 的 URL → 显式返回 `NotLockable` 并指向 `fetch_verified` | 想用统一 trait 面做 tlnet 取料的调用方必须改走 `TlPackage` 入口 | 设计如此（不可锁定就不可复现，不猜"最新"） |
| `tlnet.rs:352` `materialize` | 缓存树只有 `texmf-dist/**` + `tlpkg/texlive.tlpdb`；**无 `tlpkg/tlpobj/**`、无 `ls-R`、无 binfiles**；容器内 doc/source 条目一律丢弃 | 缓存树可当 TL 树根喂给 ① 与 `vendor`，但**不能当真正的 TeX Live 安装用** | 设计如此（不把整个容器灌进资产树） |
| `tlnet.rs:298` `TlnetSource::is_available` | 只判「仓库串非空」，**不联网探测** | 配置了不可达仓库时，"可用性检查"会通过、真正取料才报错 | 设计如此（trait 契约要求可离线判定） |
| `vendor.rs:214` `apply` | 逐文件读源树 + 读目标树做**字节比对**（`Vfs` 无哈希、无 mtime），无索引加速 | MB 级资产可接受；全量 TL 树规模会慢 | 待做（可加 sha512 索引/清单） |
| `ntex-io/src/lib.rs` `Vfs::create_dir_all` | 默认实现为 **no-op**（平坦后端如 `MemVfs` 无需建目录）；只有 `LocalVfs`/`SearchPathVfs` 真建 | 平坦后端上调用它不报错也**不检查**路径——不建目录的后端写"嵌套"路径仍会成功（键是整串） | 设计如此 |

---

## 维护记录

- 2026-09-17：第十八刀尾递归鞍具 ✅ 已修（d222043，已用 `git show --stat` 核对）：
  `tests_bytecode18` 补数字终止空格及 `\iter` 调用边界，三项双轨回归使门禁
  797→800。pdfTeX 对照证明无空格的 `20000\expandafter\iter\fi` 同样爆栈；
  `TokenList(pos=0)` 是未消费 token，不能作为耗尽帧删除。本轮没有新增简化或
  修改引擎。**真实 latex.ltx 主墙仍待修**：500 秒超时，pos=685828（88.4%）、
  steps=11180000；细节见 [真实跑分 §第十八刀](archive/expl3-real-scoreboard.md#第十八刀尾递归鞍具校准2026-09-17)。

- 2026-09-02：建清单（58 处标记扫描归档）；数学内 `$$`、`\mkern/\mskip` mu 上下文已修（3a2cb63）
- 2026-09-02：内部量 no-op 统一、`\right` 缺配对恢复、数学模式组结束 Missing $（9b0bc69）
- 2026-09-02：fraction 原语族挂载 + Ambiguous 恢复式（721646c）；\tracingcommands2 可选 `=` 赋值开启追踪（6cc16f0）；mode_name internal vertical（483b640）；\if 求值 {true}/{false}（3a5ec1e）
- 2026-09-02：\eqno/\leqno 非数学报错 + math_style 报错（942346e）；\outer 展开上下文禁止（a4c2aeb，R3 补登 commit）
- 2026-09-07：R3 账实同步——5 处"未提交"悬空项核实归位（eqno 多报 f75a638、radical l.412 报错 f75a638、math_mode_error 942346e、\outer a4c2aeb，位置列顺带刷新到分片后路径）；demo1 六刀 + 输出例程刀 2/3/5 战果登记（§7）；架构债小节新设（§8）
- 2026-09-26：双栏短文档 0 页修复（断点惩罚 FIFO + 消费计数松绑，本次刀）——`fired_penalty` 单槽改 `fired_penalties` 队列（page.rs，与 `pending_pages` 1:1 入队，注入 take-per-injection，队空回退 `last_fired_penalty`，`\outputpenalty` 只写不清的 tex.web 语义）；p=0 惩罚按非惩罚断点记账（tex.web `if pen<>0` 才插行间惩罚节点，NTex 段落器无条件插 p=0 节点被列表测试钉住，故在 page.rs 惩罚臂对齐）；`-10001` 持盒臂 + `output_consumed` 松绑（expand/mod.rs maybe_inject_output）。**已登记偏差**：松绑使 TRIP insert-split 演练块（trip.tex L114 组）例程多跑、多出 22 页 shipout（5→27，转录 +66 行）；纯 HEAD 口径（去掉 consumed 项）TRIP 字节一致但双栏回退 0 页——两难取主目标，回退开关=`crates/ntex-core/src/expand/mod.rs` maybe_inject_output 的 `&& !consumed` 一项。transformer 16→18 页为既登记「table 浮体丢失」债的回收（页 1–16 字形一致）。
- 2026-09-08：LaTeX 战役二十九刀——\expanded 实参 IPN ×256 清零（scan_edef_body 加 in_definition，78dd892）+ \lowercase 转换 active char（token 表示层 CS_ACTIVE_FLAG，78dd892），详见 archive/latex-feasibility.md §36；\halign 战役刀 1/3——\everycr 两点注入 + align_peek 入口 align_state 复位 + to/spread 摊派真语义（4b912f9），§3 对齐组条目同步刷新
- 2026-09-09：格式预载/scan 线补登（§7.bis 新设）——scan_glue dimen 臂（226c177，latex.ltx l.13899 \skip_const 停点消除）+ scan_int dimendef 数字上下文（04d2023，\z@）+ G3 page 参数（3af87dd）；输出例程刀 4 \newinsert 分配器补登（3ef1f67，§7）；新增三项简化登记：EmbeddedFormatVfs 仅 ntex-dvi 接线、预载空页、\lccode/\uccode 初表全 0（510431b 系）
- 2026-09-10：M9 中文刀 1 登记（§5 新增三行）——\char 上界按字体判定（`FontLoader::char_code_limit`，8-bit 255/Unicode 0x10FFFF，TRIP/ETRIP 口径不变已对照 HEAD 逐字节验证）+ OTF→FontMetrics 直映通道（`unicode_native`/`unicode_chars`，无 ic/kerning）+ 渲染 cmap 直查；遗留：输入层 UTF-8（A5，刀 2）为源文件直写中文前提
- 2026-09-11：M9 中文刀 2 登记（§5 首行刷新）——`\utfinputmode`（misc 65）UTF-8 直写通路打通，默认 bytes 零改动；遗留改为：\catcode >255 赋值扩展（刀 3）、CJK 断行/标点挤压（④）
- 2026-09-11：M9 中文刀 3 登记（**Tauri/WASM 端中文端到端打通**）——本次修自 `resume-plain.tex` 在 Tauri 报错起：① 内嵌 CM TFM 14 → **48 件**（`! Font cmr9 not loadable` ×34 的根因）；② `EmbeddedTfmSource::otf_bytes` + `set_otf_font`（wasm 侧 OpenType **度量**缝，此前只喂渲染轮廓）；③ 引擎级 UTF-8 开关 `Typesetter::set_utf8_input`（经 `Expander::set_misc_int` 写 misc 65，**不改源码**以免 log `l.N` 与编辑器行号错位）；④ `expand/primitive_param.rs` 内部量判定补 `EqSlot::Char`（`\fam\bffam` 误判 → 每个 `\bf` 多插一个字符码 6，见 §4 反例登记）；⑤ `resume-plain.tex` 加 NTex 三分支（`\ifx\utfinputmode\undefined`）。新增回归锁 4 项（ntex-wasm）+ 1 项（ntex-core）。遗留：中文无粗体（同月刀 4）、`\catcode` >255、CJK 断行/标点挤压（④）
- 2026-09-12：`\hrule` 横线消失修复（两处）——① `typeset/sink.rs` rule 分支不再把无 width 说明的 null_flag 落 0，保持哨兵到出货端，DVI `vlist`/渲染 `collect_vlist` 按 tex.web L12598 `rule_wd:=width(this_box)` 解析为包含盒宽（layout lib 再导出 `NULL_FLAG`）；② `expand/primitive_box.rs` 补 head_for_vmode 语义——主水平模式的 `\hrule` 先隐式 end_graf（此前规则被塞进后段行盒内部）。仍存简化（本次登记不修）：`\vrule` 的 height/depth 扫描期落 0（tex.web 保持 null 到 hpack/hlist_out 取包含盒轴高深）→ 行内竖线不长高；受限水平模式 `\hrule` 应报错（tex.web head_for_vmode "except with leaders"）现仍静默落盒；showbox 对 null 宽已打 `*` 但 TRIP 口径未系统校准。
- 2026-09-12：`vbox_dimensions` 修复——垂直装盒漏算 **glue/kern 宽度**（tex.web L13196/L13209 的 `x += d + width; d := 0` 结转臂缺失，只对盒/规则求和）。后果：`\vtop` 内多行段落的行间 `\baselineskip` glue 不占高 → 盒总深低估 → 外层 `\line` 深度偏小 → 页构建把下一行压到换行第二行上（resume-plain.tex 换行条目行距消失，用户现场报告）。DVI 出货端 `vlist` 本就推进 glue，故盒内行距看似正常、行间蹊跷只在「换行行的下一行」暴露——定位时易误判。另：`height` 语义从「首盒高」改为 tex.web 的「自然高 x（不含末件挂起深度）」，depth = 末件深度。
- 2026-09-17：M9 中文刀 5 登记（**CJK 汉字字间断点，④ 主项销账**）——`\cjkbreakmode`（misc 66，默认关）打开后 `close_paragraph` 在可断汉字字间插零宽可拉伸胶水（`0pt plus 0.5pt minus 0.05pt`，同 XeTeX inter-character skip），折行断点与两端对齐同时到位；禁则按 **gap** 判定（`charcode_of(前)`×`charcode_of(后)`，开括号不得收行、闭标点不得起行都落地——单字符 active 宏方案做不到行尾禁则）；与断字 discretionary 同层插入，走既有 `preprocess` 的 Glue 臂，`linebreak.rs`/`knuth_plass` 签名零改动。现场：`resume1-plain.tex`（LLM 手写朴素 plain 源，73 行）原报 7 处 Overfull（最甚 338pt 出页、内容被裁），开启后**零 Overfull**、中文段落正确折行两端对齐。同轮登记两项**新发现**（均未修，见 §5）：① `\catcode` >255 赋值的预读时序（`\catcode"XXXX=13` 紧跟 `\def<该字>` 会粘连，附规避写法）；② 断字未滤 `\lefthyphenmin`/`\righthyphenmin`（`Python/Java` 断成 `J-`/`ava`，会动 TRIP 口径故单独立题）。回归锁：`cjk_break_mode_controls_han_breakpoints`（排版 A/B 对照）+ `linebreak.rs` 三项单测（区段判定 / 禁则真值表 / 胶水插入与断点）。
- 2026-09-18：刀 5 折行收尾（**中西文交界断点 + 断字最小宽，两项遗留销账**）——① 上文 2026-09-17 登记的「假名/汉字与西文交界不插断点」属**保守过度**：XeTeX `\XeTeXlinebreaklocale "zh"` 同样在交界给断点，缺它则 `…数据库原理、Python/Java开发` 一类的**唯一**断点距离可达数十 pt（实测该行 Overfull 20.6pt）。改为 `cjk_breakable` 三类判定：CJK↔CJK（守禁则）、CJK↔ASCII 字母数字（可断，拉丁词整体不拆）、其余不插。现场：resume1 **0 处 Overfull/Underfull**，`pdftotext -layout` 与 XeTeX 对照驱动**逐行一致**。② `\lefthyphenmin`/`\righthyphenmin` 按 tex.web §924/§927 落地（`norm_min` 钳制 + `l_hyf <= j <= hn-r_hyf` + `hn<l_hyf+r_hyf` 整词跳过；模式表与异常词表**统一过滤**）。**证据口径**：改动前先取 XeTeX 参考（`xetex` + FandolSong 同参数）作为折行 oracle，再以 `git worktree` 拉 HEAD 干净树跑 `ntex-trip --test both`，两份输出规范化临时路径后 **`cmp` 逐字节一致**（TRIP `组未闭合 […Align]`、ETRIP l.332 `group_end 无配对` 均为 §5 已登记的既有基线失败，形态未变）→ 零回归。回归锁：`cjk_breakable` 真值表扩中西文交界三例、`hyphen_minima_filter_pattern_breaks`、`hyphen_minima_filter_exception_breaks`（后两条均实测「去修复即失败」）。
- 2026-09-19：M9 宏包管理 **ntex-pkg** 新建登记（§9 新设）——plan.md §6.2 第 8/9 条落地：TLPDB 解析（continuation 行状态机 / `runfiles size=N` 是 **RIV 块计数**非行数 / `.ARCH` 展开）+ 文件反查索引 + `\usepackage`/`\documentclass`→包解析 + 依赖闭包 BFS + `ntex.lock` 确定性契约（身份字段 name/revision/sha512 校验、参考字段忽略）+ 内容寻址缓存 + 可插拔取料源链（仅 ① 本地 TeX Live 树已实现）。**解析层以真实 TLPDB（2024basic，346 记录）+ `kpsewhich` 路径 oracle 9/9 验收**；73 项单测 + `make check` 全绿。刻意简化 8 项（relocated 未实现 / tlnet·CTAN·离线三源未实现 / sha512 只验形状 / tds_rank 简化 / VFS 无 exists 等）见 §9。
- 2026-09-19（同日第二刀）：ntex-pkg **取料层 ② + 资产物化**登记（§9 增量）——`tlnet.rs`（② tlnet 镜像：URL 由 `revision` 钉死、下载后按 TLPDB `containerchecksum` 做**真实 SHA-512 字节校验**、容器条目按 `runfiles` 裁剪 + `RELOC/`→`texmf-dist/` 重定位）+ `vendor.rs`（依赖闭包 → TDS 子树物化，四态 `Add`/`Differ`/`Identical`/`SourceMissing` 逐字节比对，**源缺失显式报告并阻塞收敛**）+ `Vfs::create_dir_all`（宿主侧建目录能力，默认 no-op 不破平坦后端）+ `tlpdb.rs` 新增 `install_rel_path`（已安装树与 tlnet 缓存树共用一条路径映射）+ `tds_rank` 修为**先归一后判档**（真缺陷：TL2026 `RELOC/` 前缀曾使稳定版 `latex` 与预测试版 `latex-base-dev` 平票，取到 dev）+ `cache.rs` `sha512_hex` 接入 `sha2`（NIST 向量锁）+ CLI `vendor` / `fetch`。**验收**：① 离线——`make check` 全绿（ntex-pkg 92 单测，较上刀 +19），② 的协议逻辑（URL 构造 / 缺校验和拒绝 / 校验和不等 / 条目裁剪与前缀不泄漏 / `materialize` 产物可被 ① `LocalTexLiveSource` 接受）全部以**假边缘**覆盖，测试不依赖网络。② 真实镜像端到端（2026-09-19，阿里云 CTAN 镜像 `mirrors.aliyun.com/CTAN/systems/texlive/tlnet` + 真实 TL2026 TLPDB 20.7MB）：`fetch infwarerr` → 1 容器 1 文件 8.2KB（SHA-512 校验通过，URL 与库内 `revision 79461` 一致）；`fetch --documentclass article` → `latex.r79618`，171 文件 2.7MB；`vendor --write --lock` → 目标树 171 文件，**复跑一致 171 / 新增 0**（幂等）；`resolve --documentclass article` 选定稳定版 `latex`（`RELOC/tex/latex/base/article.cls`）而非备选 `latex-base-dev`，即 `tds_rank` 修复在真实库上生效。（注：CTAN 主站与清华镜像在本机仍被拦 403，故走阿里云镜像。）**未做**：GPG 验签、③ CTAN / ④ 离线归档、重试/断点续传、缓存布局统一（上表新增 7 行）。
- 2026-09-19（同日第三刀）：**preview.sty 通路两修**（`\toks` 系赋值 RHS 族 + optional space 消费位）——① `scan_toks_rhs` 补 `\output` 作 RHS 臂（tex.web `toks_register,assign_toks` 共用分支 L22945-22979：output_routine_loc 与 toks 寄存器同族，`{token list}` 之外也接受另一 toks 寄存器/`\output` 内容复制；引擎侧输出例程 token 列表独立存储，读臂在 save.rs `\the\output`），preview L375 `\pr@output\output` 从「RHS 需为 {token list} 或 toks 寄存器」致命 → 载入走通。② `skip_trailing_spaces` 按 tex.web `@<Scan an optional space@>`（L8755）收口：**至多吞一个空格，非空格 token 取一次即放回，不再向前多取**——旧实现"吞光连续空格再取下一个放回"多出的那次取 token 把**赋值执行前的陈旧 catcode 表**烙进后继 token（token 在词法时点绑定 catcode，`\catcode`\@=11 \q@=5 …` 中 `\q@` 被按 @=12 切成 `\q`+`@` → `Missing number`；真 TeX 同文档正常）。回归锁：`output_as_toks_rhs_copies_content`、`optional_space_after_value_consumed_before_following_cs_lex`（去修复即失败：`@=50`）、`preview_addto_front_idiom_matches_pdftex`（pdfTeX GT 对拍 plainx fmt）。**附案**：任务书的 `\toks@\expandafter{\the\expandafter\toks@B}` repro 在 plain 格式下两引擎同报 `Missing number`（`@` 在 plain L1239 后是 cat12，`\toks@` 切成 `\toks`+`@`，GT 一致），filler 循环内 `\expandafter` 语义本就正确（/tmp/ex_faithful.tex NTex "EX: AX" == pdfTeX）。**零回归**：`ntex-trip --driver ntex --test both` 改动前后规范化 diff **0 行**（TRIP/ETRIP 既有基线失败形态不变）；sizes.tex（KOMA）阻塞点与 HEAD 同签名同计数。遗留新登记 §6 两行（`\let\output\pr@output` 委托后 output_toks 陈旧 → preview 0 页出；`\read` 流未打开致命）。
- 2026-09-27：ctex 第三刀收口两根（breadth #1 续，701ba05+60fd110）——① 控制词后空格吸收改惰性 skip_blanks 状态（tex.web L7417；`\ExplSyntaxOn` 下 cat32 翻转让就地吞字节多发空格 token，`\GetIdInfo` 定界实参失配自递归）；② fmt v22 持久化 `\fontdimen`/`\hyphenchar` 覆盖表（l3kernel intarray 模拟字体的条目/count，fmt 生成期创建、此前恢复全 0/45）。**新登记简化**：pass2 `TfmLoader` 对 `at` 规格的单位换算走样——`\c_document_cctab` 的 `\meaning` 显示 `cmr10 at 40pt` vs GT `cmr10 at 0.0004pt`（26sp）；数组身份/读写不受影响（逐字体键控），仅显示与真实字号语义走样，待立题。另：`\protect` 绑定时序假设证伪（mg 探针字节一致），广度 #1 表已更正。
- 2026-09-27（同日收尾）：**TRIP 门禁 154→146 的判读**（fmt v22 连带改善）——基线 154 条 `! ` 签名中 8 条 `Font \smalltrip has only 13 fontdimen parameters.`（trip.tex l.229-236 `\symbolpar` 写参数 14..21）消失，其余零差异且复跑稳定。Knuth 官方 `fixtures/trip/trip.log` 该 8 处**零报错**（全 log 唯一同类错误是 l.404 `\ip` 读路径 1 条，NTex 残余恰 1 条同形）：trip.tex L90-92 的 `\dump` 结构下 font_params 扩容属 fmt 状态，旧快照丢失→pass2 误报，v22 持久化后即消——**GT 更近而非回归**。门禁口径「154 逐字不变」据此判读放行。
- 2026-09-27（同日收尾）：**名字扫描前导空格按字符码 32 盲于 catcode**（`skip_name_leading_blanks`，`scan.rs` 新增；`io.rs scan_file_name` + `primitive_font.rs scan_font_name` 两入口换用）——pdftex GT（/tmp/geGT mg44/45/47/48/49）：`\catcode`\ =12\input q7.tex \end` → `(./q7.tex)` 文件装载、不存在文件报完整名 `nosuchfile.tex`；`\show` 对拍证明 cat-12 空格在普通流仍产出空格 token（mg47 双空格 vs mg48 基线单空格、mg49 证 show 无分隔插入）——吞它**只在名字扫描入口、不在全局 get_next 状态机**。名字循环内 char-32 仍由 `more_name` 终止（消费不回退）。701ba05 惰性化暴露此缺口（旧急切字节消费恰好吞掉该空格）。**行尾空格 token 语义改判**：`input.rs` 测试 `space_then_newline_yields_two_spaces` 期望 4 token 系旧急切语义，tex.web `skip_blanks+car_ret → finish line 不发 token`（L7321-7323）+ GT mg51（`\def\a#1!{<#1>}` 定界实参捕获 `<a b>` 单空格）→ 改期望 3 token，测试更名 `…_yields_one_space`。
- 2026-09-27（同日收尾）：**`\write` 串行化丢弃 cat-8 `_`（cat-7 `^` 同）——此前登记，现已修**。原缺陷：`expand/io.rs expand_to_string`（l.697）序列化只收 `Catcode::Letter|Other` 字符 token，其余落入丢弃臂；csname 身份却保留字符 95 → **二趟 aux 解析 miss**。最小复现（4 行 LaTeX，两趟）：`\cite{foo_bar}`+`\bibitem{foo_bar}` → pass1 aux 写出 `\bibcite{foobar}{1}`（`_` 已吞），pass2 查 `b@foo_bar`（live 侧含 `_`）≠ 读回侧 `b@foobar`（纯字母）→ `Citation 'foobar' undefined`。GT 对拍（plain 探针 /tmp/citep/p.tex）：pdftex 写 `foo_bar`、NTex 写 `foobar`；csname 侧两引擎同为 `b@foo_bar`。**基线 41656fd 同病**（同探针 A/B 实证）；transformer-standalone 的 `neural_gpu` 键即此（tex/bbl 均带 `_`，aux 两行均成 `neuralgpu`、但 live 侧查 `b@neural_gpu`）→ `[?]`=0 验收不可达的既有根因。修复：`expand_to_string` 改为 tex.web `token_show` 语义，所有字符 token 一律按 `charcode` 输出，catcode 只影响扫描/语法；保留 undefined cs 静默丢弃债（§6 表内 `expand_to_string` 行）。对拍：pdfTeX/NTex `\message{a_b}`、`\show\m`（宏体 `a_b`）、`\write` 文件 `W:a_b`、DVI `\special{S:a_b}` 均一致；新增回归 `write_stringification_prints_all_character_catcodes`（含 cat-8 `_` / cat-7 `^` / cat-6 `#` / cat-1/2 `{}`）、`special_stringification_prints_subscript_character`、`bibcite_key_with_subscript_char_roundtrips`。
- 2026-09-27：ctex 第五刀 rescan 机器推进——根因是 `\scantokens` 接了 `dispatch_expandable` 但未进入 `Primitive::is_expandable()` 白名单，`\expandafter\__tl_rescan:NNw ... \tex_scantokens:D { ... }` 链中 `\scantokens` 被当成宏实参数据扫入，expl3 依赖的 `\everyeof` marker 永不到达，触发 `Use of \__tl_rescan:NNw doesn't match its definition` / extra `}`。修复：`Scantokens` 加入可展开原语表，`expand_once` 中执行 `exec_scantokens()`；其 `<general text>` 参数改用展开扫描。最小 rescan 探针组内 `\tl_show:N` 得 `AB`；ctex 原 rescan mismatch 清零，推进到 zhnumber/`\__tl_build_last:NNn` 的 `\edef` 展开递归新墙（`Missing number` ×167，64M 步 guard）。
- 2026-09-28：ctex 第六刀收口 **Missing number ×167 单根**（6e0d745）——上刀指认的「l3keys 定义/取值路径」**证伪**：l3keys 最小探针（`\keys_define:nn {mymod}{foo .tl_set:N=…}`+`\keys_set`）与 `\tl_build` 基础探针全过。真根因是 68e5d55（fp 展开递归 WIP 护栏）把「`suppress_expansion>0` 时 protected 宏不可展开」伸进 scan_int 四个循环（符号/字母常量/基数/十进制），只白名单 `exp_end_continue_f:w|:nw`。**tex.web 口径**：protected 抑制面只盖「构建展开 token 列表」语境（`\edef`/`\write` 吸收循环，`scan_edef_body`/`scan_group_contents_xpand` 的 protected 臂），数值扫描走 get_x_token 无此门。GT pdflatex 决定性证据：`\cs_new_protected:Npn \myp{0}` + `\edef\x{\exp:w \myp Z}` → `\x`=Z 无错误；`\edef\x{\number 2\zz}`（`\zz` protected→4）→ 24。**连锁机制**：l3tl-build 哨兵 `\__tl_build_last:NNn`（protected）在 `\exp:w`（=`\romannumeral`）前瞻里落成不可展开；tl_build 机器初值 4 枚 `\exp_end:`（chardef 0）、每次 `\tl_build_put_right:Nx` 的 romannumeral 消耗一枚，第 5 次 put_right 起数值扫描越过末尾撞哨兵 → 每次一条 Missing number（zhnumber 块共 176 次 put_right 中 167 条，全部同一签名 `<to be read again> \__tl_build_last:NNn`）。**探针法教训**：`\tl_show:N` 输出在 79 列折行，`grep '^>'` 只收首行会把「没截断」看成截断——判内容用 `\int_show:n{\tl_count:N …}`。**验收**：zhnumber 块探针 167→0；ctex 探针 Missing number 167→0 并推进（`zhnumber-utf8.cfg` 载入），新墙=`\ctex_deprecated_command:Nn \CTEXoptions` 处 `\edef` 步数/栈深护栏（6400 万步、栈深 26）；`ntex-trip --driver ntex --test both` 改动前后 diff 仅临时路径 2 行（TRIP「组未闭合 […Align]」/ETRIP 终止形态与基线一致）；Transformer 样张 DVI 逐字节一致（18 页/3 图/`[?]`=0）；ntex-core 472 通过、clippy 绿。**遗留**：`scan_dimen_body`/`scan_keyword` 同族 protected 门未动（本轮探针未及，下一刀一并核 GT）。
- **收尾纪律提醒**：后续每轮修复后同步更新本清单（已修项标 ✅ + commit；维护记录追加）
