//! 契约层：`ntex.lock` 的确定性编解码（plan.md §6.2 第 9 条「契约层」）。
//!
//! **为什么需要它**：M5 增量与 M7 项目级 `.fmt` 都要求「同一份源码 = 同一份输入」。
//! 「按需下载」天然是浮动的（镜像上今天和明天的 `geometry` 可能不是同一份字节），
//! 所以按需取料必须配一份**锁定契约**：包名 + revision + SHA-512。锁定即复现，
//! 浮动的「最新版」永远不进 `.fmt`。
//!
//! **格式**（行文本、手写编解码、零依赖）：
//!
//! ```text
//! ntex-lock 1
//! # 注释行
//!
//! [[pkg]]
//! name geometry
//! category Package
//! revision 70501
//! size 4096
//! sha512 <128 位十六进制>
//! files 3
//! ```
//!
//! **身份字段 vs 参考字段**：`name` / `revision` / `sha512` 决定「这是不是同一份输入」，
//! 缺失即拒绝（[`Error::NotLockable`]）；`category` / `size` / `files` 仅供审阅，
//! 不参与校验，缺失记空/0。
//!
//! **与 TLPDB 相反的一条纪律**：TLPDB 解析**忽略**未知字段（上游会持续加字段，必须前向兼容）；
//! 锁文件**拒绝**未知字段——把 `revision` 拼成 `revisoin` 会让校验静默失效，
//! 契约文件宁可报错也不许降级。

use std::collections::BTreeMap;

use crate::cache::{is_safe_component, is_valid_sha512};
use crate::resolve::Closure;
use crate::tlpdb::TlPdb;
use crate::Error;

/// 格式魔数（同时是格式版本号）。
pub const MAGIC: &str = "ntex-lock 1";

/// 一条锁定记录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockedPackage {
    /// 包名（TLPDB 主键）。
    pub name: String,
    /// 类别字面（参考字段）。
    pub category: String,
    /// 版本号（身份字段）。
    pub revision: u64,
    /// 容器字节数（参考字段；TLPDB 未给出则 0）。
    pub size: u64,
    /// 容器 SHA-512（身份字段）。
    pub sha512: String,
    /// 运行面文件数（参考字段；用于发现「同 revision 文件清单变了」的异常）。
    pub files: usize,
}

/// 锁文件与当前解析结果之间的差异。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LockDiff {
    /// 现在需要、契约里没有的包（需重新生成 lock）。
    pub added: Vec<String>,
    /// 契约里有、现在不需要的包（可清理）。
    pub removed: Vec<String>,
    /// 两边都有但 revision 或 sha512 变了（**必须**重新生成 lock）。
    pub changed: Vec<String>,
}

impl LockDiff {
    /// 是否有任何差异。
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.changed.is_empty()
    }

    /// 差异是否会改变「算出来的字节」——换版本/换校验和会，增删包通常不会
    /// （增删只影响可用命令集）。
    pub fn alters_output(&self) -> bool {
        !self.changed.is_empty()
    }
}

