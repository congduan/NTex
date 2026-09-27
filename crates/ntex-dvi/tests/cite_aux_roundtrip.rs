//! `\cite` 引用通路（aux 读回）回归：
//!
//! 链路（latex.ltx ltbibl.dtx）：
//!   `\cite{key}` → `\@citex` 写 `\citation{key}` 入 aux、按 `b@<key>` 取编号；
//!   文献表侧 `\bibitem` → 写 `\bibcite{key}{n}` 入 aux；`\bibliography{db}`
//!   写 `\bibdata{db}` 且 `\@input@{\jobname.bbl}`（.bbl 名随 job，不随库名）；
//!   二趟 `\begin{document}` 读回 aux，`\bibcite` → `\@namedef{b@<key>}{n}`。
//!
//! 读回缺失即「Question mark」：`\cite` 打印粗体 `?` 并告警
//! `Citation 'key' on page N undefined`。本文件用该告警的有无 + aux 内容
//! + `\typeout` 探针三面钉住读写两侧。
//!
//! 走真二进制（`CARGO_BIN_EXE_*`）双趟驱动：aux 的读写都发生在 CLI 进程内，
//! 单测探针（`expand_vfs` 内存 VFS）够不到「跨趟落盘再读回」这一半。

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// 独立工作目录：aux 落在 cwd（`\openout\@mainaux\jobname.aux`）。
fn workdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("ntex-cite-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

fn run(cwd: &Path, args: &[&Path]) -> (String, String, bool) {
    let out = Command::new(env!("CARGO_BIN_EXE_ntex-dvi"))
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap();
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.success(),
    )
}

fn compile(dir: &Path, job: &str, dvi: &str) -> (String, String, bool) {
    run(
        dir,
        &[
            dir.join(format!("{job}.tex")).as_path(),
            dir.join(dvi).as_path(),
        ],
    )
}

const CITE_DOC: &str = "\
\\documentclass{article}
\\begin{document}
X\\cite{foo}Y\\cite{foo, bar}Z
\\end{document}
";

#[test]
fn twocolumn_section_aux_keeps_contentsline_protected() {
    let d = workdir("twocol-section");
    fs::write(
        d.join("p.tex"),
        "\
\\documentclass[twocolumn]{article}
\\begin{document}
\\title{T}\\author{A}\\date{D}\\maketitle
\\begin{abstract}Abstract text.\\end{abstract}
\\section{Intro}
Hello.
\\end{document}
",
    )
    .unwrap();

    let (_stdout, stderr, _ok) = compile(&d, "p", "p.dvi");
    assert!(
        !stderr.contains("Runaway") && !stderr.contains("Incomplete \\if"),
        "stderr={stderr:?}"
    );
    let aux = fs::read_to_string(d.join("p.aux")).unwrap();
    assert!(
        aux.contains("\\@writefile{toc}{\\contentsline"),
        "aux={aux:?}"
    );
    assert!(!aux.contains("\\series@check@toks"), "aux={aux:?}");
    assert!(!aux.contains("\\protect \\gdef"), "aux={aux:?}");
}

#[test]
fn twocolumn_short_document_flushes_final_column() {
    let d = workdir("twocol-short");
    fs::write(
        d.join("p.tex"),
        "\
\\documentclass[twocolumn]{article}
\\begin{document}
Hello.
\\end{document}
",
    )
    .unwrap();

    let (stdout, stderr, ok) = compile(&d, "p", "p.dvi");
    assert!(ok, "stdout={stdout:?}\nstderr={stderr:?}");
    assert!(
        stdout.contains("1 页"),
        "short twocolumn document should ship one page: stdout={stdout:?}"
    );
    assert!(d.join("p.dvi").exists(), "DVI should be written");
}

#[test]
fn cite_pass1_writes_citation_and_warns_undefined() {
    let d = workdir("pass1");
    fs::write(d.join("p.tex"), CITE_DOC).unwrap();
    let (stdout, stderr, ok) = compile(&d, "p", "p.dvi");
    assert!(ok, "stdout={stdout}\nstderr={stderr}");

    // aux：单 key 与多 key（含逗号后空格，写出须去空白）各一条 \citation。
    let aux = fs::read_to_string(d.join("p.aux")).unwrap();
    assert!(aux.contains("\\citation{foo}"), "aux={aux:?}");
    assert!(aux.contains("\\citation{bar}"), "逗号后空格未去：{aux:?}");
    assert!(!aux.contains("\\citation{ bar}"), "aux={aux:?}");

    // 首趟读回缺失 → LaTeX 规范告警（[?] 之源）。
    assert!(
        stderr.contains("Citation `foo' on page 1 undefined"),
        "stderr={stderr:?}"
    );
}

