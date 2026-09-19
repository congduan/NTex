//! 落地层：内容寻址缓存路径 + 校验和比对（plan.md §6.2 第 9 条「落地层」）。
//!
//! 三条纪律：
//!
//! 1. **不做 IO**：只算路径、只比字符串。落盘一律经 [`ntex_io::Vfs`]（RFC-3），
//!    于是同一套逻辑在 CLI 与 WASM（`MemVfs`）里都能跑。
//! 2. **路径即内容**：缓存目录由「包名 + revision」命名，锁定值同时进
//!    [`crate::lock`] 的契约文件，二者必须一致才能命中。
//! 3. **路径必须安全**：包名来自 TLPDB，但仍是外部输入——含 `/`、`..`、绝对路径
//!    一律拒绝，否则缓存目录可以被写出工作区（下载源的路径逃逸防线）。
//!
//! **未实现**：SHA-512 **字节级**校验。本模块只做「128 位十六进制字面」的形状校验
//! 与锁定值/实际值比对；真正对字节流算哈希需要新依赖，按仓库「不轻易加依赖」的
//! 口径推迟，见 docs/KNOWN-SIMPLIFICATIONS.md。

use crate::Error;

/// 默认缓存根目录（相对用户主目录的原始字符串；**不做 `~` 展开**——
/// 展开是宿主的职责，WASM 下没有家目录概念）。
pub const DEFAULT_CACHE_ROOT: &str = ".ntex/pkgs";

/// SHA-512 字面是否合法：长度 128 且全部为十六进制字符。
pub fn is_valid_sha512(s: &str) -> bool {
    s.len() == 128 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// 名字是否可以安全地作为缓存路径的一段。
///
/// 拒绝：空串、含 `/` 或 `\`、`..`、以 `.` 开头（避免 `.sha512` 一类 sidecar 冲突）。
pub fn is_safe_component(s: &str) -> bool {
    !s.is_empty() && s != ".." && !s.starts_with('.') && !s.contains('/') && !s.contains('\\')
}

/// 生成一个包的缓存目录：`<root>/<name>@<revision>`。
///
/// `revision` 是必填（不是 `Option`）——没有版本号的包**不可锁定**，也就
/// 不该有缓存目录；把缺失静默当 0 会让「同一条 lock 在不同时间指向不同字节」。
pub fn package_dir(root: &str, name: &str, revision: u64) -> Result<String, Error> {
    if !is_safe_component(name) {
        return Err(Error::UnsafeName {
            name: name.to_owned(),
            context: "缓存目录名",
        });
    }
    Ok(crate::join_path(root, &format!("{name}@{revision}")))
}

/// 包缓存目录内的校验和 sidecar 文件路径。
pub fn checksum_sidecar(dir: &str) -> String {
    crate::join_path(dir, ".sha512")
}

/// 比对锁定值与实际值。
///
/// `expected` 来自 [`crate::lock`]，`actual` 来自刚取到的字节流。
/// 两侧都须是合法 SHA-512 字面，否则视为格式错误（而非「不匹配」）——免得
/// 拿一个截断的字符串糊弄过校验。
pub fn check_sha512(package: &str, expected: &str, actual: &str) -> Result<(), Error> {
    if !is_valid_sha512(expected) || !is_valid_sha512(actual) {
        return Err(Error::ChecksumMismatch {
            package: package.to_owned(),
            expected: format!("{expected}（形状非法，须 128 位十六进制）"),
            actual: format!("{actual}（形状非法，须 128 位十六进制）"),
        });
    }
    if expected.eq_ignore_ascii_case(actual) {
        Ok(())
    } else {
        Err(Error::ChecksumMismatch {
            package: package.to_owned(),
            expected: expected.to_owned(),
            actual: actual.to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA_A: &str = "86424974e5f54ae5dd07a44af6b220f0a7d53988c5cebc450a8d1c6b8280d771e86dd3d5957649aaa3b73530435cf3794a5ed6c13febf9717269a994bf07b7f3";

    #[test]
    fn sha512_shape_check() {
        assert!(is_valid_sha512(SHA_A));
        assert!(is_valid_sha512(&"a".repeat(128)));
        assert!(!is_valid_sha512(&"a".repeat(127)), "长度不足必须拒绝");
        assert!(!is_valid_sha512(&"a".repeat(129)), "长度超出必须拒绝");
        assert!(!is_valid_sha512(&"z".repeat(128)), "非十六进制必须拒绝");
        assert!(!is_valid_sha512(""), "空串必须拒绝");
    }

    #[test]
    fn unsafe_components_are_rejected() {
        assert!(is_safe_component("geometry"));
        assert!(is_safe_component("pdftex.universal-darwin"));
        assert!(!is_safe_component(""));
        assert!(!is_safe_component(".."));
        assert!(!is_safe_component("a/b"));
        assert!(!is_safe_component("a\\b"));
        assert!(!is_safe_component(".sha512"));
        assert!(!is_safe_component("../escape"));
    }

    #[test]
    fn package_dir_joins_name_and_revision() {
        let d = package_dir(".ntex/pkgs", "geometry", 70501).expect("合法名");
        assert_eq!(d, ".ntex/pkgs/geometry@70501");
        let d2 = package_dir(".ntex/pkgs/", "geometry", 1).expect("合法名");
        assert_eq!(d2, ".ntex/pkgs/geometry@1");
    }

    #[test]
    fn package_dir_rejects_escape_attempt() {
        match package_dir(".ntex/pkgs", "../../etc", 1) {
            Err(Error::UnsafeName { name, .. }) => assert_eq!(name, "../../etc"),
            other => panic!("应拒绝路径逃逸，实际 {other:?}"),
        }
    }

    #[test]
    fn checksum_sidecar_lives_in_package_dir() {
        assert_eq!(
            checksum_sidecar(".ntex/pkgs/geometry@70501"),
            ".ntex/pkgs/geometry@70501/.sha512"
        );
    }

    #[test]
    fn check_sha512_accepts_match_case_insensitively() {
        let upper = SHA_A.to_uppercase();
        assert!(check_sha512("geometry", SHA_A, &upper).is_ok());
    }

    #[test]
    fn check_sha512_reports_mismatch_with_both_values() {
        let other = "b".repeat(128);
        match check_sha512("geometry", SHA_A, &other) {
            Err(Error::ChecksumMismatch {
                package,
                expected,
                actual,
            }) => {
                assert_eq!(package, "geometry");
                assert_eq!(expected, SHA_A);
                assert_eq!(actual, other);
            }
            other => panic!("应报校验和不匹配，实际 {other:?}"),
        }
    }

    #[test]
    fn check_sha512_rejects_malformed_input_before_comparing() {
        match check_sha512("geometry", SHA_A, "deadbeef") {
            Err(Error::ChecksumMismatch { actual, .. }) => {
                assert!(
                    actual.contains("形状非法"),
                    "actual 应说明形状问题：{actual}"
                );
            }
            other => panic!("应报形状非法，实际 {other:?}"),
        }
    }
}
