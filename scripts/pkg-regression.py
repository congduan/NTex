#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""宏包 CI 回归集：ntex-dvi --auto-pkg 双跑矩阵 vs pdfTeX GT。

对 fixtures/pkg-regression/ 下每个最小样张各跑两遍：
  1. ntex-dvi --auto-pkg（本地 TL 树取料）→ DVI → dvipdfmx → PDF；
  2. pdfTeX GT（-interaction=nonstopmode，两趟取稳）→ PDF。
判定口径：PASS = ntex-dvi 写出 DVI 且 0 错（'^!' 行计数）。
GT 对照分级报告：GT 也错的构造不算 NTex 缺口（错误构成对齐 = 销账）。

用法：
  ~/.venvs/pixtools/bin/python scripts/pkg-regression.py [--pix] [--only a,b]
      [--work DIR] [--keep] [--no-report] [--json PATH]

幂等性：work 目录默认清空重建；不依赖网络（本地 TL 树）；重跑结果稳定。
顺序执行，一次只跑一个编译类进程；单文档 timeout 见 TIMEOUT_*。
"""

import argparse
import difflib
import os
import re
import shutil
import subprocess
import sys
from datetime import date
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
FIXTURES = REPO / "fixtures" / "pkg-regression"
NTEX = REPO / "target" / "debug" / "ntex-dvi"
REPORT = REPO / "docs" / "pkg-regression-baseline.md"
HOME = Path(os.path.expanduser("~"))
TLPDB = HOME / ".TinyTeX" / "tlpkg" / "texlive.tlpdb"
TLROOT = HOME / ".TinyTeX"
GT_BIN = HOME / ".local" / "bin" / "pdflatex"
DVIPDFMX = HOME / ".local" / "bin" / "dvipdfmx"
PIX_PY = HOME / ".venvs" / "pixtools" / "bin" / "python"

TIMEOUT_NTEX = 240
TIMEOUT_GT = 180
TIMEOUT_PDF = 120

# 文本层/像素对照用的包；矩阵次序 = 报告行序。
PACKAGES = [
    "geometry", "amsmath", "amssymb", "booktabs", "multirow", "enumitem",
    "hyperref", "microtype", "array", "xcolor", "tabularx", "graphicx",
    "etoolbox",
]

# 负例：本机 TLPDB 无提供者的包——验收 auto-pkg「缺包即报 + 反查指引」契约
# （本矩阵 13 包全被宿主 TinyTeX 树覆盖，取料探测为 no-op，可验收面即此契约）。
NEGATIVE_PKGS = ["caption"]

fitz = None  # pymupdf 惰性加载


def ensure_pymupdf():
    """pymupdf 缺席时自动落到 pixtools venv 解释器重入。"""
    global fitz
    try:
        import pymupdf as _fitz
        fitz = _fitz
        return
    except ImportError:
        pass
    if PIX_PY.exists() and Path(sys.executable).resolve() != PIX_PY.resolve():
        os.execv(str(PIX_PY), [str(PIX_PY), str(Path(__file__).resolve())] + sys.argv[1:])
    print("警告：pymupdf 不可用，文本层/像素对照跳过", file=sys.stderr)


def run(cmd, cwd, timeout):
    try:
        p = subprocess.run(
            [str(c) for c in cmd], cwd=str(cwd), timeout=timeout,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            text=True, errors="replace",
        )
        return p.returncode, p.stdout + "\n" + p.stderr
    except subprocess.TimeoutExpired as e:
        out = (e.stdout or b"").decode("utf-8", "replace") if isinstance(e.stdout, bytes) else (e.stdout or "")
        err = (e.stderr or b"").decode("utf-8", "replace") if isinstance(e.stderr, bytes) else (e.stderr or "")
        return None, out + "\n" + err + "\n[TIMEOUT %ss]" % timeout


def count_errors(text):
    return len(re.findall(r"(?m)^!", text))


def first_errors(text, n=3):
    lines = re.findall(r"(?m)^!.*$", text)
    return lines[:n]


def auto_pkg_line(text):
    m = re.search(r"\[auto-pkg\] 已物化 (\d+) 个包、(\d+) 个文件", text)
    return "%s包/%s文件" % (m.group(1), m.group(2)) if m else ""


def run_ntex(name, work, vendor):
    """ntex-dvi --auto-pkg 一跑。PASS 判据：0 错 + out.dvi 存在。"""
    d = work / "ntex" / name
    d.mkdir(parents=True, exist_ok=True)
    rc, out = run(
        [NTEX, FIXTURES / (name + ".tex"), "out.dvi",
         "--auto-pkg", "--pkg-tlpdb", TLPDB, "--pkg-root", TLROOT,
         "--pkg-vendor-dir", vendor],
        cwd=d, timeout=TIMEOUT_NTEX,
    )
    dvi = d / "out.dvi"
    return {
        "rc": rc, "errors": count_errors(out),
        "dvi": dvi.exists(), "auto_pkg": auto_pkg_line(out),
        "excerpt": first_errors(out), "log": out,
    }


def run_gt(name, work):
    """pdfTeX GT 两趟（第二趟 aux 已稳）；错误数取第二趟 log。"""
    d = work / "gt" / name
    d.mkdir(parents=True, exist_ok=True)
    run([GT_BIN, "-interaction=nonstopmode", FIXTURES / (name + ".tex")],
        cwd=d, timeout=TIMEOUT_GT)
    rc, out = run([GT_BIN, "-interaction=nonstopmode", FIXTURES / (name + ".tex")],
                  cwd=d, timeout=TIMEOUT_GT)
    log = ""
    lf = d / (name + ".log")
    if lf.exists():
        log = lf.read_text(errors="replace")
    errors = count_errors(log) or count_errors(out)
    pdf = d / (name + ".pdf")
    return {"rc": rc, "errors": errors, "pdf": pdf.exists(),
            "excerpt": first_errors(out + "\n" + log), "log": out + "\n" + log,
            "pdf_path": pdf}


def dvi_to_pdf(name, work):
    d = work / "ntex" / name
    if not (d / "out.dvi").exists():
        return False
    rc, _ = run([DVIPDFMX, "-o", "out.pdf", "out.dvi"], cwd=d, timeout=TIMEOUT_PDF)
    return rc == 0 and (d / "out.pdf").exists()


def pdf_words(path):
    doc = fitz.open(str(path))
    words = []
    for page in doc:
        words.extend(page.get_text().split())
    n = doc.page_count
    doc.close()
    return n, words


def text_ratio(gt_pdf, ntex_pdf):
    """词序列相似度（difflib ratio，0..1）；页数差一并回报。"""
    gp, gw = pdf_words(gt_pdf)
    np_, nw = pdf_words(ntex_pdf)
    if not gw and not nw:
        return 1.0, gp, np_
    ratio = difflib.SequenceMatcher(None, gw, nw).ratio()
    return ratio, gp, np_


def pix_diff(gt_pdf, ntex_pdf, dpi=96, tol=8):
    """灰度位图差异率：|Δ|>tol 的像素占比（按页数较少一方逐页平均）。

    页面尺寸不合（如 GT 被 hyperref/xcolor 翻成 letter 而 NTex/dvipdfmx 保持
    TinyTeX 缺省 A4）时按左上交集裁剪后对照，返回 (差异率, 尺寸不合?)，
    不再把尺寸差冒充成 100% 内容分歧。
    """
    a, b = fitz.open(str(gt_pdf)), fitz.open(str(ntex_pdf))
    diffs, mismatch = [], False
    for i in range(min(a.page_count, b.page_count)):
        pa = a[i].get_pixmap(dpi=dpi, colorspace=fitz.csGRAY)
        pb = b[i].get_pixmap(dpi=dpi, colorspace=fitz.csGRAY)
        if (pa.width, pa.height) != (pb.width, pb.height):
            mismatch = True
        w, h = min(pa.width, pb.width), min(pa.height, pb.height)
        sa, sb = pa.samples, pb.samples
        if (w, h) != (pa.width, pa.height):
            sa = bytes(sa[y * pa.width + x] for y in range(h) for x in range(w))
        if (w, h) != (pb.width, pb.height):
            sb = bytes(sb[y * pb.width + x] for y in range(h) for x in range(w))
        bad = sum(1 for x, y in zip(sa, sb) if abs(x - y) > tol)
        diffs.append(bad / max(1, len(sa)))
    a.close()
    b.close()
    return (sum(diffs) / max(1, len(diffs)) if diffs else None), mismatch


def head_sha():
    try:
        return subprocess.run(["git", "rev-parse", "--short", "HEAD"], cwd=str(REPO),
                              stdout=subprocess.PIPE, text=True).stdout.strip()
    except OSError:
        return "?"


def run_negative(pkg, work):
    """缺包负例：期待「自动取料失败」含包名与反查指引，且不产 DVI。"""
    d = work / "ntex" / ("_neg-" + pkg)
    d.mkdir(parents=True, exist_ok=True)
    tex = d / "neg.tex"
    tex.write_text(
        "\\documentclass{article}\n\\usepackage{%s}\n"
        "\\begin{document}\nnegative probe: %s must not load.\n"
        "\\end{document}\n" % (pkg, pkg))
    rc, out = run(
        [NTEX, tex, "out.dvi", "--auto-pkg", "--pkg-tlpdb", TLPDB,
         "--pkg-root", TLROOT, "--pkg-vendor-dir", work / "vendor"],
        cwd=d, timeout=TIMEOUT_NTEX,
    )
    guided = ("自动取料失败" in out) and (pkg in out) and ("反查" in out)
    no_dvi = not (d / "out.dvi").exists()
    return {"pkg": pkg, "guided": guided, "no_dvi": no_dvi,
            "msg": out.strip().splitlines()[-1] if out.strip() else ""}


def verdict_of(r):
    if r["ntex"]["dvi"] and r["ntex"]["errors"] == 0:
        return "PASS"
    if not r["ntex"]["dvi"] and r["ntex"]["errors"] == 0:
        return "NO-DVI"
    return "FAIL(%d)" % r["ntex"]["errors"]


def main():
    ap = argparse.ArgumentParser(description="宏包 CI 回归集双跑矩阵")
    ap.add_argument("--pix", action="store_true", help="灰度位图差异率对照")
    ap.add_argument("--only", help="逗号分隔的包名子集")
    ap.add_argument("--work", default="/tmp/ntex-pkgreg", help="工作目录（默认清空重建）")
    ap.add_argument("--keep", action="store_true", help="保留工作目录中间产物")
    ap.add_argument("--no-report", action="store_true", help="不写 docs/pkg-regression-baseline.md")
    ap.add_argument("--json", help="另存机器可读摘要 JSON")
    args = ap.parse_args()

    for path, label in [(NTEX, "ntex-dvi"), (GT_BIN, "pdflatex"),
                        (TLPDB, "texlive.tlpdb"), (TLROOT, "TL 树"),
                        (FIXTURES, "fixtures 目录")]:
        if not Path(path).exists():
            sys.exit("缺 %s：%s" % (label, path))
    ensure_pymupdf()

    names = [n.strip() for n in args.only.split(",")] if args.only else PACKAGES
    for n in names:
        if not (FIXTURES / (n + ".tex")).exists():
            sys.exit("无样张：%s" % (FIXTURES / (n + ".tex")))

    work = Path(args.work)
    if work.exists() and not args.keep:
        shutil.rmtree(work)
    work.mkdir(parents=True, exist_ok=True)
    vendor = work / "vendor"

    rows = []
    for name in names:
        print("== %-10s ntex ... " % name, end="", flush=True)
        ntex = run_ntex(name, work, vendor)
        print("错=%d dvi=%s" % (ntex["errors"], "有" if ntex["dvi"] else "无"), end="", flush=True)
        ntex_pdf = work / "ntex" / name / "out.pdf"
        ntex["pdf"] = dvi_to_pdf(name, work) if ntex["dvi"] else False
        print(" pdf=%s | gt ... " % ("有" if ntex["pdf"] else "无"), end="", flush=True)
        gt = run_gt(name, work)
        print("错=%d pdf=%s" % (gt["errors"], "有" if gt["pdf"] else "无"), end="", flush=True)

        ratio = pages_n = pages_g = pix = None
        pix_mm = False
        if fitz and gt["pdf"] and ntex["pdf"]:
            ratio, pages_g, pages_n = text_ratio(gt["pdf_path"], ntex_pdf)
            if args.pix:
                pix, pix_mm = pix_diff(gt["pdf_path"], ntex_pdf)
        elif fitz and gt["pdf"]:
            gdoc = fitz.open(str(gt["pdf_path"]))
            pages_g = gdoc.page_count
            gdoc.close()

        row = {"name": name, "ntex": ntex, "gt": gt,
               "ratio": ratio, "pix": pix, "pix_mismatch": pix_mm,
               "pages_ntex": pages_n, "pages_gt": pages_g,
               "verdict": verdict_of(
                   {"ntex": ntex})}
        rows.append(row)
        print(" | %s 文本层=%s" % (
            row["verdict"],
            "%.1f%%" % (ratio * 100) if ratio is not None else "n/a"))

    print()
    print("%-10s %8s %5s %5s %7s %5s %9s %9s %s" % (
        "包", "NTex错", "DVI", "页", "GT错", "GT页", "文本层", "像素差", "判定"))
    for r in rows:
        print("%-10s %8d %5s %5s %7d %5s %9s %9s %s" % (
            r["name"], r["ntex"]["errors"], "有" if r["ntex"]["dvi"] else "无",
            r["pages_ntex"] if r["pages_ntex"] is not None else "-",
            r["gt"]["errors"],
            r["pages_gt"] if r["pages_gt"] is not None else "-",
            "%.1f%%" % (r["ratio"] * 100) if r["ratio"] is not None else "n/a",
            ("%.2f%%%s" % (r["pix"] * 100, "†" if r.get("pix_mismatch") else ""))
            if r["pix"] is not None else ("-" if not args.pix else "n/a"),
            r["verdict"]))
    npass = sum(1 for r in rows if r["verdict"] == "PASS")
    print("\nPASS %d/%d（判定口径：ntex-dvi 写出 DVI 且 0 错）" % (npass, len(rows)))

    negs = []
    if not args.only or any(n in args.only for n in NEGATIVE_PKGS):
        for pkg in NEGATIVE_PKGS:
            neg = run_negative(pkg, work)
            negs.append(neg)
            print("负例 %-8s %s（报错含包名+反查指引=%s，未产 DVI=%s）" % (
                pkg, "PASS" if (neg["guided"] and neg["no_dvi"]) else "FAIL",
                neg["guided"], neg["no_dvi"]))
    if args.pix and any(r.get("pix_mismatch") for r in rows):
        print("† 页面尺寸不合（GT 被该包翻成 letter，NTex/dvipdfmx 保持 TinyTeX 缺省 A4），"
              "像素差按左上交集裁剪对照。")

    if args.json:
        slim = [{"name": r["name"], "verdict": r["verdict"],
                 "ntex_errors": r["ntex"]["errors"], "ntex_dvi": r["ntex"]["dvi"],
                 "ntex_pages": r["pages_ntex"], "gt_errors": r["gt"]["errors"],
                 "gt_pages": r["pages_gt"],
                 "text_ratio": r["ratio"], "pix_diff": r["pix"],
                 "pix_mismatch": r.get("pix_mismatch", False),
                 "auto_pkg": r["ntex"]["auto_pkg"],
                 "ntex_excerpt": r["ntex"]["excerpt"],
                 "gt_excerpt": r["gt"]["excerpt"]} for r in rows]
        payload = {"packages": slim,
                   "negative": [{"pkg": n["pkg"], "guided": n["guided"],
                                 "no_dvi": n["no_dvi"]} for n in negs]}
        Path(args.json).write_text(__import__("json").dumps(payload, ensure_ascii=False, indent=1))
        print("JSON 摘要 → %s" % args.json)

    if not args.no_report:
        write_report(rows, npass, negs)
        print("基线报告 → %s" % REPORT)

    if not args.keep:
        shutil.rmtree(work, ignore_errors=True)
    return 0


def write_report(rows, npass, negs=()):
    """基线报告：矩阵 + 分级注记；只陈述测量事实，不为绿造假。"""
    L = []
    L.append("# 宏包 CI 回归基线（auto-pkg 放量）\n")
    L.append("- 日期：%s；HEAD：`%s`；二进制：`target/debug/ntex-dvi`（mtime %s）"
             % (date.today().isoformat(), head_sha(),
                __import__("datetime").datetime.fromtimestamp(NTEX.stat().st_mtime)
                .strftime("%Y-%m-%d %H:%M")))
    L.append("- 跑法：`scripts/pkg-regression.py`——ntex-dvi `--auto-pkg --pkg-tlpdb "
             "~/.TinyTeX/tlpkg/texlive.tlpdb --pkg-root ~/.TinyTeX` vs pdfTeX GT "
             "`-interaction=nonstopmode` 两趟；顺序执行、单文档 timeout 240s；本地 TL 树，无网络依赖。")
    L.append("- 判定口径：**PASS = ntex-dvi 写出 DVI 且 0 错**（`^!` 行计数）。"
             "GT 对照分级报告：GT 也错的构造不算 NTex 缺口。")
    L.append("- 文本层 = pymupdf 词序列 difflib 相似度（GT PDF vs DVI→dvipdfmx PDF）。")
    L.append("- 矩阵替换说明：任务单所列 caption/parskip 在本机 TL 树（2024basic）与 TLPDB "
             "均缺席，双跑皆无源可比：parskip 弃用，caption 转作缺包负例（见下）；"
             "以 array/xcolor/tabularx/graphicx/etoolbox 替补凑足矩阵。")
    L.append("- auto-pkg 通路说明：13 包的 `.sty` 均已被既有检索路径（assets + 宿主 TinyTeX 树）"
             "命中，auto-pkg 探测零缺失、取料为 no-op——本机放量验收面因此落在"
             "「缺包即报 + 反查指引」契约（负例）与全矩阵 0 错判定上。")
    L.append("")
    L.append("| 包 | NTex 错 | DVI | NTex 页 | GT 错 | GT 页 | 文本层 | 像素差 | 判定 | auto-pkg 取料 |")
    L.append("|---|---|---|---|---|---|---|---|---|---|")
    for r in rows:
        L.append("| %s | %d | %s | %s | %d | %s | %s | %s | **%s** | %s |" % (
            r["name"], r["ntex"]["errors"],
            "有" if r["ntex"]["dvi"] else "无",
            r["pages_ntex"] if r["pages_ntex"] is not None else "-",
            r["gt"]["errors"],
            r["pages_gt"] if r["pages_gt"] is not None else "-",
            "%.1f%%" % (r["ratio"] * 100) if r["ratio"] is not None else "n/a",
            ("%.2f%%%s" % (r["pix"] * 100, "†" if r.get("pix_mismatch") else ""))
            if r["pix"] is not None else "—",
            r["verdict"],
            r["ntex"]["auto_pkg"] or "（源已在路径）"))
    L.append("")
    if any(r.get("pix_mismatch") for r in rows):
        L.append("† 页面尺寸不合：GT 裸 article 在本机 TinyTeX 缺省 **A4**，载入 hyperref/xcolor "
                 "后 GT 侧被翻成 **letter**（612×792），NTex/dvipdfmx 侧保持 A4——该列像素差"
                 "已按左上交集裁剪对照，不代表内容全分歧。")
        L.append("")
    L.append("**PASS %d/%d**。\n" % (npass, len(rows)))

    if negs:
        L.append("## auto-pkg 缺包负例（契约验收）\n")
        for neg in negs:
            ok = neg["guided"] and neg["no_dvi"]
            L.append("- **%s**（TLPDB 无提供者）：%s——报错含包名+反查指引=`%s`、未产 DVI=`%s`。"
                     % (neg["pkg"], "✅ 通过" if ok else "❌ 未过",
                        neg["guided"], neg["no_dvi"]))
        L.append("")

    fails = [r for r in rows if r["verdict"] != "PASS"]
    if fails:
        L.append("## 非 PASS 明细（首错摘录）\n")
        for r in fails:
            L.append("### %s — %s（NTex %d 错，GT %d 错）"
                     % (r["name"], r["verdict"], r["ntex"]["errors"], r["gt"]["errors"]))
            for line in r["ntex"]["excerpt"]:
                L.append("- NTex：`%s`" % line.replace("`", "'"))
            for line in r["gt"]["excerpt"][:2]:
                L.append("- GT：`%s`" % line.replace("`", "'"))
            L.append("")

    REPORT.write_text("\n".join(L) + "\n")


if __name__ == "__main__":
    sys.exit(main())
