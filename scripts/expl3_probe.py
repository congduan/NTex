#!/usr/bin/env python3
"""expl3 截断载入探针：cut N 行 + 自定义 stub，一键量化错误面。

## 为什么需要它

expl3 载入攻坚（2689→0）的定位循环高度重复：截断 expl3-code.tex 到第 N 行、
拼接探针 stub、跑 NTex（必要时双引擎）、数错误签名、抓首错现场。本会话
手工重复 20+ 次才固化为工具。三个子命令覆盖三种节奏：

    # 1) 分布看板：当前代码在完整 expl3 上的错误签名分布
    scripts/expl3_probe.py dist

    # 2) 首错现场：第一个错误前后 ±12 行（含 l.N 行号）
    scripts/expl3_probe.py first

    # 3) 截断二分：错误类型首次出现的行号区间（定位引入点）
    scripts/expl3_probe.py bisect --error "Missing endcsname" --lo 18160 --hi 25848

    # 任意 cut + 自定义 stub 的手动模式
    scripts/expl3_probe.py run --cut 21291 --stub stub.tex

## 对拍模式（--pdfTeX）

    双引擎同跑，错误签名分布并排显示——「pdfTeX 0 错 / NTex N 错」的
    差分现场一步到位（oracle 先行的纪律要求，防修不存在的问题）。

## 退出码
    0 = 无错误    1 = 有错误    2 = 环境错误
"""
import argparse
import collections
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
PDFTEX = Path.home() / ".local" / "bin" / "pdftex"
# expl3 备料优先级：/tmp 缓存 → 仓库 fixtures（2026-09-13 入库，防 /tmp
# 随整机重启丢失）→ 可用环境变量 EXPL3_SRC 覆盖。
_FIX_EXPL3 = REPO / "fixtures" / "l3kernel"
EXPL3 = (
    Path("/tmp/l3kernel-tests/testfiles/expl3-code.tex")
    if Path("/tmp/l3kernel-tests/testfiles/expl3-code.tex").exists()
    else _FIX_EXPL3 / "expl3-code.tex"
)
GEN = (
    Path("/tmp/iw/exgeneric.tex")
    if Path("/tmp/iw/exgeneric.tex").exists()
    else _FIX_EXPL3 / "exgeneric.tex"
)

# 载入完成判据 stub（探测 LOADED-OK 与否）
DEFAULT_TAIL = "\\end\n"


def build_src(cut: int, stub: str = "") -> str:
    """exgeneric 模板：expl3-code 截断到 cut 行 + stub。"""
    gen_text = GEN.read_text()
    inp_line = next(l for l in gen_text.splitlines() if "expl3-code" in l)
    code_lines = EXPL3.read_text().splitlines(keepends=True)
    body = "".join(code_lines[:cut]) + stub
    return gen_text.replace(inp_line, body)


def run_engine(engine: str, wd: Path) -> str:
    if engine == "pdftex":
        cmd = [str(PDFTEX), "-interaction=nonstopmode", "s.tex"]
        r = subprocess.run(cmd, cwd=wd, capture_output=True, timeout=300)
        return (wd / "s.log").read_text(errors="replace")
    ntex = (REPO / "target" / "debug" / "ntex-dvi").resolve()
    r = subprocess.run([str(ntex), "--input-path", str(wd), str(wd / "s.tex")],
                       cwd=wd, capture_output=True, timeout=300)
    return (r.stdout + r.stderr).decode(errors="replace")


def run_cut(cut: int, stub: str, engines) -> dict:
    wd = Path(tempfile.mkdtemp(prefix=f"ep{cut}-"))
    try:
        src = build_src(cut, stub)
        (wd / "exgeneric.tex").write_text(src)
        (wd / "s.tex").write_text("\\input exgeneric.tex\n" + DEFAULT_TAIL)
        out = {}
        for eng in engines:
            out[eng] = run_engine(eng, wd)
        return out
    finally:
        shutil.rmtree(wd, ignore_errors=True)


def signatures(log: str) -> "collections.Counter[str]":
    return collections.Counter(
        l[:44] for l in log.splitlines() if l.startswith("! "))


def first_error_scene(log: str, window: int = 12) -> str:
    lines = log.splitlines()
    idx = next((k for k, l in enumerate(lines) if l.startswith("! ")), None)
    if idx is None:
        return "(无错误)"
    return "\n".join(lines[max(0, idx - 4):idx + window])


def cmd_dist(args):
    engines = ["pdftex", "ntex"] if args.pdftex else ["ntex"]
    outs = run_cut(args.cut, args.stub, engines)
    for eng, log in outs.items():
        sig = signatures(log)
        total = sum(sig.values())
        print(f"=== {eng}: 总错 {total} ===")
        for k, v in sig.most_common(12):
            print(f"{v:>6}  {k}")
    if len(outs) == 2:
        a, b = (signatures(v) for v in outs.values())
        if not b and a:
            print(">>> pdfTeX 0 错 / NTex 有错 —— 纯 NTex 缺陷，放心修")
        elif a == b:
            print(">>> 两侧签名一致（引擎无关，先查 stub/环境）")
    return 0 if not any(signatures(v) for v in outs.values()) else 1


def cmd_first(args):
    engines = ["pdftex", "ntex"] if args.pdftex else ["ntex"]
    outs = run_cut(args.cut, args.stub, engines)
    for eng, log in outs.items():
        print(f"=== {eng} 首错现场 (cut={args.cut}) ===")
        print(first_error_scene(log))
    return 1


def cmd_bisect(args):
    lo, hi = args.lo, args.hi

    def count(cut: int) -> int:
        log = run_cut(cut, "", ["ntex"])["ntex"]
        return sum(1 for l in log.splitlines() if l.startswith(args.error))

    if count(hi) == 0:
        print(f"cut={hi} 处 {args.error!r} 计数为 0——hi 太小或错误名不匹配")
        return 2
    while hi - lo > 1:
        mid = (lo + hi) // 2
        n = count(mid)
        print(f"cut={mid}: {args.error!r} × {n}")
        if n > 0:
            hi = mid
        else:
            lo = mid
    print(f"\n{args.error!r} 首现区间: ({lo}, {hi}]")
    code = EXPL3.read_text().splitlines()
    print(f"边界行 {hi}: {code[hi-1].strip()[:90]}")
    print(f"边界行 {lo+1}: {code[lo].strip()[:90]}")
    return 0


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)

    for name in ("dist", "first"):
        p = sub.add_parser(name)
        p.add_argument("--cut", type=int, default=25848,
                       help="expl3-code 截断行号（默认 25848 = tl_analysis 前）")
        p.add_argument("--stub", default="", help="追加到截断点后的探针语句")
        p.add_argument("--pdftex", action="store_true", help="双引擎对拍")

    p = sub.add_parser("bisect")
    p.add_argument("--error", required=True, help="错误签名前缀（如 '! Missing endcsname'）")
    p.add_argument("--lo", type=int, required=True)
    p.add_argument("--hi", type=int, required=True)

    p = sub.add_parser("run")
    p.add_argument("--cut", type=int, required=True)
    p.add_argument("--stub", default="")
    p.add_argument("--pdftex", action="store_true")

    args = ap.parse_args()
    if not EXPL3.exists() or not GEN.exists():
        print(f"环境缺失：需要 expl3-code.tex 与 exgeneric.tex（找过 /tmp 缓存与 {_FIX_EXPL3}）",
              file=sys.stderr)
        return 2
    return {"dist": cmd_dist, "first": cmd_first, "bisect": cmd_bisect}.get(
        args.cmd, lambda a: 2)(args)


if __name__ == "__main__":
    sys.exit(main())
