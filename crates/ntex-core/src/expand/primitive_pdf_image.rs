// pdfTeX 图片三原语（\includegraphics 图片管线 Step A）+ PDF 变换栈
// （\includegraphics 图片管线 Step B）。
//
// 主题：\pdfximage（读图定自然尺寸 + 登记 xobject + 置 \pdflastximage）、
// \pdflastximage（misc 67）、\pdfrefximage（零墨占位盒 + 图片 special 拼进
// 当前列表）、\pdfsave/\pdfsetmatrix/\pdfrestore（PDF 变换栈，只取对角缩放）。
//
// 为什么是这几者：graphicx 在 PDF 模式驱动（pdftex.def）下，图的自然尺寸
// **只**来自引擎——`\Gread@@pdftex` 执行 `\pdfximage{文件}` 后
// `\setbox\@tempboxa=\hbox{\pdfrefximage\pdflastximage}` 再读 \wd/\ht
// （pdftex.def L296-301）。不实现它们，`\includegraphics` 拿不到任何尺寸。
//
// Step B 口径（位图真嵌入）：
// - 占位盒仍**零墨**且保持**自然尺寸**——`\hbox{\kern W \vrule width 0pt
//   height H depth 0pt}`。保持自然尺寸是硬要求：pdftex.def 的实际包含结构
//   是 `\hbox{\Gscale@box{sx}[sy]{\pdfrefximage..}}`，\Gscale@box 以
//   `\hb@xt@ sx\wd\z@` 定外盒宽——\wd\z@ 必须是自然宽，缩放比才对。
// - 视觉由图片 special 携带：`\pdfrefximage` 在占位盒 token 之前向 sink 发
//   一个 `ntex-image` whatsit（DVI 侧落 xxx 载荷、ntex-pdf 侧读它写 Image
//   XObject）。载荷带**自然尺寸**；锚点 = whatsit 的 DVI 当前点 = 盒参考点
//   = 图左下角（DVI y 向下，图在基线上方）。
// - 缩放（scale=/width=/height= 走 \Gscale@box → \pdfsave\pdfsetmatrix..
//   \pdfrestore）**不能**在 \pdfrefximage 时刻取——\Gscale@box 先排内容盒
//   后开矩阵域，CTM 生效在 `\copy` 重放时。所以三原语只发 `ntex-ctm` 标记
//   whatsit，缩放由 DVI 写出器在标记处维护、对 `ntex-image` 载荷就地乘出
//   显示尺寸（见 ntex-dvi `Writer::special`）。
//
// 维护约定（与 expand/mod.rs 既有 include! 链一致）：
// - 仅 `impl Expander { ... }` 块 + 文件级自由函数，无 use / 无模块声明；
// - 入口由 `primitive.rs` 主 match 委托。

impl Expander {
    /// `\pdfinfo{...}`：pdfTeX 扫一个 general text 组；NTex DVI 路径只保留
    /// stub 标记。GT 对拍（2026-10-04）：组后 token 立即可见。
    fn exec_pdf_info(&mut self) -> Result<()> {
        let _ = self.scan_group_contents(None)?;
        self.sink.special("PDF-PRIMITIVE-STUB pdfinfo".to_owned())
    }

    /// `\pdfcatalog{...}`：同 `\pdfinfo`，吞一个字典组。
    fn exec_pdf_catalog(&mut self) -> Result<()> {
        let _ = self.scan_group_contents(None)?;
        self.sink
            .special("PDF-PRIMITIVE-STUB pdfcatalog".to_owned())
    }

    /// `\pdfcolorstack<n> push{...}|set{...}|pop|current`。
    ///
    /// hyperref/colorlinks 主要走 `push{color}`/`pop`。pdfTeX 允许颜色栈编号后接
    /// 关键字；`push`/`set` 后必须跟一个平衡组，`pop` 无组。这里不维护真实颜色
    /// 栈，只发标记给后端将来选择性消费。
    fn exec_pdf_colorstack(&mut self) -> Result<()> {
        let stack = self.scan_number()?;
        let op = self
            .scan_keyword(|w| matches!(w, "push" | "pop" | "set" | "current"))?
            .unwrap_or_else(|| "push".to_owned());
        if matches!(op.as_str(), "push" | "set") {
            let _ = self.scan_group_contents(None)?;
        }
        self.sink
            .special(format!("PDF-PRIMITIVE-STUB pdfcolorstack {stack} {op}"))
    }

