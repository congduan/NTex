#!/usr/bin/env python3
"""NTex 语料库批量探针：逐 .tex 跑 ntex-dvi→ntex-pdf，产出矩阵报告。

用法：
  ~/.venvs/pixtools/bin/python scripts/corpus-probe.py            # 跑全库
  ~/.venvs/pixtools/bin/python scripts/corpus-probe.py plain      # 只跑 plain/

输出：
  - stdout：每个文件的 PASS/FAIL + 失败首错（一行）
  - fixtures/corpus/report.json：结构化结果（复测对比用）

口径：只判"产出 PDF 且无引擎 ERROR"，不做像素对比（那是 pixdiff 的活）。
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


def run(cmd: list[str]) -> tuple[bool, str]:
    try:
        r = subprocess.run(cmd, capture_output=True, text=True,
                           timeout=TIMEOUT, env=ENV, cwd=str(REPO))
        return r.returncode == 0, (r.stdout + r.stderr).strip()
    except subprocess.TimeoutExpired:
        return False, f"TIMEOUT {TIMEOUT}s"


def probe_one(tex: Path) -> dict:
    """跑单文件全链，返回结果记录。"""
    stem = tex.stem
    # 文件名可能撞车（不同子目录同名）——工作副本带子目录前缀
    work = stem if tex.parent.name == "plain" else f"{tex.parent.name}-{stem}"
    dvi = REPO / f"{work}.dvi"
    pdf = REPO / f"{work}.pdf"
    rec = {"file": str(tex.relative_to(CORPUS)), "dvi": None, "pdf": None,
           "error": None}
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
    pdf.unlink()  # 成功产物不入库（golden 另管）
    return rec


def main() -> None:
    only = sys.argv[1] if len(sys.argv) > 1 else None
    texs = sorted(p for d in CORPUS.iterdir() if d.is_dir() and d.name != "reference-pdf"
                  for p in d.glob("*.tex")
                  if only is None or d.name == only)
    results = [probe_one(t) for t in texs]
    n_pass = sum(1 for r in results if r["pdf"])
    REPORT.write_text(json.dumps(results, ensure_ascii=False, indent=1))
    for r in results:
        if r["pdf"]:
            print(f"PASS {r['file']}  dvi={r['dvi']}B pdf={r['pdf']}B")
        else:
            print(f"FAIL {r['file']}  {r['error']}")
    print(f"\n{n_pass}/{len(results)} PASS → {REPORT}")
    # 清理工作副本 dvi
    for f in REPO.glob("*.dvi"):
        if f.name.startswith(("plain-", "latex-", "math-", "basic", "symbols")):
            f.unlink()


if __name__ == "__main__":
    main()
