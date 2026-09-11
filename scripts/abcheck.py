#!/usr/bin/env python3
"""双引擎差分对拍器：pdfTeX 原生（ground truth）vs NTex，逐字对照。

## 为什么需要它

NTex 的多数定位工作靠「手写探针 .tex + 目视比对两份输出」。实测踩坑：
探针本身写错（`\\let\\else:\\else` 配对、`%` 续行、少写 plain 定义）
浪费的轮次往往多于真定位。本工具把对拍自动化：
**一次跑两侧 → 抽取关注信号 → 逐行 diff → 首个差异点带上下文**。

## 用法

    # 基础：比对转录全文
    scripts/abcheck.py probe.tex

    # 只看关注信号的 trace（`\message`/`\typeout` 输出行）
    scripts/abcheck.py probe.tex --trace

    # 带 plain 预载（NTex 侧 `\\input plain`，pdfTeX 侧 `\\input plain`）
    scripts/abcheck.py probe.tex --plain

    # pdfTeX 用 -ini（INITEX 初表，供语义初值对拍）
    scripts/abcheck.py probe.tex --initex

    # 自定义抽取正则
    scripts/abcheck.py probe.tex --grep '^\\[.*\\]$'

    # 保存两侧完整输出
    scripts/abcheck.py probe.tex --dump /tmp/ab/

## 退出码
    0 = 两侧逐行一致    1 = 有差异    2 = 环境/运行错误

## 环境
    需要 ~/.local/bin/pdftex（TinyTeX）与 target/debug/ntex-dvi
    先 `cargo build -p ntex-dvi`。
"""
import argparse
import difflib
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
PDFTEX = Path.home() / ".local" / "bin" / "pdftex"
NTEX_DVI = REPO / "target" / "debug" / "ntex-dvi"
TIMEOUT = 60

# 两侧都必须屏蔽的噪声（路径/时间戳/版本横幅）
NOISE_PATTERNS = [
    re.compile(r"^This is pdfTeX"),
    re.compile(r"^entering extended mode"),
    re.compile(r"^restricted \\write18"),
    re.compile(r"^\(.*\.tex$"),
    re.compile(r"^Transcript written on"),
    re.compile(r"^Output written on"),
    re.compile(r"^No pages of output"),
    re.compile(r"^\)\s*$"),
    re.compile(r"\.pfb>?\s*$"),
    re.compile(r"^\[1\{.*\}\s*\]$"),
]


def clean(text: str) -> list[str]:
    out = []
    for line in text.splitlines():
        if any(p.search(line) for p in NOISE_PATTERNS):
            continue
        out.append(line.rstrip())
    return out


def run_pdftex(tex: Path, workdir: Path, plain: bool, initex: bool) -> tuple[int, str]:
    """跑 pdfTeX。返回 (exit, 转录文本)。"""
    src = tex.read_text()
    # pdfTeX 侧：--plain 时前置 \input plain；initex 时用 -ini
    if plain:
        src = "\\input plain\n" + src
    staged = workdir / tex.name
    staged.write_text(src)
    cmd = [str(PDFTEX), "-interaction=nonstopmode"]
    if initex:
        cmd.append("-ini")
    cmd.append(staged.name)
    try:
        r = subprocess.run(cmd, capture_output=True, text=True,
                           timeout=TIMEOUT, cwd=str(workdir))
    except subprocess.TimeoutExpired:
        return -1, f"[abcheck] pdfTeX TIMEOUT {TIMEOUT}s"
    log = workdir / (staged.stem + ".log")
    text = log.read_text(errors="replace") if log.is_file() else (r.stdout + r.stderr)
    return r.returncode, text


def run_ntex(tex: Path, workdir: Path, plain: bool) -> tuple[int, str]:
    """跑 ntex-dvi（转录走 stderr）。--no-plain 关预载。"""
    if not NTEX_DVI.is_file():
        return -2, ("[abcheck] 缺 ntex-dvi 二进制：先 `cargo build -p ntex-dvi`\n"
                    f"  期望路径 {NTEX_DVI}")
    cmd = [str(NTEX_DVI), str(tex)]
    if not plain:
        cmd.append("--no-plain")
    env = dict(os.environ)
    env.setdefault("NTEX_TFM_DIR", str(Path.home() / ".ntex-fonts"))
    try:
        r = subprocess.run(cmd, capture_output=True, text=True,
                           timeout=TIMEOUT, cwd=str(workdir), env=env)
    except subprocess.TimeoutExpired:
        return -1, f"[abcheck] ntex TIMEOUT {TIMEOUT}s"
    return r.returncode, r.stdout + r.stderr


