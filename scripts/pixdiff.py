#!/usr/bin/env python3
"""NTex 像素级对比验收基础设施（2026-09-06 建）。

对比链：
  .tex → NTex (ntex-dvi → ntex-pdf) → ntex.pdf
  .tex → TinyTeX (tex → dvipdfmx 或 pdflatex) → ref.pdf
  ref.pdf vs ntex.pdf → 同 DPI 位图 → numpy 逐像素 diff

用法：
  scripts/pixdiff.sh file.tex              # 单文件：全链对比，输出差异率
  scripts/pixdiff.sh file.tex --save       # 附带保存差区截图 /tmp/pixdiff/<stem>/
  scripts/pixdiff.sh --corpus [分类]        # 批量：fixtures/corpus/<分类>/ 全跑
  scripts/pixdiff.sh --baseline            # 重算 corpus 基线（TinyTeX golden）

退出码：0 = 全部 0 差异；1 = 有差异/失败；2 = 环境缺失。
结果 JSON 追加写入 fixtures/corpus/pixdiff-report.json（每轮覆盖）。
"""
import os
import subprocess
import sys
import json
import shutil
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
PIX = os.path.expanduser("~/.venvs/pixtools/bin/python")
OUTDIR = Path("/tmp/pixdiff")
REPORT = REPO / "fixtures/corpus/pixdiff-report.json"
TIMEOUT = 120

for binname in ("tex", "pdftex", "dvipdfmx"):
    if not shutil.which(binname):
        # TinyTeX 装在 ~/.local/bin 或 ~/.TinyTeX/bin/x86_64-linux
        for p in (Path.home() / ".local/bin", *(Path.home() / ".TinyTeX/bin").glob("*")):
            if (p / binname).exists():
                os.environ["PATH"] = f"{p}:{os.environ['PATH']}"
                break
if not (shutil.which("tex") and shutil.which("dvipdfmx")):
    print("环境缺失：TinyTeX 未安装（tex/dvipdfmx 不在 PATH）", file=sys.stderr)
    sys.exit(2)
if not os.path.exists(PIX):
    print(f"环境缺失：pixtools venv 不存在 {PIX}", file=sys.stderr)
    sys.exit(2)

ENV = dict(os.environ)
ENV["NTEX_TFM_DIR"] = ENV.get("NTEX_TFM_DIR", os.path.expanduser("~/.ntex-fonts"))


def run(cmd, cwd, timeout=TIMEOUT):
    return subprocess.run(cmd, cwd=cwd, env=ENV, timeout=timeout,
                          capture_output=True, text=True)


def tex_to_dvi(tex: Path, engine="tex"):
    """TeX → DVI（默认 tex 引擎；latex 类源码由调用方传 pdflatex 路径处理）"""
    r = run([engine, "-interaction=nonstopmode", tex.name], tex.parent)
    dvi = tex.with_suffix(".dvi")
    return (dvi.exists()), r.stdout + r.stderr


def dvi_to_pdf_via_dvipdfmx(dvi: Path):
    r = run(["dvipdfmx", "-o", dvi.with_suffix(".ref.pdf").name, dvi.name], dvi.parent)
    return dvi.with_suffix(".ref.pdf").exists(), r.stdout + r.stderr


def ntex_pipeline(tex: Path):
    """NTex 全链：ntex-dvi → ntex-pdf"""
    r1 = run(["cargo", "run", "-q", "-p", "ntex-dvi", "--", str(tex)], REPO, timeout=TIMEOUT + 60)
    dvi = tex.with_suffix(".dvi")
    if not dvi.exists():
        return None, f"DVI: {tail_err(r1.stdout + r1.stderr)}"
    r2 = run(["cargo", "run", "-q", "-p", "ntex-pdf", "--", str(dvi)], REPO)
    pdf = dvi.with_suffix(".pdf")
    if not pdf.exists():
        return None, f"PDF: {tail_err(r2.stdout + r2.stderr)}"
    return pdf, None


def check_plain_pitfalls(tex_src: str) -> str:
    """检测 NTex 已知限制导致的假差异源，给出提示（引擎无 plain 宏层）。

    - \\tt/\\cmr 等 plain 控制词未定义 → NTex 侧静默产出 0 字体 DVI（空白页），
      而参考 tex 有 plain format 预载。样例须显式 \\font\\cmr=cmr10。
    - 无 \\font 声明的 .tex 在 NTex 下 DVI 会显示「0 字体」——先提示再跑，省一轮排障。
    """
    tips = []
    if "\\font" not in tex_src and ("\\" in tex_src):
        tips.append("⚠ 源码无 \\font 声明：NTex 无 plain 宏层，\\tt/\\rm 等未定义会产出"
                    " 0 字体 DVI（空白页）。参考样例请显式写 \\font\\x=cmr10。")
    tips.append("📌 已知系统性差异（非 bug，验收时按类豁免）：\n"
                "  ① plain 页码：参考 tex 的输出例程投 \\folio（页码数字），NTex 无输出例程不投——"
                "页面底部差异属预期；\n"
                "  ② 垂直基线：NTex 无 \\topskip 注入时首行贴顶（y≈0），参考从 \\topskip 起——"
                "整体位移类差异先查 topskip。")
    return "\n".join(tips)