    /// `\pdfdest name{...}|num<n> <view>`。
    ///
    /// GT 对拍：`name{abc} xyz`、`name{abc} fit`、`num 3 xyz`、`fitr`+4 dimen
    /// 都完整吞参后继续。视图坐标只影响 PDF 后端，当前仅消费常见形态。
    fn exec_pdf_dest(&mut self) -> Result<()> {
        self.consume_pdf_name_or_num()?;
        self.consume_pdf_view_spec()?;
        self.sink.special("PDF-PRIMITIVE-STUB pdfdest".to_owned())
    }

    /// `\pdfstartlink [attr{...}] <action>`。
    ///
    /// 这是本族最复杂的文法。GT 对拍（2026-10-04）：水平模式下常见
    /// `attr{...} goto name{...}`、`user{...}`、`goto page <n>{...}` 均在 action
    /// 后把后续 token 留给正文；未知 action 报 "action type missing"。NTex 当前
    /// 只覆盖这些 hyperref/pdftex.def 路径，登记在 KNOWN-SIMPLIFICATIONS。
    fn exec_pdf_startlink(&mut self) -> Result<()> {
        loop {
            if self.scan_keyword(|w| w == "attr")?.is_some() {
                let _ = self.scan_group_contents(None)?;
                continue;
            }
            if self.scan_keyword(|w| w == "width" || w == "height" || w == "depth")?.is_some() {
                let _ = self.scan_dimen()?;
                continue;
            }
            break;
        }
        if self.scan_keyword(|w| w == "user")?.is_some() {
            let _ = self.scan_group_contents(None)?;
        } else if self
            .scan_keyword(|w| matches!(w, "goto" | "thread"))?
            .is_some()
        {
            self.consume_pdf_link_target()?;
        } else {
            return Err(Error::invalid_input("\\pdfstartlink: action type missing"));
        }
        self.sink
            .special("PDF-PRIMITIVE-STUB pdfstartlink".to_owned())
    }

    /// `\pdfendlink`：无参数，结束当前链接矩形。
    fn exec_pdf_endlink(&mut self) -> Result<()> {
        self.sink
            .special("PDF-PRIMITIVE-STUB pdfendlink".to_owned())
    }

    /// `\pdfsave`：变换域开括号。以 `ntex-ctm` 标记 whatsit 落进当前列表——
    /// **不是**引擎侧状态：graphicx 的 \Gscale@box 先排内容盒（`\pdfrefximage`
    /// 在此执行、CTM 尚未生效）后开矩阵域再 `\copy` 重放，所以缩放语义只能
    /// 在**重放侧**（DVI 写出器走到这些标记时）生效，见 ntex-dvi `Writer::special`。
    fn exec_pdf_save(&mut self) -> Result<()> {
        self.sink.special("ntex-ctm push".to_owned())
    }

    /// `\pdfsetmatrix{a b c d}`：矩阵乘入当前变换域（矩阵项是小数字面量，
    /// ⟨general text⟩ 展开后取串解析）。DVI 无矩阵，只保留对角缩放分量
    /// （a→sx、d→sy；pdftex.def 实际只发 `{sx 0 0 sy}`；angle= 的旋转矩阵
    /// 退化为 cos 缩放是已记录的限制）。
    fn exec_pdf_setmatrix(&mut self) -> Result<()> {
        let toks = self.scan_group_contents(None)?;
        let text = self.expand_to_string(&toks)?;
        let nums: Vec<&str> = text.split_whitespace().take(4).collect();
        let mut m = [1.0f64; 4];
        for (i, tok) in nums.iter().enumerate() {
            if let Ok(v) = tok.parse::<f64>() {
                m[i] = v;
            }
        }
        self.sink
            .special(format!("ntex-ctm matrix {} {} {} {}", m[0], m[1], m[2], m[3]))
    }

    /// `\pdfrestore`：变换域闭括号（失衡保底由 DVI 写出器负责）。
    fn exec_pdf_restore(&mut self) -> Result<()> {
        self.sink.special("ntex-ctm pop".to_owned())
    }

