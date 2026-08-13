//! eqtb（等价表）槽：控制序列的运行时含义（RFC-1 §5）。
//!
//! 设计要点：
//! - token 只含 csid，等价关系一律查 eqtb 槽；
//! - 宏定义槽版本化（[`Versioned`]），供 M5 增量依赖追踪；
//! - `\let` 走 [`EqSlot::Alias`] 间接（不复制宏体）。

use std::sync::Arc;

use crate::catcode::Catcode;
use crate::macrodef::MacroDef;
use crate::version::{Version, Versioned};

/// M1 最小原语集（随里程碑扩充）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Primitive {
    Def = 1,
    Edef,
    Gdef,
    Let,
    Relax,
    Expandafter,
    Noexpand,
    Catcode,
    End,
}

impl Primitive {
    /// 参与展开的原语（其余为不可展开，直接执行）。
    pub fn is_expandable(self) -> bool {
        matches!(self, Self::Expandafter | Self::Noexpand)
    }
}

/// 控制序列的等价槽。
#[derive(Debug, Clone, PartialEq)]
pub enum EqSlot {
    /// 未定义。
    Undefined,
    /// 宏定义（版本化）。
    Macro(Versioned<Arc<MacroDef>>),
    /// 内建原语。
    Primitive(Primitive),
    /// `\let` 别名（cs → cs）。
    Alias(u32),
    /// `\let` 到字符（cs 等价于某字符 token，保留其 catcode）。
    Char { catcode: Catcode, charcode: u32 },
}

/// eqtb：按 csid 索引的等价槽数组。
#[derive(Debug, Clone, Default)]
pub struct Eqtb {
    slots: Vec<EqSlot>,
}

impl Eqtb {
    pub fn new() -> Self {
        Self::default()
    }

    /// 确保 csid 槽存在（自动扩展为 `Undefined`）。
    fn ensure(&mut self, csid: u32) {
        while (self.slots.len() as u32) <= csid {
            self.slots.push(EqSlot::Undefined);
        }
    }

    /// 读取槽（越界视为未定义）。
    pub fn slot(&self, csid: u32) -> &EqSlot {
        self.slots.get(csid as usize).unwrap_or(&EqSlot::Undefined)
    }

    /// 可变槽（自动扩展）。
    pub fn slot_mut(&mut self, csid: u32) -> &mut EqSlot {
        self.ensure(csid);
        &mut self.slots[csid as usize]
    }

    /// 定义宏：已有宏槽则 bump 版本（保持版本单调），否则新建。
    pub fn define_macro(&mut self, csid: u32, def: MacroDef) {
        let slot = self.slot_mut(csid);
        match slot {
            EqSlot::Macro(v) => v.bump(Arc::new(def)),
            _ => *slot = EqSlot::Macro(Versioned::new(Arc::new(def))),
        }
    }

    /// 注册原语。
    pub fn set_primitive(&mut self, csid: u32, prim: Primitive) {
        *self.slot_mut(csid) = EqSlot::Primitive(prim);
    }

    /// `\let\cs\other`：cs → Alias(other)。
    pub fn alias(&mut self, csid: u32, target: u32) {
        *self.slot_mut(csid) = EqSlot::Alias(target);
    }

    /// `\let\cs=x`：cs 等价于字符 token。
    pub fn char_alias(&mut self, csid: u32, catcode: Catcode, charcode: u32) {
        *self.slot_mut(csid) = EqSlot::Char { catcode, charcode };
    }

    /// 槽版本（供 M5 依赖追踪；Undefined 返回 None）。
    pub fn version_of(&self, csid: u32) -> Option<Version> {
        match self.slot(csid) {
            EqSlot::Macro(v) => Some(v.version),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catcode::Catcode;
    use crate::token::Token;

    fn tiny_def() -> MacroDef {
        MacroDef {
            params: crate::macrodef::ParamSpec {
                num_params: 0,
                long: false,
                delimiter: None,
            },
            body: Arc::from([Token::char(Catcode::Letter, b'A' as u32)]),
        }
    }

    #[test]
    fn undefined_by_default() {
        let e = Eqtb::new();
        assert_eq!(e.slot(100), &EqSlot::Undefined);
    }

    #[test]
    fn define_macro_bumps_version() {
        let mut e = Eqtb::new();
        e.define_macro(1, tiny_def());
        let v1 = e.version_of(1).unwrap();
        e.define_macro(1, tiny_def());
        let v2 = e.version_of(1).unwrap();
        assert!(v2 > v1);
    }

    #[test]
    fn alias_redirects() {
        let mut e = Eqtb::new();
        e.alias(2, 3);
        assert_eq!(e.slot(2), &EqSlot::Alias(3));
    }

    #[test]
    fn char_alias_stores_catcode() {
        let mut e = Eqtb::new();
        e.char_alias(4, Catcode::Other, b'x' as u32);
        assert_eq!(
            e.slot(4),
            &EqSlot::Char {
                catcode: Catcode::Other,
                charcode: b'x' as u32
            }
        );
    }

    #[test]
    fn ensure_grows_to_csid() {
        let mut e = Eqtb::new();
        e.set_primitive(50, Primitive::Relax);
        assert_eq!(e.slot(50), &EqSlot::Primitive(Primitive::Relax));
        // 中间槽保持 Undefined
        assert_eq!(e.slot(49), &EqSlot::Undefined);
    }
}