/// 从依赖闭包生成锁定记录。
///
/// 任何缺 `revision` 或缺 `container-sha512` 的包都会让整次锁定失败——
/// 宁可拒绝，也不产出一份「看着像锁、实际验不了」的契约。
pub fn from_closure(db: &TlPdb, closure: &Closure) -> Result<Vec<LockedPackage>, Error> {
    let mut out = Vec::with_capacity(closure.packages.len());
    for name in &closure.packages {
        let Some(pkg) = db.get(name) else {
            // closure 已保证成员存在；此处仅为防御（TLPDB 换了对象）
            continue;
        };
        let Some(revision) = pkg.revision else {
            return Err(Error::NotLockable {
                package: name.clone(),
                reason: "TLPDB 未给出 revision——无版本号即不可复现".to_owned(),
            });
        };
        let Some(sha512) = pkg.container_sha512.clone() else {
            return Err(Error::NotLockable {
                package: name.clone(),
                reason: "TLPDB 未给出 container-sha512——无校验和即不可验证".to_owned(),
            });
        };
        out.push(LockedPackage {
            name: name.clone(),
            category: pkg
                .category
                .as_ref()
                .map(|c| c.as_str().to_owned())
                .unwrap_or_default(),
            revision,
            size: pkg.container_size.unwrap_or(0),
            sha512,
            files: pkg.run_files.len(),
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// 编码锁文件。**确定性**：按包名排序、字段顺序固定、只用 LF，与入参顺序无关。
///
/// 编码前校验名字安全与 SHA-512 形状——不产出一份「写得进去、验不起来」的契约。
pub fn encode(pkgs: &[LockedPackage]) -> Result<String, Error> {
    let mut items: Vec<&LockedPackage> = pkgs.iter().collect();
    items.sort_by(|a, b| a.name.cmp(&b.name));
    for pair in items.windows(2) {
        if pair[0].name == pair[1].name {
            return Err(Error::Lock {
                line: 1,
                detail: format!("重复的包记录 `{}`", pair[0].name),
            });
        }
    }

    let mut out = String::with_capacity(items.len() * 160 + 128);
    out.push_str(MAGIC);
    out.push('\n');
    out.push_str("# NTex 宏包契约（生成物，请勿手改；重生成：`ntex-pkg lock`）\n");
    out.push_str("# 身份字段：name / revision / sha512；参考字段：category / size / files\n");

    for pkg in items {
        if !is_safe_component(&pkg.name) {
            return Err(Error::UnsafeName {
                name: pkg.name.clone(),
                context: "锁文件包名",
            });
        }
        if !is_valid_sha512(&pkg.sha512) {
            return Err(Error::NotLockable {
                package: pkg.name.clone(),
                reason: format!(
                    "sha512 字面非法（须 128 位十六进制，实际 {} 字符）",
                    pkg.sha512.len()
                ),
            });
        }
        out.push('\n');
        out.push_str("[[pkg]]\n");
        out.push_str(&format!("name {}\n", pkg.name));
        if !pkg.category.is_empty() {
            out.push_str(&format!("category {}\n", pkg.category));
        }
        out.push_str(&format!("revision {}\n", pkg.revision));
        out.push_str(&format!("size {}\n", pkg.size));
        out.push_str(&format!("sha512 {}\n", pkg.sha512));
        out.push_str(&format!("files {}\n", pkg.files));
    }
    Ok(out)
}

/// 解码锁文件。返回值按包名排序（与编码顺序无关），可安全做相等比较。
pub fn decode(text: &str) -> Result<Vec<LockedPackage>, Error> {
    let mut out: Vec<LockedPackage> = Vec::new();
    let mut cur: Option<Partial> = None;
    let mut magic_seen = false;

    for (idx, raw) in text.lines().enumerate() {
        let line_no = idx + 1;
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        let t = line.trim();

        if !magic_seen {
            // 魔数须是**首个非注释、非空行**（允许文件头放许可证/说明注释）
            if t.is_empty() || t.starts_with('#') {
                continue;
            }
            if t != MAGIC {
                return Err(Error::Lock {
                    line: line_no,
                    detail: format!("首个非注释行必须是 `{MAGIC}`，实际 `{t}`"),
                });
            }
            magic_seen = true;
            continue;
        }

        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        if t == "[[pkg]]" {
            if let Some(p) = cur.take() {
                out.push(p.finish()?);
            }
            cur = Some(Partial {
                start_line: line_no,
                ..Partial::default()
            });
            continue;
        }

        let Some(pkg) = cur.as_mut() else {
            return Err(Error::Lock {
                line: line_no,
                detail: format!("字段 `{t}` 出现在任何 `[[pkg]]` 之前"),
            });
        };

        let Some((key, value)) = t.split_once(' ') else {
            return Err(Error::Lock {
                line: line_no,
                detail: format!("行 `{t}` 缺字段名与值之间的空格"),
            });
        };
        let value = value.trim();
        match key {
            "name" => pkg.name = value.to_owned(),
            "category" => pkg.category = value.to_owned(),
            "revision" => pkg.revision = Some(parse_num(value, line_no, "revision")?),
            "size" => pkg.size = Some(parse_num(value, line_no, "size")?),
            "sha512" => pkg.sha512 = value.to_owned(),
            "files" => pkg.files = Some(parse_usize(value, line_no, "files")?),
            other => {
                return Err(Error::Lock {
                    line: line_no,
                    detail: format!(
                        "未知字段 `{other}`——契约文件不容忍未知字段（拼错会让校验静默失效）"
                    ),
                })
            }
        }
    }

    if !magic_seen {
        return Err(Error::Lock {
            line: 1,
            detail: format!("文件为空或缺少 `{MAGIC}` 魔数行"),
        });
    }
    if let Some(p) = cur.take() {
        out.push(p.finish()?);
    }

    out.sort_by(|a, b| a.name.cmp(&b.name));
    for pair in out.windows(2) {
        if pair[0].name == pair[1].name {
            return Err(Error::Lock {
                line: 1,
                detail: format!("重复的包记录 `{}`", pair[0].name),
            });
        }
    }
    Ok(out)
}

/// 比较「契约里记的」与「现在解析出来的」。
///
/// `locked` 为契约文件内容，`current` 为本次解析结果。
pub fn diff(locked: &[LockedPackage], current: &[LockedPackage]) -> LockDiff {
    let old: BTreeMap<&str, &LockedPackage> = locked.iter().map(|p| (p.name.as_str(), p)).collect();
    let new: BTreeMap<&str, &LockedPackage> =
        current.iter().map(|p| (p.name.as_str(), p)).collect();

    let mut d = LockDiff::default();
    for (name, np) in &new {
        match old.get(name) {
            None => d.added.push((*name).to_owned()),
            Some(op) => {
                if op.revision != np.revision || !op.sha512.eq_ignore_ascii_case(&np.sha512) {
                    d.changed.push((*name).to_owned());
                }
            }
        }
    }
    for name in old.keys() {
        if !new.contains_key(name) {
            d.removed.push((*name).to_owned());
        }
    }
    d
}

/// 解码期间的半成品记录（身份字段缺失时不能构造 [`LockedPackage`]）。
#[derive(Debug, Default)]
struct Partial {
    start_line: usize,
    name: String,
    category: String,
    revision: Option<u64>,
    size: Option<u64>,
    sha512: String,
    files: Option<usize>,
}

impl Partial {
    fn finish(self) -> Result<LockedPackage, Error> {
        if self.name.is_empty() {
            return Err(Error::Lock {
                line: self.start_line,
                detail: "`[[pkg]]` 块缺 `name`".to_owned(),
            });
        }
        if !is_safe_component(&self.name) {
            return Err(Error::UnsafeName {
                name: self.name,
                context: "锁文件包名",
            });
        }
        let Some(revision) = self.revision else {
            return Err(Error::Lock {
                line: self.start_line,
                detail: format!("包 `{}` 缺 `revision`（身份字段，不可省）", self.name),
            });
        };
        if self.sha512.is_empty() {
            return Err(Error::Lock {
                line: self.start_line,
                detail: format!("包 `{}` 缺 `sha512`（身份字段，不可省）", self.name),
            });
        }
        if !is_valid_sha512(&self.sha512) {
            return Err(Error::Lock {
                line: self.start_line,
                detail: format!("包 `{}` 的 sha512 字面非法（须 128 位十六进制）", self.name),
            });
        }
        Ok(LockedPackage {
            name: self.name,
            category: self.category,
            revision,
            size: self.size.unwrap_or(0),
            sha512: self.sha512,
            files: self.files.unwrap_or(0),
        })
    }
}

fn parse_num(s: &str, line: usize, key: &str) -> Result<u64, Error> {
    s.parse::<u64>().map_err(|_| Error::Lock {
        line,
        detail: format!("`{key}` 的值 `{s}` 不是非负整数"),
    })
}

/// `files` 是**文件计数**，按 `usize` 解析（wasm32 下 usize 为 32 位，不做 u64 截断转换）。
fn parse_usize(s: &str, line: usize, key: &str) -> Result<usize, Error> {
    s.parse::<usize>().map_err(|_| Error::Lock {
        line,
        detail: format!("`{key}` 的值 `{s}` 不是非负整数"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resolve::closure;
    use crate::testdata::mini;

    const SHA_A: &str = "86424974e5f54ae5dd07a44af6b220f0a7d53988c5cebc450a8d1c6b8280d771e86dd3d5957649aaa3b73530435cf3794a5ed6c13febf9717269a994bf07b7f3";
    const SHA_B: &str = "927521fb6b6a5787d0e94ad724cf19825b2cf2ce23333e60e13625a36390eaa4cbaa1bbe50dbc718efae97036d5d815860919f536601bb97224b489d20082953";

    fn pkg(name: &str, revision: u64, sha: &str) -> LockedPackage {
        LockedPackage {
            name: name.to_owned(),
            category: "Package".to_owned(),
            revision,
            size: 4096,
            sha512: sha.to_owned(),
            files: 3,
        }
    }

    #[test]
    fn roundtrip_is_stable() {
        let items = vec![pkg("geometry", 70501, SHA_A), pkg("atbegshi", 1, SHA_B)];
        let text = encode(&items).expect("编码应成功");
        let back = decode(&text).expect("解码应成功");
        let mut expect = items.clone();
        expect.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(back, expect, "decode(encode(x)) 必须等于按名排序后的 x");
    }

    #[test]
    fn encode_is_deterministic_and_order_independent() {
        let a = vec![pkg("geometry", 70501, SHA_A), pkg("atbegshi", 1, SHA_B)];
        let b = vec![pkg("atbegshi", 1, SHA_B), pkg("geometry", 70501, SHA_A)];
        let ea = encode(&a).expect("编码 a");
        let eb = encode(&b).expect("编码 b");
        assert_eq!(ea, eb, "入参顺序不得影响输出字节");
        assert_eq!(
            ea,
            encode(&a).expect("再编一次"),
            "同一输入两次编码必须逐字节相同"
        );
    }

    #[test]
    fn encode_output_shape_is_reviewable() {
        let text = encode(&[pkg("geometry", 70501, SHA_A)]).expect("编码");
        assert!(text.starts_with(MAGIC), "首行必须是魔数");
        assert!(text.contains("\n[[pkg]]\n"));
        assert!(text.contains("\nname geometry\n"));
        assert!(text.contains("\nrevision 70501\n"));
        assert!(!text.contains('\r'), "只允许 LF");
        assert!(text.ends_with('\n'), "以换行收尾");
    }

    #[test]
    fn encode_rejects_duplicate_names() {
        let items = vec![pkg("geometry", 1, SHA_A), pkg("geometry", 2, SHA_B)];
        assert!(matches!(encode(&items), Err(Error::Lock { .. })));
    }

    #[test]
    fn encode_rejects_malformed_sha512() {
        assert!(matches!(
            encode(&[pkg("geometry", 1, "deadbeef")]),
            Err(Error::NotLockable { .. })
        ));
    }

    #[test]
    fn encode_rejects_unsafe_name() {
        assert!(matches!(
            encode(&[pkg("../escape", 1, SHA_A)]),
            Err(Error::UnsafeName { .. })
        ));
    }

    #[test]
    fn decode_rejects_wrong_magic() {
        match decode("ntex-lock 2\n") {
            Err(Error::Lock { line, detail }) => {
                assert_eq!(line, 1);
                assert!(detail.contains("ntex-lock 1"), "detail: {detail}");
            }
            other => panic!("应报魔数不符，实际 {other:?}"),
        }
    }
    #[test]
    fn decode_rejects_unknown_field() {
        let text = format!("{MAGIC}\n\n[[pkg]]\nname x\nrevisoin 1\nsha512 {SHA_A}\n");
        match decode(&text) {
            Err(Error::Lock { line, detail }) => {
                assert_eq!(line, 5, "行号应指向拼错的字段");
                assert!(detail.contains("未知字段"), "detail: {detail}");
            }
            other => panic!("应报未知字段，实际 {other:?}"),
        }
    }

    #[test]
    fn decode_requires_identity_fields() {
        let no_rev = format!("{MAGIC}\n\n[[pkg]]\nname x\nsha512 {SHA_A}\n");
        assert!(matches!(decode(&no_rev), Err(Error::Lock { .. })));

        let no_sha = format!("{MAGIC}\n\n[[pkg]]\nname x\nrevision 1\n");
        match decode(&no_sha) {
            Err(Error::Lock { detail, .. }) => assert!(detail.contains("sha512"), "{detail}"),
            other => panic!("应报缺 sha512，实际 {other:?}"),
        }
    }

    #[test]
    fn decode_rejects_field_before_first_block() {
        let text = format!("{MAGIC}\nname x\n");
        match decode(&text) {
            Err(Error::Lock { line, .. }) => assert_eq!(line, 2),
            other => panic!("应报字段位置非法，实际 {other:?}"),
        }
    }

    #[test]
    fn decode_rejects_duplicate_blocks() {
        let text = format!(
            "{MAGIC}\n\n[[pkg]]\nname x\nrevision 1\nsha512 {SHA_A}\n\n\
             [[pkg]]\nname x\nrevision 2\nsha512 {SHA_B}\n"
        );
        assert!(matches!(decode(&text), Err(Error::Lock { .. })));
    }

    #[test]
    fn decode_tolerates_comments_and_blank_lines() {
        let text = format!(
            "# 前导注释\n\n{MAGIC}\n\n# 注释\n\n[[pkg]]\n\nname x\nrevision 1\nsha512 {SHA_A}\n"
        );
        let items = decode(&text).expect("注释与空行可容忍");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "x");
    }

    #[test]
    fn decode_of_empty_input_is_error_not_empty_lock() {
        assert!(matches!(decode(""), Err(Error::Lock { line: 1, .. })));
        assert!(matches!(decode("\n\n"), Err(Error::Lock { .. })));
    }

    #[test]
    fn diff_classifies_added_removed_changed() {
        let locked = vec![pkg("a", 1, SHA_A), pkg("b", 1, SHA_A), pkg("c", 1, SHA_A)];
        let current = vec![pkg("a", 1, SHA_A), pkg("b", 2, SHA_A), pkg("d", 1, SHA_A)];
        let d = diff(&locked, &current);
        assert_eq!(d.added, vec!["d".to_owned()]);
        assert_eq!(d.removed, vec!["c".to_owned()]);
        assert_eq!(d.changed, vec!["b".to_owned()]);
        assert!(!d.is_empty());
        assert!(d.alters_output(), "换版本必须视为会改变输出");
    }

    #[test]
    fn diff_detects_checksum_only_change() {
        let locked = vec![pkg("a", 1, SHA_A)];
        let current = vec![pkg("a", 1, SHA_B)];
        let d = diff(&locked, &current);
        assert_eq!(
            d.changed,
            vec!["a".to_owned()],
            "同 revision 不同 sha 也必须报 changed"
        );
        assert!(d.alters_output());
    }

    #[test]
    fn diff_is_empty_for_identical_inputs() {
        let v = vec![pkg("a", 1, SHA_A)];
        let d = diff(&v, &v);
        assert!(d.is_empty());
        assert!(!d.alters_output());
    }

    #[test]
    fn diff_of_added_only_does_not_alter_output() {
        let locked = vec![pkg("a", 1, SHA_A)];
        let current = vec![pkg("a", 1, SHA_A), pkg("b", 1, SHA_B)];
        let d = diff(&locked, &current);
        assert_eq!(d.added, vec!["b".to_owned()]);
        assert!(!d.alters_output(), "仅增包不改变既有包的字节");
    }

    #[test]
    fn from_closure_builds_sorted_records() {
        let db = mini();
        // 夹具里只有 geometry 带 checksum；先把它的依赖裁掉不现实，
        // 故这里直接构造一个只含 geometry 的闭包视角（种子即全部）。
        let c = crate::resolve::Closure {
            packages: ["geometry".to_owned()].into_iter().collect(),
            ..Default::default()
        };
        let items = from_closure(&db, &c).expect("geometry 可锁定");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "geometry");
        assert_eq!(items[0].revision, 70501);
        assert_eq!(items[0].files, 3);
        assert!(is_valid_sha512(&items[0].sha512));
    }

    #[test]
    fn from_closure_refuses_unlockable_package() {
        let db = mini();
        let c = closure(&db, &["geometry".to_owned()], "universal-darwin");
        match from_closure(&db, &c) {
            Err(Error::NotLockable { package, reason }) => {
                assert_eq!(package, "pdftex.universal-darwin");
                assert!(reason.contains("sha512"), "reason: {reason}");
            }
            other => panic!("应拒绝不可锁定包，实际 {other:?}"),
        }
    }
}
