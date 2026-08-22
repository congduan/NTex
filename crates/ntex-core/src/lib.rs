//! # ntex-core
//!
//! NTex 引擎的基础类型库，被所有上层 crate 复用。
//!
//! M0 阶段：错误、源码位置、版本号（见 [`error`] / [`span`] / [`version`]）。
//!
//! M1 阶段（RFC-1 数据模型 + 展开引擎）：
//! - [`token`]：8 字节 tagged union（RFC-1 §3）
//! - [`catcode`]：16 种 catcode 与默认查表（RFC-1 M1-4）
//! - [`intern`]：控制序列驻留表，csid=u32（RFC-1 §4）
//! - [`macrodef`]：宏体 TokenArray + 参数规格（RFC-1 §5）
//! - [`eqtb`]：等价槽，版本化 + `\let` 别名（RFC-1 §5）
//! - [`input`]：字节 → token 扫描器
//! - [`register`]：寄存器文件（\count/\dimen/\skip/\toks）与内部量格式化
//! - [`expand`]：展开引擎主循环（M1-6 ~ M1-11）

#![deny(unsafe_code)]

pub mod bytecode;
pub mod catcode;
pub mod eqtb;
pub mod error;
pub mod expand;
pub mod font;
pub mod input;
pub mod intern;
pub mod macrodef;
pub mod param;
pub mod register;
pub mod sink;
pub mod span;
pub mod token;
pub mod version;

pub use catcode::{Catcode, CatcodeTable};
pub use eqtb::{EqSlot, Eqtb, Primitive};
pub use error::{Error, Result};
pub use expand::Expander;
pub use font::{FontLoader, NoFontLoader};
pub use intern::InternTable;
pub use macrodef::{MacroDef, ParamSpec, TokenArray};
pub use register::{format_dimen, format_glue, Glue, Registers, SP_PER_PT, REGISTER_COUNT};
pub use sink::{TokenSink, VecSink};
pub use span::{BytePos, LineCol, SourceId, Span};
pub use token::{meaning, Token, TokenKind};
pub use version::{Version, VersionCounter, Versioned};