#[test]
fn bibcite_in_aux_resolves_second_pass() {
    let d = workdir("bibcite");
    fs::write(d.join("p.tex"), CITE_DOC).unwrap();
    let (stdout, stderr, ok) = compile(&d, "p", "p.dvi");
    assert!(ok, "stdout={stdout}\nstderr={stderr}");

    // 手写 \bibcite（bibtex+thebibliography 一趟后 aux 的产物）。
    let aux = fs::read_to_string(d.join("p.aux")).unwrap();
    let seeded = aux.replace(
        "\\gdef \\@abspage@last",
        "\\bibcite{foo}{1}\n\\bibcite{bar}{2}\n\\gdef \\@abspage@last",
    );
    assert_ne!(seeded, aux, "aux 缺 \\@abspage@last 锚点：{aux:?}");
    fs::write(d.join("p.aux"), &seeded).unwrap();

    // 二趟：读回探针 + 告警消失。
    fs::write(
        d.join("p.tex"),
        "\
\\documentclass{article}
\\begin{document}
\\makeatletter\\typeout{BCHECK:\\b@foo/\\b@bar}\\makeatother
X\\cite{foo}Y\\cite{foo, bar}Z
\\end{document}
",
    )
    .unwrap();
    let (stdout, stderr, ok) = compile(&d, "p", "p2.dvi");
    assert!(ok, "stdout={stdout}\nstderr={stderr}");
    assert!(stderr.contains("BCHECK:1/2"), "读回探针：{stderr:?}");
    assert!(
        !stderr.contains("undefined"),
        "二趟仍告警未解析：{stderr:?}"
    );
}

#[test]
fn bibcite_key_with_subscript_char_roundtrips() {
    let d = workdir("bibcite-underscore");
    fs::write(
        d.join("p.tex"),
        "\
\\documentclass{article}
\\begin{document}
Cite \\cite{foo_bar}.
\\begin{thebibliography}{1}
\\bibitem{foo_bar} A. Author.
\\end{thebibliography}
\\end{document}
",
    )
    .unwrap();

    let (stdout, stderr, ok) = compile(&d, "p", "p1.dvi");
    assert!(ok, "stdout={stdout}\nstderr={stderr}");
    let aux = fs::read_to_string(d.join("p.aux")).unwrap();
    assert!(aux.contains("\\citation{foo_bar}"), "aux={aux:?}");
    assert!(aux.contains("\\bibcite{foo_bar}{1}"), "aux={aux:?}");
    assert!(
        !aux.contains("foobar"),
        "cat-8 underscore was dropped: {aux:?}"
    );

    let (stdout, stderr, ok) = compile(&d, "p", "p2.dvi");
    assert!(ok, "stdout={stdout}\nstderr={stderr}");
    assert!(
        !stderr.contains("Citation `foo_bar' on page 1 undefined"),
        "二趟仍未解析：{stderr:?}"
    );
}

const BBL: &str = "\
\\begin{thebibliography}{2}

\\bibitem{k1}
A.~Author.
\\newblock First entry.
\\newblock {\\em Journal}, 1(2):3--4, 2001.

\\bibitem{k2}
B.~Writer.
\\newblock Second entry.

\\end{thebibliography}
";

#[test]
fn bibliography_inputs_jobname_bbl_and_roundtrips_bibcite() {
    let d = workdir("bbl");
    // .bbl 名随 job（\@input@{\jobname.bbl}），不随 \bibliography{refs} 的库名。
    fs::write(d.join("p.bbl"), BBL).unwrap();
    fs::write(
        d.join("p.tex"),
        "\
\\documentclass{article}
\\begin{document}
Cite \\cite{k1} and \\cite{k2}.
\\bibliographystyle{plain}
\\bibliography{refs}
\\end{document}
",
    )
    .unwrap();

    // 趟 1：.bbl 被输入、文献表排版、\bibcite 落 aux（但正文 \cite 在前仍告警）。
    let (stdout, stderr, ok) = compile(&d, "p", "p1.dvi");
    assert!(ok, "stdout={stdout}\nstderr={stderr}");
    assert!(
        stderr.contains("Citation `k1' on page 1 undefined"),
        "趟1 未告警：{stderr:?}"
    );
    let aux = fs::read_to_string(d.join("p.aux")).unwrap();
    assert!(aux.contains("\\bibdata{refs}"), "aux={aux:?}");
    assert!(aux.contains("\\bibstyle{plain}"), "aux={aux:?}");
    assert!(aux.contains("\\bibcite{k1}{1}"), "aux={aux:?}");
    assert!(
        aux.contains("\\bibcite{k2}{2}"),
        "\\bibitem 编号未按 \\@listctr 递增：{aux:?}"
    );

    // 趟 2：引用解析，告警消失。
    let (stdout, stderr, ok) = compile(&d, "p", "p2.dvi");
    assert!(ok, "stdout={stdout}\nstderr={stderr}");
    assert!(
        !stderr.contains("undefined"),
        "趟2 仍有未解析引用：{stderr:?}"
    );
    // .bbl 改名（随库名）则不被输入——钉住 jobname 语义。
    let d2 = workdir("bbl-wrongname");
    fs::write(d2.join("refs.bbl"), BBL).unwrap();
    fs::write(
        d2.join("p.tex"),
        "\
\\documentclass{article}
\\begin{document}
Cite \\cite{k1}.
\\bibliographystyle{plain}
\\bibliography{refs}
\\end{document}
",
    )
    .unwrap();
    let (stdout, stderr, ok) = compile(&d2, "p", "p.dvi");
    assert!(ok, "stdout={stdout}\nstderr={stderr}");
    assert!(
        !fs::read_to_string(d2.join("p.aux"))
            .unwrap()
            .contains("\\bibcite"),
        "refs.bbl 不应被输入（.bbl 名随 job）"
    );
}