def ref_pipeline(tex: Path):
    """TinyTeX 参考链：tex → DVI → dvipdfmx → ref.pdf"""
    ok, log = tex_to_dvi(tex)
    if not ok:
        return None, f"参考 tex: {tail_err(log)}"
    ok, log = dvi_to_pdf_via_dvipdfmx(tex.with_suffix(".dvi"))
    if not ok:
        return None, f"dvipdfmx: {tail_err(log)}"
    return tex.with_suffix(".ref.pdf"), None


def tail_err(log: str, n: int = 3):
    lines = [l for l in log.splitlines() if l.strip() and not l.startswith("(")]
    return " | ".join(lines[-n:])[:200] if lines else "无输出"


def pixdiff(ref: Path, ntex: Path, dpi=144, save_stem=None):
    """像素 diff：返回 (diff_ratio, total_px, diff_px, hotspots)"""
    code = f"""
import pymupdf, numpy as np, json, sys
a = pymupdf.open({str(ref)!r}); b = pymupdf.open({str(ntex)!r})
if len(a) != len(b):
    print(json.dumps({{"pages": [len(a), len(b)], "error": "页数不同"}})); sys.exit()
worst = {{"ratio": 0.0, "total": 0, "diff": 0}}
hotspots = []
for i in range(len(a)):
    pa = a[i].get_pixmap(dpi={dpi}); pb = b[i].get_pixmap(dpi={dpi})
    va = np.frombuffer(pa.samples, dtype=np.uint8).reshape(pa.height, pa.width, pa.n)
    vb = np.frombuffer(pb.samples, dtype=np.uint8).reshape(pb.height, pb.width, pb.n)
    if va.shape != vb.shape:
        print(json.dumps({{"error": f"页 {{i+1}} 尺寸不同 {{va.shape}} vs {{vb.shape}}"}})); sys.exit()
    d = (va != vb).any(axis=2)
    n = int(d.sum()); total = va.shape[0]*va.shape[1]
    if n > worst["diff"]:
        worst = {{"ratio": n/total, "total": total, "diff": n}}
    if n and {save_stem is not None!r}:
        ys, xs = np.where(d)
        hotspots.append({{"page": i+1, "bbox": [int(xs.min()), int(ys.min()), int(xs.max()), int(ys.max())]}})
print(json.dumps({{**worst, "hotspots": hotspots}}))
"""
    r = run([PIX, "-c", code], REPO)
    try:
        return json.loads(r.stdout.strip().splitlines()[-1])
    except Exception:
        return {"error": f"pixdiff 子进程失败: {tail_err(r.stdout + r.stderr)}"}


def one_file(tex: Path, save=False):
    stem = tex.stem
    work = OUTDIR / stem
    work.mkdir(parents=True, exist_ok=True)
    # 复制到独立工作目录（防 transcript 覆盖写互扰）
    t = work / tex.name
    shutil.copy(tex, t)
    tip = check_plain_pitfalls(t.read_text(errors="replace"))
    if tip:
        print(tip)
    ref, err1 = ref_pipeline(t)
    ntex, err2 = ntex_pipeline(t)
    if err1 or err2 or ref is None or ntex is None:
        return {"file": str(tex), "ok": False, "error": err1 or err2}
    res = pixdiff(ref, ntex, save_stem=stem if save else None)
    if isinstance(res, dict):
        res.update({"file": str(tex), "ok": res.get("error") is None and res.get("diff", 1) == 0})
    return res


def corpus(category=None):
    root = REPO / "fixtures/corpus"
    cats = [category] if category else ["plain", "latex", "math"]
    results = []
    for cat in cats:
        for tex in sorted((root / cat).glob("*.tex")):
            print(f"== {cat}/{tex.name} …", flush=True)
            r = one_file(tex)
            r["category"] = cat
            results.append(r)
            if r["ok"]:
                print("   PIXEL-IDENTICAL ✅")
            else:
                print(f"   差异 {r.get('diff','?')}/{r.get('total','?')} ({100*r.get('ratio',1):.3f}%) {r.get('error','')}")
    REPORT.write_text(json.dumps(results, ensure_ascii=False, indent=1))
    ident = sum(1 for r in results if r["ok"])
    print(f"\n像素级一致 {ident}/{len(results)}（报告 → {REPORT}）")
    return 0 if ident == len(results) else 1


def main():
    args = sys.argv[1:]
    if not args:
        print(__doc__)
        return 2
    if args[0] == "--corpus":
        return corpus(args[1] if len(args) > 1 else None)
    tex = Path(args[0])
    if not tex.exists():
        print(f"找不到 {tex}", file=sys.stderr)
        return 2
    r = one_file(tex, save="--save" in args)
    if r["ok"]:
        print(f"✅ {tex} 像素级一致（{r['total']} px）")
        return 0
    if "error" in r:
        print(f"❌ {tex}: {r['error']}")
    else:
        print(f"❌ {tex}: 差异 {r['diff']}/{r['total']}（{100*r['ratio']:.4f}%）hotspots={r.get('hotspots')}")
    return 1


if __name__ == "__main__":
    sys.exit(main())
