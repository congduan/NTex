//! ntex-dvi 二进制级探针（格式预载 G0/G1）：
//! - 转录通道：`\message`/undefined cs 必须落到 stderr（survey §2.4 的「静默」陷阱）；
//! - `--input-path`：`\input` 在 cwd 找不到的文件经搜索路径命中。
//!
//! 走真二进制（`CARGO_BIN_EXE_*`）而非库面：诊断价值就在 CLI 的 stderr 透传。

use std::ffi::OsStr;
use std::path::PathBuf;
use std::process::Command;

fn workdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("ntex-dvi-cli-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn run(args: &[&OsStr]) -> (String, String, bool) {
    let out = Command::new(env!("CARGO_BIN_EXE_ntex-dvi"))
        .args(args)
        .output()
        .unwrap();
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.success(),
    )
}

#[test]
fn message_and_undefined_cs_reach_stderr() {
    let d = workdir("g0");
    let src = d.join("p.tex");
    std::fs::write(
        &src,
        "\\message{hello}\n\\bogusprimitive\n\\shipout\\hbox{x}\n",
    )
    .unwrap();
    let dvi = d.join("p.dvi");
    let (stdout, stderr, ok) = run(&[src.as_os_str(), dvi.as_os_str()]);
    assert!(ok, "stdout={stdout}\nstderr={stderr}");
    assert!(
        stderr.contains("hello"),
        "\\message 未上 stderr：{stderr:?}"
    );
    assert!(
        stderr.contains("! Undefined control sequence."),
        "undefined cs 静默跳过（survey §2.4 回归）：{stderr:?}"
    );
}

#[test]
fn input_path_resolves_input_from_other_dir() {
    let d = workdir("g1");
    // plain 库目录（模拟 texmf 树）与主文件所在目录分离。
    let lib = d.join("tex/plain/base");
    std::fs::create_dir_all(&lib).unwrap();
    std::fs::write(lib.join("hyphen.tex"), "% patterns stub\n").unwrap();
    let main = d.join("main.tex");
    std::fs::write(&main, "\\input hyphen\n\\shipout\\hbox{x}\n").unwrap();
    let dvi = d.join("main.dvi");
    let (stdout, stderr, ok) = run(&[
        main.as_os_str(),
        dvi.as_os_str(),
        OsStr::new("--input-path"),
        lib.as_os_str(),
    ]);
    assert!(ok, "stdout={stdout}\nstderr={stderr}");
    assert!(
        !stderr.contains("I can't find file"),
        "\\input 未走搜索路径：{stderr:?}"
    );
    assert!(dvi.exists(), "DVI 未产出");
}

#[test]
fn quiet_flag_suppresses_transcript() {
    let d = workdir("quiet");
    let src = d.join("q.tex");
    std::fs::write(&src, "\\message{SHOULD-NOT-APPEAR}\n\\shipout\\hbox{x}\n").unwrap();
    let dvi = d.join("q.dvi");
    let (stdout, stderr, ok) = run(&[src.as_os_str(), dvi.as_os_str(), OsStr::new("--quiet")]);
    assert!(ok, "stdout={stdout}\nstderr={stderr}");
    assert!(
        !stderr.contains("SHOULD-NOT-APPEAR"),
        "--quiet 未静音：{stderr:?}"
    );
}
