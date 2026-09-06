// M8-A WASM 骨架线：TFM 字节源缝。
//
// 浏览器/Node 没有文件系统，[`TfmLoader`](super::TfmLoader) 原本 `find_tfm` +
// `std::fs::read` 的取字节路径在 wasm32 上不通。本文件提供宿主 → 排版器的
// TFM 注入口：宿主（`crates/ntex-wasm`，内嵌 CM 字体；或 JS 侧自带的 TFM 字节）
// 调 [`set_tfm_source`] 注册一个 [`TfmSource`]，`\font` 解析字体时按名字取字节。
//
// 设计取舍（与 native 的关系）：
// - 注册表是 thread_local、**默认为空**——不注册时 [`TfmLoader`](super::TfmLoader)
//   照走文件系统，native 既有行为逐字节不变（TRIP/ETRIP/demo 无感知）；
// - 缝本身不分目标编译（wasm/native 同一份代码），使 `cargo test -p ntex-wasm`
//   能在 native 上原样驱动「内嵌 TFM → 排版 → DVI」全链，与 wasm 侧行为同路径
//   可回归——这是 A 档能在无浏览器环境验证的主要手段；
// - `thread_local` 而非全局 `Mutex`：wasm32 单线程无需锁，native 测试多线程
//   并行也不互相污染（各测各的注册表）。
//
// 本文件经 `include!` 嵌入 typeset/mod.rs（与其余分片一致），`Rc`/`RefCell`
// 复用父模块顶部的 use，不重复导入。

/// TFM 字节源：按字体名提供 TFM 文件字节。
///
/// 与 [`ntex_core::FontLoader`] 的分工：本 trait 只负责「名字 → TFM 字节」
/// （RFC-3 里 Vfs 之于文件的角色），TFM 解析、nullfont 占位、同参去重、
/// at/scaled 缩放仍由 [`TfmLoader`](super::TfmLoader) 统一负责。
pub trait TfmSource: std::fmt::Debug {
    /// 取字体 `name` 的 TFM 字节；缺失返回 `None`（上层报 TeX 的"找不到字体"错误）。
    fn tfm_bytes(&mut self, name: &str) -> Option<Vec<u8>>;
}

/// 可克隆的共享源句柄（`Rc<RefCell>`：wasm 单线程足够，native 无跨线程需求）。
pub type SharedTfmSource = Rc<RefCell<Box<dyn TfmSource>>>;

thread_local! {
    static TFM_SOURCE: RefCell<Option<SharedTfmSource>> = const { RefCell::new(None) };
}

/// 注册 TFM 字节源（覆盖既有注册）。wasm32 宿主（ntex-wasm）启动时调用；
/// native 宿主可选调用（显式 opt-in，见模块注释）。
pub fn set_tfm_source(source: Box<dyn TfmSource>) {
    TFM_SOURCE.with(|cell| *cell.borrow_mut() = Some(Rc::new(RefCell::new(source))));
}

/// 取消注册（测试隔离用；wasm 侧重复编译同一模块时也可用于复位）。
pub fn clear_tfm_source() {
    TFM_SOURCE.with(|cell| *cell.borrow_mut() = None);
}

/// 已注册则取名字对应的 TFM 字节；未注册返回 `None`（调用方回落文件系统）。
pub fn registered_tfm_bytes(name: &str) -> Option<Vec<u8>> {
    TFM_SOURCE.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|src| src.borrow_mut().tfm_bytes(name))
    })
}

#[cfg(test)]
mod wasm_fonts_tests {
    use super::*;

    #[derive(Debug)]
    struct Fixed(Vec<String>);

    impl TfmSource for Fixed {
        fn tfm_bytes(&mut self, name: &str) -> Option<Vec<u8>> {
            self.0
                .iter()
                .any(|n| n.as_str() == name)
                .then(|| format!("{name}-bytes").into_bytes())
        }
    }

    /// 注册表为空 → 无字节（TfmLoader 回落文件系统）；注册 → 按名取字节；
    /// clear 后复位。测试线程互不干扰（thread_local）。
    #[test]
    fn registry_roundtrip() {
        clear_tfm_source();
        assert_eq!(registered_tfm_bytes("cmr10"), None);
        set_tfm_source(Box::new(Fixed(vec!["cmr10".to_owned()])));
        assert_eq!(
            registered_tfm_bytes("cmr10"),
            Some(b"cmr10-bytes".to_vec())
        );
        assert_eq!(registered_tfm_bytes("cmti10"), None);
        clear_tfm_source();
        assert_eq!(registered_tfm_bytes("cmr10"), None);
    }
}
