#!/usr/bin/env python3
"""末页冲页判据矩阵：pdfTeX ground truth vs NTex。

## 为什么需要它

「什么内容算一页」是 LaTeX 兼容链条上的**基础契约**：plain 的 `\bye`
= `\\par\\vfill\\supereject\\end`，若冲页判据与真 TeX 不同，则
**每个无正文的宏文件都会产出一个假页**（corpus `plain/*` 三个 EMPTY 即此）。

pdfTeX ground truth（2026-09-11 实测，`-interaction=nonstopmode`）：

| 输入                | pdfTeX      | 判据 |
|---------------------|-------------|------|
| `\\vfill\\end`         | **1 页**    | 垂直胶水 + `\\end` 路径 → 冲页 |
| `\\vskip10pt\\end`     | **1 页**    | 同上 |
| `\\kern10pt\\end`      | **1 页**    | kern 算内容 |
| `\\hrule height1pt\\end`| **1 页**   | 规则算内容 |
| `\\null\\end`          | **1 页**    | 空盒算内容 |
| `\\penalty100\\end`    | **0 页**    | 单独 penalty 不算内容 |
| `\\vskip10pt\\supereject\\end` | **0 页** | **胶水 + 强惩罚断页 → 丢页** |
| `\\vfill\\supereject\\end`     | **0 页** | 同上（= plain `\\bye` 的形态）|
| `\\null\\vfill\\supereject\\end`| **1 页**| 有盒子 → 保页 |

**核心结论**：`\\supereject`（强惩罚）触发的断页在**页面只有胶水**时被丢弃
（tex.web `fire_up` 的 `page_head` 判据）；而 `\\end` 的 `its_all_over` 路径
对纯胶水**仍然冲页**。

## 用法

    scripts/page-eject-matrix.py            # 跑矩阵，两侧对照
    scripts/page-eject-matrix.py --ntex-only # 只看 NTex（快）
"""
import subprocess
import sys
import tempfile
from pathlib import Path

PDFTEX = Path.home() / ".local/bin/pdftex"
NTEX = Path("/home/ubuntu/NTex/target/debug/ntex-dvi")

# (标签, 源码体)  —— 源码体后自动补 `\end`
CASES = [
    (r"\vfill", "垂直胶水 + \\end"),
    (r"\vskip10pt", "vskip + \\end"),
    (r"\kern10pt", "kern + \\end"),
    (r"\hrule height1pt", "规则 + \\end"),
    (r"\null", "空盒 + \\end"),
    (r"\penalty100", "单独 penalty + \\end"),
    (r"\vskip10pt\supereject", "胶水 + 强惩罚"),
    (r"\vfill\supereject", "vfill + 强惩罚（= \\bye 形态）"),
    (r"\null\vfill\supereject", "空盒 + vfill + 强惩罚"),
]


def pages_pdftex(body: str, wd: Path) -> str:
    src = wd / "p.tex"
    src.write_text(body + "\\end\n")
    subprocess.run(
        [str(PDFTEX), "-interaction=nonstopmode", str(src)],
        cwd=wd, capture_output=True, timeout=60,
    )
    log = (wd / "p.log")
    if not log.exists():
        return "?"
    txt = log.read_text(errors="replace")
    if "No pages of output" in txt:
        return "0"
    import re
    m = re.search(r"Output written on .*\((\d+) page", txt)
    return m.group(1) if m else "?"


def pages_ntex(body: str, wd: Path) -> str:
    src = wd / "n.tex"
    src.write_text(body + "\\end\n")
    r = subprocess.run(
        [str(NTEX), str(src)], cwd=wd, capture_output=True, timeout=120,
        env={**__import__("os").environ, "NTEX_TFM_DIR": str(Path.home() / ".ntex-fonts")},
    )
    out = r.stdout.decode(errors="replace")
    import re
    m = re.search(r"(\d+) 页", out)
    return m.group(1) if m else ("0" if "未产出" in out else "?")


def main() -> int:
    ntex_only = "--ntex-only" in sys.argv
    wd = Path(tempfile.mkdtemp(prefix="eject-"))
    hdr = f"{'用例':<34} {'pdfTeX':>8} {'NTex':>8}   判定"
    print(hdr)
    print("-" * len(hdr))
    bad = 0
    for body, tag in CASES:
        nt = pages_ntex(body, wd)
        if ntex_only:
            print(f"{tag:<34} {'—':>8} {nt:>8}")
            continue
        px = pages_pdftex(body, wd)
        ok = "✅" if px == nt else "❌ 偏差"
        if px != nt:
            bad += 1
        print(f"{tag:<34} {px:>8} {nt:>8}   {ok}")
    if not ntex_only:
        print(f"\n{'一致' if bad == 0 else f'{bad} 项偏差'} / 共 {len(CASES)} 项")
    return 0 if bad == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
