use super::*;

impl FontSink for NodeBuilder {
    fn font_selected(&mut self, font: u32) -> Result<()> {
        // fn 指针模式恒为 FontId(0)；TFM 模式更新当前字体
        self.current_font = FontId(font);
        Ok(())
    }
    fn font_defined(&mut self, font: u32, cs_name: &str) -> Result<()> {
        if self.font_cs_names.len() <= font as usize {
            self.font_cs_names.resize(font as usize + 1, None);
        }
        self.font_cs_names[font as usize] = Some(cs_name.to_string());
        Ok(())
    }
    fn hyphen_char_changed(&mut self, font: u32, value: i64) -> Result<()> {
        self.hyphenchars.insert(font, value);
        Ok(())
    }
    fn current_font(&self) -> u32 {
        self.current_font.0
    }
    /// `\spacefactor` 实时查询（活参数：随字符/句号由排版器调整）。
    fn space_factor(&self) -> i64 {
        self.space_factor
    }
    /// `\spacefactor=<number>` 赋值（组作用域恢复由 save/restore 处理）。
    /// TeX 只允许 1..32767：越界 `int_error("Bad space factor")` 报错恢复、不赋值
    /// （tex.web L23220-23228；TRIP L289 `\showbox0\spacefactor=0`——否则后续
    ///  `app_space` 中 `xn_over_d(shrink, 1000, sf)` 除零）。
    fn set_space_factor(&mut self, v: i64) -> Result<()> {
        if !(1..=32767).contains(&v) {
            // 参考 log 格式（int_error 带值 + help 行，TRIP L289）
            let _ = self.write16(format!(
                "! Bad space factor ({}).\n\
                 I allow only values in the range 1..32767 here.\n",
                v
            ));
        } else {
            self.space_factor = v;
        }
        Ok(())
    }
    /// `\/`：斜体校正（水平模式发 kern / 数学模式斜体校正原子 / 垂直模式报错）。
    fn italic_correction(&mut self) -> Result<()> {
        // 数学模式：斜体校正原子（当前无字体斜体校正度量，宽度 0）
        if matches!(self.mode(), Mode::Math | Mode::DisplayMath) {
            return self.math_push_atom(MathAtom::MSkip {
                width: 0,
                stretch: 0,
                shrink: 0,
                stretch_order: 0,
                shrink_order: 0,
                nonscript: false,
                mu: false,
            });
        }
        // 垂直模式：TeX 报 "You can't use `\/' in vertical mode"
        if matches!(self.mode(), Mode::Vertical) {
            return self.math_mode_error("/");
        }
        // 水平/受限水平：斜体校正 kern（无字体度量数据，宽度 0 不输出；TeX 同理）
        self.append(Node::Kern { width: 0 });
        Ok(())
    }
}