    /// `\pdfximage⟨attr{..}⟩⟨page n⟩⟨页面盒词⟩{文件}`（pdfTeX §图片包含）。
    ///
    /// 前缀关键字按 pdfTeX 1.40 语法扫描（pdftex.def 实际只发
    /// `attr{..}`/`page ⟨n⟩`/`cropbox` 族词）：attr 的组内容 Step A 不解读、
    /// page 与页面盒词只消费——它们影响的是真嵌入（Step B）的子页选择。
    fn exec_pdf_ximage(&mut self) -> Result<()> {
        loop {
            if self.scan_keyword(|w| w == "attr")?.is_some() {
                // `attr{/Decode[..] /Group<</S/Transparency..>>}`：整组吃掉。
                let _ = self.scan_group_contents(None)?;
                continue;
            }
            if self.scan_keyword(|w| w == "page")?.is_some() {
                let _ = self.scan_number()?;
                continue;
            }
            if self
                .scan_keyword(|w| {
                    matches!(
                        w,
                        "mediabox" | "cropbox" | "artbox" | "trimbox" | "boundingbox" | "none"
                    )
                })?
                .is_some()
            {
                continue;
            }
            break;
        }
        // 文件名：花括号组内容**展开后**取串（pdftex.def 传 `\Gin@base\Gin@ext`，
        // 扫描组时是两个未展开 cs）。
        let toks = self.scan_group_contents(None)?;
        let name = self.expand_to_string(&toks)?;
        let data = self
            .vfs
            .read(&name)
            .map_err(|e| Error::io("VFS 读取", name.clone(), e))?
            .ok_or_else(|| {
                Error::invalid_input(format!("\\pdfximage: file not found: `{name}'"))
            })?;
        let Some((w, h)) = image_natural_size(&data) else {
            return Err(Error::invalid_input(format!(
                "\\pdfximage: 无法识别图片尺寸（PNG IHDR / PDF /MediaBox / EPS %%BoundingBox）：`{name}'"
            )));
        };
        self.pdf_xobjects.push((w, h, name));
        self.params.misc[crate::param::MISC_PDF_LAST_XIMAGE] = self.pdf_xobjects.len() as i64;
        Ok(())
    }

    /// `\pdfrefximage⟨id⟩`：把 xobject `id` 的占位盒 + 图片 special 拼进当前列表。
    ///
    /// 两个产物，顺序敏感：
    /// 1. `ntex-image` whatsit（Step B 载荷）：先发——它进当前列表时 DVI 当前点
    ///    恰是后续占位盒的参考点（= 图左下角）。载荷
    ///    `ntex-image <自然宽 sp> <自然高 sp> <名字节数> <名>`；**显示尺寸**
    ///    （scale= 等缩放后）由 DVI 写出器乘上当前变换域（`ntex-ctm` 标记，
    ///    见 exec_pdf_save 注释与 ntex-dvi `Writer::special`）。
    /// 2. `\hbox{\kern Wsp \vrule width 0pt height Hsp depth 0pt}` 占位盒
    ///    回灌输入执行——**自然尺寸**（\Gscale@box 以 sx\wd\z@ 定外盒宽，
    ///    \wd\z@ 必须自然宽）。宽=图宽（kern 贡献，不引胶水故无 Underfull）、
    ///    高=图高（零宽 rule 的竖直贡献）、深=0。
    fn exec_pdf_ref_ximage(&mut self) -> Result<()> {
        let id = self.scan_number()?;
        // id 从 1 起（0 = 无图，pdfTeX 同口径）；越界/0 → pdfTeX 同文报错
        let dims = if id >= 1 {
            self.pdf_xobjects.get((id - 1) as usize)
        } else {
            None
        };
        let Some(&(w, h, _)) = dims else {
            return Err(Error::invalid_input(format!(
                "\\pdfrefximage: invalid image id `{id}'"
            )));
        };
        let name = dims
            .map(|(_, _, n)| n.clone())
            .unwrap_or_default();
        // Step B 载荷：先于占位盒进列表（当前点 = 图锚点）。名字长度前缀
        // 防文件名含空格断词。
        self.sink
            .special(format!("ntex-image {w} {h} {} {name}", name.len()))?;
        let mut toks: Vec<(Token, bool)> = Vec::with_capacity(24);
        let mut cs = |name: &str| (Token::control_sequence(self.intern.intern(name)), false);
        let word = |s: &str, out: &mut Vec<(Token, bool)>| {
            for ch in s.chars() {
                out.push((Token::char(Catcode::Other, ch as u32), false));
            }
        };
        let dimen = |v: i64, out: &mut Vec<(Token, bool)>| {
            for ch in format!("{v}sp").chars() {
                out.push((Token::char(Catcode::Other, ch as u32), false));
            }
        };
        toks.push(cs("hbox"));
        toks.push((Token::char(Catcode::BeginGroup, b'{' as u32), false));
        toks.push(cs("kern"));
        dimen(w, &mut toks);
        toks.push(cs("vrule"));
        word("width", &mut toks);
        dimen(0, &mut toks);
        word("height", &mut toks);
        dimen(h, &mut toks);
        word("depth", &mut toks);
        dimen(0, &mut toks);
        toks.push((Token::char(Catcode::EndGroup, b'}' as u32), false));
        self.push_frame(InputFrame::TokenList {
            items: toks.into(),
            pos: 0,
        });
        Ok(())
    }

