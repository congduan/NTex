// 格式预载 G2(a)：内嵌 plain.tex/hyphen.tex + 启动预载 + `\input` 兜底 VFS。
// 行为式断言走 G0 转录通道（`\message` 展开——undefined cs 静默跳过，不能拿
// 「没炸」当证据，survey §2.4）。字体相关断言无 TeX 安装则跳过。
use super::*;

#[test]
fn preload_off_by_default() {
    // 缺省调用方（TRIP / latex probe / corpus math）必须保持 INITEX 裸表：
    // `\newcount` 未定义（两个未定义 cs 按 \ifx 相等 → UNDEF 分支）。
    let mut ts = Typesetter::with_tfm();
    ts.typeset(r"\message{\ifx\newcount\undefined UNDEF\else DEFINED\fi}")
        .unwrap();
    let t = ts.take_transcript();
    assert!(!ts.preload_plain(), "缺省不预载");
    assert!(t.contains("UNDEF"), "缺省不应有 plain 宏：{t:?}");
}

#[test]
fn preload_runs_before_user_source_and_allocates() {
    // 预载 = 源首行 `\input plain`：plain 的分配机制（\newcount→\countdef）
    // 在用户源前可用；`\message` 全展开 `\the` 取寄存器值作行为式证据。
    let mut ts = Typesetter::with_tfm();
    ts.set_preload_plain(true);
    ts.typeset(r"\newcount\foo \foo=42 \message{N=\the\foo}").unwrap();
    let t = ts.take_transcript();
    assert!(t.contains("Preloading the plain format"), "预载应已跑：{t:?}");
    assert!(t.contains("N=42"), "plain 分配机制应可用：{t:?}");
}

#[test]
fn plain_format_builder_form() {
    let mut ts = Typesetter::with_tfm().plain_format();
    ts.typeset(r"\newdimen\bar \bar=3pt \message{D=\the\bar}").unwrap();
    let t = ts.take_transcript();
    assert!(t.contains("D=3.0pt") || t.contains("D=3pt"), "builder 形同效：{t:?}");
}

#[test]
fn input_plain_resolves_via_embedded_vfs_without_local_files() {
    // 源内 `\input plain`（预载关）+ 空 VFS：内嵌层兜底，hyphen.tex 同源可解
    // （plain.tex:1222 `\input hyphen` 是 survey §2.1 的唯一致命点）。
    let mut ts = Typesetter::with_tfm();
    ts.set_vfs(Box::new(ntex_io::MemVfs::new()));
    ts.use_embedded_format();
    assert!(!ts.preload_plain());
    ts.typeset(r"\input plain \newcount\foo \foo=7 \message{M=\the\foo}")
        .unwrap();
    let t = ts.take_transcript();
    assert!(t.contains("M=7"), "内嵌 plain 应可 \\input：{t:?}");
    assert!(t.contains("hyphenation"), "内嵌 hyphen.tex 应被带出：{t:?}");
}

#[test]
fn local_plain_tex_wins_over_embedded() {
    // 内嵌层只作兜底：本地/宿主 VFS 命中优先（不改变既有搜索语义）。
    let mut vfs = ntex_io::MemVfs::new();
    vfs.insert("plain.tex", r"\def\localprobe{LOCAL}".as_bytes());
    let mut ts = Typesetter::with_tfm();
    ts.set_vfs(Box::new(vfs));
    ts.use_embedded_format();
    ts.typeset(r"\input plain \message{\localprobe}").unwrap();
    let t = ts.take_transcript();
    assert!(t.contains("LOCAL"), "本地 plain.tex 应优先：{t:?}");
    assert!(!t.contains("Preloading the plain format"), "不应落到内嵌层：{t:?}");
}

#[test]
fn embedded_hyphen_patterns_reach_builder_via_event() {
    // hyphen.tex 经 `\input` → `\patterns` 原语 → sink 事件 → NodeBuilder 的
    // 模式表（断词机的真数据源）。4447 条全部入 trie。
    // 注意观察点必须在 sink 存活时：`typeset()` 的 finish 会把 NodeBuilder 摘走。
    let mut ts = Typesetter::with_tfm();
    ts.set_preload_plain(true);
    let fonts = ts.fonts.clone();
    ts.install_font_loader();
    ts.install_builder(NodeBuilder::new(fonts));
    ts.run_plain_preload().unwrap();
    let count = ts
        .expander
        .sink_mut()
        .as_any_mut()
        .downcast_mut::<NodeBuilder>()
        .expect("sink 应为 NodeBuilder")
        .patterns
        .count;
    assert!(count > 4000, "hyphen.tex 模式应已入 trie：{count}");
}

#[test]
fn embedded_hyphen_tex_parses_into_pattern_trie() {
    // 内嵌 hyphen.tex 的模式表能被断词机解析（`\patterns` 事件上游的真源）。
    // 注意喂给 trie 的是 `\patterns{...}` 的**实参**（引擎 sink 事件收到的就是
    // 这段），不是整个文件——整文件含 `\hyphenation{...}` 例外词表与注释。
    //
    // 已知简化（不动本刀领地，登记 KNOWN-SIMPLIFICATIONS / survey §5.bis.g2）：
    // PatternTrie::parse 忽略 `.` 词界标志（hyphen.rs parse_pattern 注释「暂不
    // 参与 trie」），真实表里 `.ach4` / `5hand.` 这类词首/词尾受限模式会被当成
    // 任意位置模式，断点全集因此偏大——toy 表的单测（tests.rs
    // patterns_hyphenates_word_across_lines）不触发，真实表一上就显形。
    let body = HYPHEN_TEX;
    let start = body.find("\\patterns{").expect("hyphen.tex 应含 \\patterns") + "\\patterns{".len();
    let end = start + body[start..].find('}').expect("patterns 块应有闭花括号");
    let trie = crate::hyphen::PatternTrie::parse(&body.as_bytes()[start..end]);
    assert!(trie.count > 4000, "4447 条模式应全部入 trie：{}", trie.count);
}



#[test]
fn preload_plain_newif_region_zero_definition_mismatch() {
    // \newif\ifus@（plain.tex L598）此前报 24 条 "Use of macro doesn't match
    // its definition"：根因在 \csname 名字扫描里 \string 不读 \escapechar
    // （-1 时仍打前导 \），\if@ 的 "if" 定界失配。整个 plain 预载须零此类报错。
    let mut ts = Typesetter::with_tfm();
    ts.set_preload_plain(true);
    ts.typeset(r"\message{AFTER}").unwrap();
    let t = ts.take_transcript();
    let n = t.matches("doesn't match").count();
    assert_eq!(n, 0, "plain 预载不应有宏定义失配：……\n{t}");
}
