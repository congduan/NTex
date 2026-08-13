//! 宏定义（RFC-1 §5）：连续不可变 TokenArray + 参数规格。
//!
//! 宏体采用连续内存（非 TeX 的链表），支持 O(1) 随机访问与整段共享；
//! 字节码双表示（TokenArray / Bytecode）在 M2 引入。

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
    /// 分隔参数（M1-8 填充；M1-5/6 仅支持无分隔参数）。
    pub delimiter: Option<TokenArray>,
}

/// 宏定义。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MacroDef {
    pub params: ParamSpec,
    pub body: TokenArray,
}

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
                delimiter: None,
            },
            body: body.clone(),
        };
        assert_eq!(def.params.num_params, 1);
        assert!(def.params.long);
        assert_eq!(def.body.len(), 1);
        // Arc 共享：克隆不拷贝数据
        let clone = def.body.clone();
        assert_eq!(clone, body);
    }
}