    fn consume_pdf_name_or_num(&mut self) -> Result<()> {
        if self.scan_keyword(|w| w == "name")?.is_some() {
            let _ = self.scan_group_contents(None)?;
            Ok(())
        } else if self.scan_keyword(|w| w == "num")?.is_some() {
            let _ = self.scan_number()?;
            Ok(())
        } else {
            Err(Error::invalid_input("\\pdf primitive: expected name or num"))
        }
    }

    fn consume_pdf_link_target(&mut self) -> Result<()> {
        if self.scan_keyword(|w| w == "file")?.is_some() {
            let _ = self.scan_group_contents(None)?;
        }
        if self.scan_keyword(|w| w == "name")?.is_some() {
            let _ = self.scan_group_contents(None)?;
        } else if self.scan_keyword(|w| w == "num")?.is_some() {
            let _ = self.scan_number()?;
        } else if self.scan_keyword(|w| w == "page")?.is_some() {
            let _ = self.scan_number()?;
            if self.next_token_is_begin_group()? {
                let _ = self.scan_group_contents(None)?;
            }
        } else {
            return Err(Error::invalid_input("\\pdfstartlink: target missing"));
        }
        Ok(())
    }

    fn consume_pdf_view_spec(&mut self) -> Result<()> {
        if self.scan_keyword(|w| w == "fitr")?.is_some() {
            for _ in 0..4 {
                let _ = self.scan_dimen()?;
            }
            return Ok(());
        }
        if self
            .scan_keyword(|w| matches!(w, "fit" | "fith" | "fitv" | "fitb" | "fitbh" | "fitbv"))?
            .is_some()
        {
            return Ok(());
        }
        if self.scan_keyword(|w| w == "xyz")?.is_some() {
            // hyperref 的 anchor 路径常为裸 `xyz`。坐标三元组（数字/null）后续补。
            return Ok(());
        }
        Ok(())
    }

    fn next_token_is_begin_group(&mut self) -> Result<bool> {
        self.skip_spaces()?;
        let Some((tok, _)) = self.fetch()? else {
            return Ok(false);
        };
        let is_group = self.resolve_group_char(tok).catcode() == Some(Catcode::BeginGroup);
        self.unread(tok);
        Ok(is_group)
    }
}

/// 图片自然尺寸（sp）：按魔数分派 PNG / PDF / EPS，其余 None。
fn image_natural_size(data: &[u8]) -> Option<(i64, i64)> {
    if data.len() >= 8 && data[..8] == [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A] {
        return png_natural_size(data);
    }
    if data.len() >= 5 && &data[..5] == b"%PDF-" {
        return pdf_natural_size(data);
    }
    if data.starts_with(b"%!PS") {
        return eps_natural_size(data);
    }
    None
}

