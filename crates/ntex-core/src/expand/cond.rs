impl Expander {
    // ---------- M1-9 条件 ----------

    fn cond_op(&self, tok: Token) -> Option<CondOp> {
        let csid = tok.csid()?;
        match self.eqtb.slot(csid) {
            EqSlot::Primitive(p) => CondOp::from_prim(*p),
            _ => None,
        }
    }

    /// 是否处于跳过模式（最内层条件帧在跳过）。
    fn is_skipping(&self) -> bool {
        self.cond_stack
            .last()
            .is_some_and(|f| f.state == CondState::Skipping)
    }

    /// 条件状态机单步推进。
    fn step_conditional(&mut self, op: CondOp) -> Result<()> {
        match op {
            CondOp::Fi => {
                if self.cond_stack.pop().is_none() {
                    return Err(Error::invalid_input("多余的 \\fi"));
                }
                Ok(())
            }
            CondOp::Else => {
                let Some(top) = self.cond_stack.last_mut() else {
                    return Err(Error::invalid_input("多余的 \\else"));
                };
                if top.else_seen {
                    return Err(Error::invalid_input("多余的 \\else"));
                }
                top.else_seen = true;
                match top.state {
                    CondState::Skipping => {
                        if top.owns_skip {
                            top.state = CondState::Processing;
                        }
                    }
                    CondState::Processing => {
                        top.state = CondState::Skipping;
                        top.owns_skip = false;
                    }
                }
                Ok(())
            }
            CondOp::Or => {
                let Some(top) = self.cond_stack.last_mut() else {
                    return Err(Error::invalid_input("多余的 \\or"));
                };
                if !top.is_case {
                    return Err(Error::invalid_input("多余的 \\or"));
                }
                match top.state {
                    CondState::Skipping => {
                        if let Some(k) = top.ors_left {
                            if k == 1 {
                                top.state = CondState::Processing;
                                top.ors_left = None;
                            } else {
                                top.ors_left = Some(k - 1);
                            }
                        }
                    }
                    CondState::Processing => {
                        top.state = CondState::Skipping;
                        top.owns_skip = false;
                    }
                }
                Ok(())
            }
            CondOp::If
            | CondOp::IfCat
            | CondOp::IfNum
            | CondOp::IfDim
            | CondOp::IfX
            | CondOp::IfOdd
            | CondOp::IfTrue
            | CondOp::IfFalse
            | CondOp::IfDefined
            | CondOp::IfCsname
            | CondOp::IfPrimitive => {
                if self.is_skipping() {
                    // 惰性：不评估测试，仅计数（未走的分支中的宏不被展开）
                    self.cond_stack.push(CondFrame {
                        is_case: false,
                        state: CondState::Skipping,
                        owns_skip: false,
                        ors_left: None,
                        else_seen: false,
                    });
                    return Ok(());
                }
                let mut truth = self.evaluate_if(op)?;
                // e-TeX（M4-5）：`\unless` 取反下一个条件
                if self.unless_pending {
                    self.unless_pending = false;
                    truth = !truth;
                }
                self.cond_stack.push(CondFrame {
                    is_case: false,
                    state: if truth {
                        CondState::Processing
                    } else {
                        CondState::Skipping
                    },
                    owns_skip: !truth,
                    ors_left: None,
                    else_seen: false,
                });
                Ok(())
            }
            CondOp::IfCase => {
                if self.is_skipping() {
                    self.cond_stack.push(CondFrame {
                        is_case: true,
                        state: CondState::Skipping,
                        owns_skip: false,
                        ors_left: None,
                        else_seen: false,
                    });
                    return Ok(());
                }
                let n = self.scan_number()?;
                if n < 0 {
                    return Err(Error::invalid_input("\\ifcase 序号不能为负"));
                }
                self.cond_stack.push(CondFrame {
                    is_case: true,
                    state: if n == 0 {
                        CondState::Processing
                    } else {
                        CondState::Skipping
                    },
                    owns_skip: n > 0,
                    ors_left: (n > 0).then_some(n as usize),
                    else_seen: false,
                });
                Ok(())
            }
        }
    }

    /// 评估条件测试（跳过模式下不会被调用）。
    fn evaluate_if(&mut self, op: CondOp) -> Result<bool> {
        match op {
            CondOp::IfTrue => Ok(true),
            CondOp::IfFalse => Ok(false),
            CondOp::If => {
                let t1 = self
                    .fetch()?
                    .ok_or_else(|| Error::invalid_input("\\if 缺操作数"))?
                    .0;
                let t2 = self
                    .fetch()?
                    .ok_or_else(|| Error::invalid_input("\\if 缺操作数"))?
                    .0;
                Ok(matches!(t1.kind(), TokenKind::Char)
                    && matches!(t2.kind(), TokenKind::Char)
                    && t1 == t2)
            }
            CondOp::IfCat => {
                let t1 = self
                    .fetch()?
                    .ok_or_else(|| Error::invalid_input("\\ifcat 缺操作数"))?
                    .0;
                let t2 = self
                    .fetch()?
                    .ok_or_else(|| Error::invalid_input("\\ifcat 缺操作数"))?
                    .0;
                Ok(t1.catcode() == t2.catcode())
            }
            CondOp::IfX => {
                let t1 = self
                    .fetch()?
                    .ok_or_else(|| Error::invalid_input("\\ifx 缺操作数"))?
                    .0;
                let t2 = self
                    .fetch()?
                    .ok_or_else(|| Error::invalid_input("\\ifx 缺操作数"))?
                    .0;
                Ok(self.ifx_equal(t1, t2))
            }
            CondOp::IfNum => {
                let a = self.scan_number()?;
                let rel = self.scan_relation()?;
                let b = self.scan_number()?;
                Ok(compare(a, b, rel))
            }
            CondOp::IfDim => {
                let a = self.scan_dimen()?;
                let rel = self.scan_relation()?;
                let b = self.scan_dimen()?;
                Ok(compare(a, b, rel))
            }
            CondOp::IfOdd => Ok(self.scan_number()? % 2 != 0),
            // e-TeX（M4-5）
            CondOp::IfDefined => {
                let tok = self
                    .fetch()?
                    .ok_or_else(|| Error::invalid_input("\\ifdefined 缺操作数"))?
                    .0;
                match tok.kind() {
                    TokenKind::ControlSeq => {
                        let csid = tok.csid().expect("ControlSeq 必有 csid");
                        Ok(!matches!(self.eqtb.slot(csid), EqSlot::Undefined))
                    }
                    _ => Ok(false),
                }
            }
            CondOp::IfCsname => {
                let name = self.scan_csname()?;
                Ok(matches!(
                    self.intern.lookup(&name),
                    Some(id) if !matches!(self.eqtb.slot(id), EqSlot::Undefined)
                ))
            }
            // e-TeX（M4-5）：\ifprimitive <cs> —— cs 的 eqtb 槽是内建原语
            CondOp::IfPrimitive => {
                let tok = self
                    .fetch()?
                    .ok_or_else(|| Error::invalid_input("\\ifprimitive 缺操作数"))?
                    .0;
                match tok.kind() {
                    TokenKind::ControlSeq => {
                        let csid = tok.csid().expect("ControlSeq 必有 csid");
                        Ok(matches!(self.eqtb.slot(csid), EqSlot::Primitive(_)))
                    }
                    _ => Ok(false),
                }
            }
            CondOp::IfCase | CondOp::Else | CondOp::Fi | CondOp::Or => {
                unreachable!("step_conditional 已分流")
            }
        }
    }

    /// `\ifx`：字符按 (catcode,char)；控制序列按含义（解析别名）；其余 false。
    fn ifx_equal(&self, t1: Token, t2: Token) -> bool {
        match (t1.kind(), t2.kind()) {
            (TokenKind::Char, TokenKind::Char) => t1 == t2,
            (TokenKind::ControlSeq, TokenKind::ControlSeq) => {
                self.meaning_key(t1.csid().expect("ControlSeq 必有 csid"))
                    == self.meaning_key(t2.csid().expect("ControlSeq 必有 csid"))
            }
            _ => false,
        }
    }

    /// 控制序列的含义键（沿 Alias 链解析）。
    fn meaning_key(&self, csid: u32) -> MeaningKey {
        let mut id = csid;
        let mut hops = 0usize;
        while let EqSlot::Alias(target) = self.eqtb.slot(id) {
            id = *target;
            hops += 1;
            if hops > 64 {
                return MeaningKey::Alias(id);
            }
        }
        match self.eqtb.slot(id).clone() {
            EqSlot::Undefined => MeaningKey::Undefined,
            EqSlot::Macro(m) => MeaningKey::Macro(m.value),
            EqSlot::Primitive(p) => MeaningKey::Primitive(p),
            EqSlot::Char { catcode, charcode } => MeaningKey::Char { catcode, charcode },
            EqSlot::Alias(t) => MeaningKey::Alias(t),
            EqSlot::Font(font) => MeaningKey::Font(font),
            EqSlot::Register(k, n) => MeaningKey::Register(k, n),
            EqSlot::Stream(k, n) => MeaningKey::Stream(k, n),
        }
    }

    fn scan_relation(&mut self) -> Result<Relation> {
        self.skip_spaces()?;
        let tok = self
            .fetch()?
            .ok_or_else(|| Error::invalid_input("关系符扫描到输入末尾"))?
            .0;
        match tok.charcode() {
            Some(c) if c == b'<' as u32 => Ok(Relation::Lt),
            Some(c) if c == b'=' as u32 => Ok(Relation::Eq),
            Some(c) if c == b'>' as u32 => Ok(Relation::Gt),
            _ => Err(Error::invalid_input("预期 < = > 关系符")),
        }
    }

}
