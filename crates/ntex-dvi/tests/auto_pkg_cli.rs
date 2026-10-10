//! ntex-pkg engine wiring: CLI-side preflight vendors missing `\input` files from
//! a local TeX Live tree before the engine starts reading.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

fn workdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("ntex-auto-pkg-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn run_clean(cwd: &Path, args: &[&OsStr]) -> (String, String, bool) {
    let out = Command::new(env!("CARGO_BIN_EXE_ntex-dvi"))
        .args(args)
        .current_dir(cwd)
        .env("PATH", "")
        .env("HOME", cwd.join("home"))
        .output()
        .unwrap();
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.success(),
    )
}

const TLPDB: &str = "\
name autopkg
category Package
revision 1
runfiles size=1
 texmf-dist/tex/plain/autopkg/autopkg.tex
";

#[test]
fn auto_pkg_resolves_missing_input_from_local_texlive() {
    let d = workdir("hit");
    let tl = d.join("tl");
    std::fs::create_dir_all(tl.join("tlpkg")).unwrap();
    std::fs::create_dir_all(tl.join("texmf-dist/tex/plain/autopkg")).unwrap();
    std::fs::write(tl.join("tlpkg/texlive.tlpdb"), TLPDB).unwrap();
    std::fs::write(
        tl.join("texmf-dist/tex/plain/autopkg/autopkg.tex"),
        "\\def\\autopkg{AUTO-PKG}\n",
    )
    .unwrap();
    let src = d.join("main.tex");
    std::fs::write(&src, "\\input autopkg\n\\shipout\\hbox{\\autopkg}\n").unwrap();
    let dvi = d.join("main.dvi");
    let vendored = d.join("vendored");

    let (stdout, stderr, ok) = run_clean(
        &d,
        &[
            src.as_os_str(),
            dvi.as_os_str(),
            OsStr::new("--auto-pkg"),
            OsStr::new("--pkg-tlpdb"),
            tl.join("tlpkg/texlive.tlpdb").as_os_str(),
            OsStr::new("--pkg-root"),
            tl.as_os_str(),
            OsStr::new("--pkg-vendor-dir"),
            vendored.as_os_str(),
        ],
    );
    assert!(ok, "stdout={stdout}\nstderr={stderr}");
    assert!(dvi.exists(), "DVI 未产出");
    assert!(
        vendored.join("tex/plain/autopkg/autopkg.tex").exists(),
        "自动取料未物化 autopkg.tex"
    );
    assert!(
        stderr.contains("[auto-pkg]"),
        "缺少 auto-pkg 转录：{stderr:?}"
    );
}

#[test]
fn auto_pkg_missing_everywhere_reports_package_name() {
    let d = workdir("miss");
    let tl = d.join("tl");
    std::fs::create_dir_all(tl.join("tlpkg")).unwrap();
    std::fs::write(tl.join("tlpkg/texlive.tlpdb"), TLPDB).unwrap();
    let src = d.join("main.tex");
    std::fs::write(&src, "\\input no-such-auto-pkg\n\\shipout\\hbox{x}\n").unwrap();
    let dvi = d.join("main.dvi");

    let (_stdout, stderr, ok) = run_clean(
        &d,
        &[
            src.as_os_str(),
            dvi.as_os_str(),
            OsStr::new("--auto-pkg"),
            OsStr::new("--pkg-tlpdb"),
            tl.join("tlpkg/texlive.tlpdb").as_os_str(),
            OsStr::new("--pkg-root"),
            tl.as_os_str(),
        ],
    );
    assert!(!ok, "缺包应失败");
    assert!(
        stderr.contains("no-such-auto-pkg"),
        "错误应带缺包名：{stderr:?}"
    );
    assert!(!stderr.contains("panic"), "缺包不得 panic：{stderr:?}");
}