/// PNG：IHDR 宽高（px）× pHYs 密度（unit=1 → 像素/米）。无 pHYs → 72dpi
/// （1px = 1bp，pdfTeX 同口径）。
///
/// px → bp：`px * 72 / dpi`；bp → sp：65781.76（1bp = 1/72 in，
/// 1 in = 72.27 pt × 65536 sp/pt）。GT 对照：ModalNet-21 1520×2239 @11811px/m
/// → 366.168pt × 539.375pt，与 pdfTeX log 逐位一致。
fn png_natural_size(data: &[u8]) -> Option<(i64, i64)> {
    // IHDR 必为首个 chunk（偏移 8：len(4)+type(4) 后是宽高各 4 字节 BE）。
    if data.len() < 24 || &data[12..16] != b"IHDR" {
        return None;
    }
    let w_px = u32::from_be_bytes([data[16], data[17], data[18], data[19]]) as f64;
    let h_px = u32::from_be_bytes([data[20], data[21], data[22], data[23]]) as f64;
    // pHYs 在 IHDR 之后顺序不定，逐 chunk 线性找（chunk ≤ 数十个，无需全表）。
    let mut pos = 8usize;
    let mut dpi = 72.0f64;
    while pos + 8 <= data.len() {
        let len = u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]])
            as usize;
        let typ = &data[pos + 4..pos + 8];
        if typ == b"pHYs" {
            if pos + 8 + 9 <= data.len() {
                let ppu_x = u32::from_be_bytes([
                    data[pos + 8],
                    data[pos + 9],
                    data[pos + 10],
                    data[pos + 11],
                ]) as f64;
                let unit = data[pos + 16];
                // unit=1 → 每米像素数（pdfTeX 同判）；unit=0 → 无定义，用 72dpi。
                if unit == 1 && ppu_x > 0.0 {
                    dpi = ppu_x * 0.0254;
                }
            }
            break;
        }
        if typ == b"IDAT" || typ == b"IEND" {
            break;
        }
        pos += 12 + len; // len + type + data + crc
    }
    Some((px_to_sp(w_px, dpi), px_to_sp(h_px, dpi)))
}

fn px_to_sp(px: f64, dpi: f64) -> i64 {
    (px * 72.0 / dpi * 65781.76).round() as i64
}

/// PDF：`/MediaBox [ llx lly urx ury ]`（bp）——浅扫描：在原始字节里找
/// `/MediaBox` 再取方括号内四个实数。Step A 不做 xref/对象图（不需要完整
/// PDF 解析器；MediaBox 直接量在 PDF 1.x 页树节点里是明文）。
fn pdf_natural_size(data: &[u8]) -> Option<(i64, i64)> {
    let mut from = 0usize;
    loop {
        let rel = data[from..].windows(9).position(|w| w == b"/MediaBox")?;
        let at = from + rel;
        let open = data[at..].iter().position(|&b| b == b'[')? + at;
        let close = data[open..].iter().position(|&b| b == b']')? + open;
        let nums = numbers(&data[open + 1..close]);
        if nums.len() == 4 {
            // 无负尺寸：w = urx-llx, h = ury-lly（bp → sp）
            let w = nums[2] - nums[0];
            let h = nums[3] - nums[1];
            if w > 0.0 && h > 0.0 {
                return Some((bp_to_sp(w), bp_to_sp(h)));
            }
        }
        // 形如 `<< /MediaBox [ 0 0 0 0 ] >>` 的占位（被 CropBox 覆盖）→ 找下一处
        from = close + 1;
        if from >= data.len() {
            return None;
        }
    }
}

/// EPS：`%%BoundingBox: llx lly urx ury`（整数 bp；HiRes 行是实数 bp，优先）。
/// 按 DSC 约定必须在注释头（前 64KB）——浅扫描即取首个匹配行。
fn eps_natural_size(data: &[u8]) -> Option<(i64, i64)> {
    let hi = bbox_line(data, b"%%HiResBoundingBox:");
    let lo = bbox_line(data, b"%%BoundingBox:");
    let line = hi.or(lo)?;
    let nums = numbers(line);
    if nums.len() == 4 {
        let w = nums[2] - nums[0];
        let h = nums[3] - nums[1];
        if w > 0.0 && h > 0.0 {
            return Some((bp_to_sp(w), bp_to_sp(h)));
        }
    }
    None
}

/// 在数据里找以 `key` 开头的行，返回行内 key 之后的部分。
fn bbox_line<'a>(data: &'a [u8], key: &[u8]) -> Option<&'a [u8]> {
    let at = data.windows(key.len()).position(|w| w == key)?;
    let rest = &data[at + key.len()..];
    let end = rest.iter().position(|&b| b == b'\n').unwrap_or(rest.len());
    Some(&rest[..end])
}

/// 行内空白分隔的实数序列（容错：跳过非数字字符——EPS 行尾可能有 \r）。
fn numbers(buf: &[u8]) -> Vec<f64> {
    let mut out = Vec::new();
    for tok in buf.split(|&b| b.is_ascii_whitespace()).filter(|s| !s.is_empty()) {
        if let Ok(v) = std::str::from_utf8(tok).unwrap_or("").parse::<f64>() {
            out.push(v);
        }
        if out.len() == 4 {
            break;
        }
    }
    out
}

fn bp_to_sp(bp: f64) -> i64 {
    (bp * 65781.76).round() as i64
}
