use std::fs;
use std::process::Command;

#[test]
fn label_after_amsmath_measurement_survives_and_resolves_on_second_pass() {
    let dir = std::env::temp_dir().join(format!("ntex-align-label-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let tex = dir.join("align-label.tex");
    let dvi = dir.join("align-label.dvi");
    fs::write(&tex, r"\documentclass{article}
\usepackage{amsmath}
\begin{document}
\begin{align*}a&=b\end{align*}
\section{After}\label{sec:after}
See Section~\ref{sec:after}.
\end{document}
").unwrap();
    for pass in 1..=2 {
        let output = Command::new(env!("CARGO_BIN_EXE_ntex-dvi"))
            .arg(&tex).arg(&dvi).arg("--interaction").arg("nonstopmode")
            .current_dir(&dir).output().unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "pass {pass}: {stderr}");
        if pass == 2 {
            assert!(!stderr.contains("Reference `sec:after'"), "{stderr}");
        }
    }
    let aux = fs::read_to_string(dir.join("align-label.aux")).unwrap();
    assert!(aux.contains("\\newlabel{sec:after}"), "{aux}");
    fs::remove_dir_all(dir).unwrap();
}
