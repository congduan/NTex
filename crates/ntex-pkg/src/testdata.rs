//! 共享测试夹具（`cfg(test)`）：与真实 tlpdb 同格式的最小样本。
//!
//! 夹具刻意覆盖真实格式的全部要素——多记录（空行分隔）、`.ARCH` 依赖、
//! `release/`·`opt_` 非包依赖、`runfiles`/`binfiles`、SHA-512、
//! **同名副本**（`expl3-code.tex` 同时由 `base` 与 `l3kernel` 提供），
//! 以及最容易踩的一条：**`size=` 声明值与实际行数不一致**。
//!
//! 最后一条是刻意的：TL2024 的 `ae` 包声明 `runfiles size=157` 而实际只有 114 行
//! （`size=` 是 RIV 块数，不是行数）。本夹具取更极端的「声明 157 / 实际 3 行」，
//! 使任何「按 size= 读 N 行」的实现必然失败。
//!
//! 它同时是 tlpdb.rs 与 resolve.rs 的公共输入，避免两处各写一份而漂移。

/// 最小 TLPDB 样本。
pub(crate) const MINI_TLPDB: &str = "\
name geometry
category Package
revision 70501
shortdesc Show a page layout
depend atbegshi
depend pdftex.ARCH
depend release/2024
depend opt_install_docfiles:0
containersize 4096
containerchecksum 86424974e5f54ae5dd07a44af6b220f0a7d53988c5cebc450a8d1c6b8280d771e86dd3d5957649aaa3b73530435cf3794a5ed6c13febf9717269a994bf07b7f3
runfiles size=157
 texmf-dist/tex/latex/geometry/geometry.sty
 texmf-dist/doc/latex/geometry/geometry.pdf
 texmf-dist/tex/latex/geometry/geometry.cfg
binfiles arch=universal-darwin size=1
 bin/universal-darwin/geometry-helper

name atbegshi
category Package
revision 70501
containersize 2048
containerchecksum 927521fb6b6a5787d0e94ad724cf19825b2cf2ce23333e60e13625a36390eaa4cbaa1bbe50dbc718efae97036d5d815860919f536601bb97224b489d20082953
runfiles size=2
 texmf-dist/tex/latex/atbegshi/atbegshi.sty

name l3kernel
category Package
revision 71234
depend expl3
runfiles size=1
 texmf-dist/tex/latex/l3kernel/expl3-code.tex

name base
category Package
runfiles size=1
 texmf-dist/tex/latex/base/expl3-code.tex

name expl3
category Package
runfiles size=1
 texmf-dist/tex/latex/l3kernel/expl3.sty

name pdftex.universal-darwin
category TLCore
revision 70501
runfiles size=1
 texmf-dist/tex/generic/pdftex/pdftex-dvi.tex
";

/// 解析共享夹具；夹具本身不可解析属测试基建故障，直接 panic。
pub(crate) fn mini() -> crate::tlpdb::TlPdb {
    match crate::tlpdb::TlPdb::parse(MINI_TLPDB) {
        Ok(db) => db,
        Err(e) => panic!("测试 fixture 必须可解析：{e}"),
    }
}
