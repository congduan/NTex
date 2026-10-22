use std::fs;
use std::process::Command;

#[test]
fn horizontal_mode_float_reaches_enddocument_clearpage() {
    let dir = std::env::temp_dir().join(format!("ntex-float-clearpage-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let tex = dir.join("float.tex");
    let dvi = dir.join("float.dvi");
    fs::write(
        &tex,
        r"\documentclass{article}
\begin{document}
\tracingoutput=1
Start.
\begin{figure}[ht]
\centering\rule{2cm}{2cm}
\caption{A float}
\end{figure}
\end{document}
",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_ntex-dvi"))
        .arg(&tex)
        .arg(&dvi)
        .arg("--interaction")
        .arg("nonstopmode")
        .current_dir(&dir)
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stderr}");
    assert!(!stderr.contains("Float(s) lost"), "{stderr}");
    assert!(
        stderr.contains("A float"),
        "caption must reach the shipped page: {stderr}"
    );
    assert!(stdout.contains("1 页"), "{stdout} {stderr}");
    fs::remove_dir_all(dir).unwrap();
}
