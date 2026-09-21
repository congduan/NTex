// pdfTeX 图片三原语（\includegraphics 图片管线 Step A）。
//
// 主题：\pdfximage（读图定自然尺寸 + 登记 xobject + 置 \pdflastximage）、
// \pdflastximage（misc 67）、\pdfrefximage（零墨占位盒拼进当前列表）。
//
// 为什么是这三者：graphicx 在 PDF 模式驱动（pdftex.def）下，图的自然尺寸
// **只**来自引擎——`\Gread@@pdftex` 执行 `\pdfximage{文件}` 后
// `\setbox\@tempboxa=\hbox{\pdfrefximage\pdflastximage}` 再读 \wd/\ht
// （pdftex.def L296-301）。不实现它们，`\includegraphics` 拿不到任何尺寸。
//
// Step A 口径：DVI 无位图通路，占位盒**零墨**——`\hbox{\kern W \vrule
// width 0pt height H depth 0pt}`：宽=图宽（kern 贡献，不引胶水故无
// Underfull）、高=图高（零宽 rule 的竖直贡献，DVI 不落墨）、深=0。
// 版面占位与 pdfTeX 一致；视觉留白待 Step B 位图（XObject）真嵌入替换。
//
// 维护约定（与 expand/mod.rs 既有 include! 链一致）：
// - 仅 `impl Expander { ... }` 块 + 文件级自由函数，无 use / 无模块声明；
// - 入口由 `primitive.rs` 主 match 委托。

impl Expander {
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

    /// `\pdfrefximage⟨id⟩`：把 xobject `id` 的占位盒拼进当前列表。
    ///
    /// 合成 `\hbox{\kern Wsp \vrule width 0pt height Hsp depth 0pt}` 回灌输入
    /// 执行——与手写 `\hbox{..}` 走完全相同的分组/规则/打包路径（不绕过
    /// sink 的盒生命周期）。尺寸以整数 `sp` 合成：无浮点圆整损失。
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
