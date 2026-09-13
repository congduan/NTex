//! 宏定义（RFC-1 §5）：连续不可变 TokenArray + 参数规格。
//!
//! 宏体采用连续内存（非 TeX 的链表），支持 O(1) 随机访问与整段共享；
//! M2 起宏定义携带预编译字节码（[`Bytecode`]，双表示，双轨等价验证）。

use crate::bytecode::Bytecode;
use crate::token::Token;
use std::sync::Arc;

/// 不可变 token 数组（宏体、实参、token 列表的载体）。
pub type TokenArray = Arc<[Token]>;

/// 宏参数规格。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParamSpec {
    /// 参数个数（0..=9）。
    pub num_params: u8,
    /// `\long`：允许实参含 `\par`。
    pub long: bool,
    /// 参数文本（M1-8）：csname 之后、`{` 之前的所有 token，含 `#n` 参数 token
    /// 与分隔定界符。实参收集时按 `P_1 #1 P_2 #2 ... P_n P_{n+1}` 分段匹配：
    /// P_1 须在调用开头匹配；`#k` 的定界符为 P_{k+1}（空 = 无分隔参数）。
    pub text: TokenArray,
}

/// 宏定义。
///
/// `code` 为预编译字节码（M2）。`PartialEq` 只比较参数规格与宏体
/// （语义相等），不比较编译产物——保证 `\ifx` 语义与双轨切换无关。
#[derive(Debug, Clone)]
pub struct MacroDef {
    pub params: ParamSpec,
    pub body: TokenArray,
    /// 预编译字节码（解释器轨道为 `None`）。
    pub code: Option<Arc<Bytecode>>,
    /// e-TeX `\protected`：在 `\edef`/`\write`/`\detokenize` 等展开上下文不展开。
    pub protected: bool,
    /// `\outer`：禁止出现在宏实参 / `\edef` / general text / `\read` 的 token 列表
    /// 中（tex.web：outer 宏只能在正常展开上下文使用）。
    pub outer: bool,
    /// 定义目标是 **active char token**（`\def^^L{…}`）还是控制序列 token
    /// （`\def\^^L{…}`）。
    ///
    /// tex.web 有两个独立 eqtb 区：active char c 在 `active_base+c`，单字符 cs
    /// chr(c) 在 `single_base+c`（L242 `single_base=active_base+256`）——同名但
    /// **互不可见**。plain.tex L20 `\outer\def^^L{\par}` 只把 outer 写进 active
    /// 区；cs 槽 `\^^L` 保持 undefined。expl3-code.tex L9320 依赖这一点：
    /// `\char_set_catcode_active:N \^^L` 取 cs 形式实参、`` `\^^L `` 取字符码，
    /// 全程不触碰 outer 槽（pdfTeX ground truth：`\h\^^L` → "Undefined control
    /// sequence"，`^^L` 作实参才 Forbidden）。
    ///
    /// 本引擎 csid 是 InternTable 下标、无 active 区（token.rs CS_ACTIVE_FLAG
    /// 只在表示层），两种形式**共享一个槽**——用此位记录「写进的是哪个 tex.web
    /// 槽」，outer 判据按「token 形式 ↔ 槽形式一致」匹配（`is_outer_for_token`），
    /// 否则 cs 形式 `\^^L` 会继承 plain 写进 active 槽的 outer → expl3 载入
    /// Forbidden 级联（187/187 STACK 的首错，2026-09-13 定性）。
    pub active_slot: bool,
}

impl PartialEq for MacroDef {
    fn eq(&self, other: &Self) -> bool {
        self.params == other.params && self.body == other.body
    }
}

impl Eq for MacroDef {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catcode::Catcode;
    use crate::token::Token;

    #[test]
    fn macro_def_holds_params_and_body() {
        let body: TokenArray = Arc::from([Token::char(Catcode::Letter, b'A' as u32)]);
        let def = MacroDef {
            params: ParamSpec {
                num_params: 1,
                long: true,
                text: TokenArray::default(),
            },
            body: body.clone(),
            code: None,
            protected: false,
            outer: false,
            active_slot: false,
        };
        assert_eq!(def.params.num_params, 1);
        assert!(def.params.long);
        assert_eq!(def.body.len(), 1);
        // Arc 共享：克隆不拷贝数据
        let clone = def.body.clone();
        assert_eq!(clone, body);
    }

    #[test]
    fn macro_def_equality_ignores_code() {
        let body: TokenArray = Arc::from([Token::char(Catcode::Letter, b'A' as u32)]);
        let a = MacroDef {
            params: ParamSpec {
                num_params: 0,
                long: false,
                text: TokenArray::default(),
            },
            body: body.clone(),
            code: Some(Arc::new(Bytecode::default())),
            protected: false,
            outer: false,
            active_slot: false,
        };
        let b = MacroDef {
            params: ParamSpec {
                num_params: 0,
                long: false,
                text: TokenArray::default(),
            },
            body,
            code: None,
            protected: false,
            outer: false,
            active_slot: false,
        };
        assert_eq!(a, b, "\\ifx 语义不应受编译产物影响");
    }
}
