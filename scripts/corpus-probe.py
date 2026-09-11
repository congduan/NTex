#!/usr/bin/env python3
"""NTex 语料库批量探针：逐 .tex 跑 ntex-dvi→ntex-pdf，产出矩阵报告。

用法：
  ~/.venvs/pixtools/bin/python scripts/corpus-probe.py            # 跑全库
  ~/.venvs/pixtools/bin/python scripts/corpus-probe.py plain      # 只跑 plain/

输出：
  - stdout：每个文件的 PASS/EMPTY/FAIL + 失败首错（一行）
  - fixtures/corpus/report.json：结构化结果（复测对比用）

口径（三级，2026-09-11 升级）：
  - FAIL ：无 DVI/PDF 产出，或引擎报错/超时
  - EMPTY：产出 PDF 但**页面无实质内容**（非白像素 < MIN_INK）
  - PASS ：产出 PDF 且有实质内容

⚠ 为什么必须验内容（2026-09-11 血的教训）：早期版本只判\"产物存在\"，探针曾因
输出路径假设错误把全库判成 0/8（产物其实都在，见下）；修正路径后又发现
`plain/plain.tex`、`plain/letterformat.tex` 这类**宏/格式文件**产出 1 页空页
也照样\"PASS\"——真 TeX 对无正文文件是 0 页，NTex 冲页出空页（survey §5.bis
发现未修 #1）。只验存在性的 KPI 会自欺，故引入非白像素判据。
"""
import json
import subprocess
import sys
from pathlib import Path

REPO = Path("/home/ubuntu/NTex")
CORPUS = REPO / "fixtures" / "corpus"
REPORT = CORPUS / "report.json"
ENV = {"PATH": "/usr/bin:/bin:/home/ubuntu/.cargo/bin",
       "NTEX_TFM_DIR": "/home/ubuntu/.ntex-fonts",
       "HOME": "/home/ubuntu"}
TIMEOUT = 90  # 秒/文件
MIN_INK = 200  # 非白像素阈值（40dpi 下低于此视为空页）

# pymupdf 在 pixtools venv 里；探针自身用同一解释器跑（见文件头用法）
try:
    import pymupdf
except ImportError:  # 退化：不判内容，只判存在性
    pymupdf = None


def run(cmd: list[str]) -> tuple[bool, str]:
    try:
        r = subprocess.run(cmd, capture_output=True, text=True,
                           timeout=TIMEOUT, env=ENV, cwd=str(REPO))
        return r.returncode == 0, (r.stdout + r.stderr).strip()
    except subprocess.TimeoutExpired:
        return False, f"TIMEOUT {TIMEOUT}s"


def measure_content(pdf: Path) -> tuple[int, int]:
    """返回 (非白像素数, 提取字符数)；无 pymupdf 时返回 (-1, -1)。"""
    if pymupdf is None:
        return -1, -1
    try:
        doc = pymupdf.open(pdf)
        chars = len("".join(p.get_text() for p in doc).strip())
        ink = 0
        for p in doc:
            pm = p.get_pixmap(dpi=40)
            ink += sum(1 for i in range(0, len(pm.samples), pm.n)
                       if pm.samples[i] < 250)
        doc.close()
        return ink, chars
    except Exception:
        return -1, -1


def probe_one(tex: Path) -> dict:
    """跑单文件全链，返回结果记录。"""
    stem = tex.stem
    # 文件名可能撞车（不同子目录同名）——报告用相对路径区分
    # ⚠ ntex-dvi / ntex-pdf 默认输出 = `<输入路径去扩展名>.dvi/.pdf`，
    # **保留源文件的目录**。探针按源文件目录取产物（早期版本假设输出在
    # 仓库根，导致 plain/*.tex 全部被误判 FAIL —— 实际 DVI 已成功写出）。
    outdir = tex.parent
    dvi = outdir / f"{stem}.dvi"
    pdf = outdir / f"{stem}.pdf"
    rec = {"file": str(tex.relative_to(CORPUS)), "dvi": None, "pdf": None,
           "ink": None, "chars": None, "status": "FAIL", "error": None}
    for f in (dvi, pdf):
        f.unlink(missing_ok=True)

    ok, out = run(["cargo", "run", "-q", "-p", "ntex-dvi", "--", str(tex)])
    if not ok or not dvi.exists():
        rec["error"] = f"DVI: {(out or 'no dvi')[-160:]}"
        return rec
    rec["dvi"] = dvi.stat().st_size

    ok, out = run(["cargo", "run", "-q", "-p", "ntex-pdf", "--", str(dvi)])
    if not ok or not pdf.exists():
        rec["error"] = f"PDF: {(out or 'no pdf')[-160:]}"
        return rec
    rec["pdf"] = pdf.stat().st_size

    ink, chars = measure_content(pdf)
    rec["ink"], rec["chars"] = ink, chars
    # 内容判据：无实质内容（空页）标 EMPTY，不算 PASS
    if ink >= 0 and ink < MIN_INK:
        rec["status"] = "EMPTY"
    else:
        rec["status"] = "PASS"

    dvi.unlink()  # 产物不入库（golden 另管）
    pdf.unlink()
    return rec


def main() -> None:
    only = sys.argv[1] if len(sys.argv) > 1 else None
    texs = sorted(p for d in CORPUS.iterdir() if d.is_dir() and d.name != "reference-pdf"
                  for p in d.glob("*.tex")
                  if only is None or d.name == only)
    results = [probe_one(t) for t in texs]
    n_pass = sum(1 for r in results if r["status"] == "PASS")
    n_empty = sum(1 for r in results if r["status"] == "EMPTY")
    REPORT.write_text(json.dumps(results, ensure_ascii=False, indent=1))
    for r in results:
        if r["status"] == "FAIL":
            print(f"FAIL  {r['file']}  {r['error']}")
        else:
            print(f"{r['status']:<5} {r['file']}  dvi={r['dvi']}B pdf={r['pdf']}B "
                  f"ink={r['ink']} chars={r['chars']}")
    print(f"\n{n_pass}/{len(results)} PASS"
          f"（另有 {n_empty} 个 EMPTY 空页）→ {REPORT}")
    # 清理产物（按源文件目录落盘）
    for d in CORPUS.iterdir():
        if d.is_dir():
            for f in d.glob("*.dvi"):
                f.unlink()
            for f in d.glob("*.pdf"):
                f.unlink()


if __name__ == "__main__":
    main()