def extract(lines: list[str], trace: bool, grep: str | None) -> list[str]:
    """抽取对拍信号。

    --trace 模式：TeX 的 `\\message` 产物在 pdfTeX log 里**内联混排**
    （`(./t.tex MEANING=macro:if-> DONE )`），在 NTex 侧是按行落的。
    故先按标记正则从行内切出片段，再做**逐片段**比对——两侧行结构差异
    不干扰信号比对，只比信号本身。
    """
    if grep:
        rx = re.compile(grep)
        return [l for l in lines if rx.search(l)]
    if not trace:
        return lines
    # 标记片段：`KEY=value` 或 `== ... ==`。
    # 两侧行结构不同（pdfTeX 内联混排、NTex 拼接无换行），故**按标记序列**
    # 抽取：先找所有 `KEY=` 起点，值截到下一个 `KEY=` 起点或行尾（去尾噪）。
    key_rx = re.compile(r"[A-Za-z_][A-Za-z_0-9]{0,19}=")
    out = []
    for line in lines:
        for m in re.finditer(r"==[^=()]*?==", line):
            out.append(m.group(0))
        starts = [m.start() for m in key_rx.finditer(line)]
        for i, s in enumerate(starts):
            e = starts[i + 1] if i + 1 < len(starts) else len(line)
            frag = line[s:e].strip().rstrip(")")
            # 去尾噪：值截到首个空白（pdfTeX log 内联会跟 ` ) No pages...` 等）
            frag = frag.split()[0].rstrip(")") if frag.split() else ""
            if frag:
                out.append(frag)
    return out


def main() -> int:
    ap = argparse.ArgumentParser(description="pdfTeX vs NTex 差分对拍")
    ap.add_argument("tex", help="探针 .tex")
    ap.add_argument("--plain", action="store_true", help="两侧都 \\input plain")
    ap.add_argument("--initex", action="store_true", help="pdfTeX 用 -ini")
    ap.add_argument("--trace", action="store_true", help="只比对关注信号行")
    ap.add_argument("--grep", help="自定义抽取正则")
    ap.add_argument("--dump", help="把两侧完整输出存到该目录")
    ap.add_argument("--context", type=int, default=3, help="差异上下文行数")
    args = ap.parse_args()

    tex = Path(args.tex).resolve()
    if not tex.is_file():
        print(f"[abcheck] 找不到 {tex}", file=sys.stderr)
        return 2
    if not PDFTEX.is_file():
        print(f"[abcheck] 找不到 pdfTeX：{PDFTEX}", file=sys.stderr)
        return 2

    workdir = Path(tempfile.mkdtemp(prefix="abcheck-"))
    try:
        px_exit, px_raw = run_pdftex(tex, workdir, args.plain, args.initex)
        nx_exit, nx_raw = run_ntex(tex, workdir, args.plain)

        if args.dump:
            d = Path(args.dump)
            d.mkdir(parents=True, exist_ok=True)
            (d / "pdftex.txt").write_text(px_raw)
            (d / "ntex.txt").write_text(nx_raw)

        px = extract(clean(px_raw), args.trace, args.grep)
        nx = extract(clean(nx_raw), args.trace, args.grep)

        same = px == nx
        print(f"== abcheck: {tex.name}"
              f"{' [--plain]' if args.plain else ''}"
              f"{' [-ini]' if args.initex else ''}"
              f"{' [--trace]' if args.trace else ''}")
        print(f"   pdfTeX exit={px_exit} 行数={len(px)} | NTex exit={nx_exit} 行数={len(nx)}")

        if same:
            print("   ✅ 两侧逐行一致")
            return 0

        print("   ❌ 有差异（unified diff，'-' pdfTeX / '+' NTex）：\n")
        diff = difflib.unified_diff(px, nx, fromfile="pdftex", tofile="ntex",
                                    lineterm="", n=args.context)
        for line in diff:
            print("   " + line)
        # 首个差异点
        for i, (a, b) in enumerate(zip(px, nx)):
            if a != b:
                print(f"\n   首个差异 @ 第 {i+1} 行：")
                print(f"     pdfTeX: {a[:160]}")
                print(f"     NTex  : {b[:160]}")
                break
        else:
            print(f"\n   行数不等：pdfTeX {len(px)} vs NTex {len(nx)}"
                  f"（首个多出行见 diff）")
        return 1
    finally:
        shutil.rmtree(workdir, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
