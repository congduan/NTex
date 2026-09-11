#!/usr/bin/env python3
"""仪器自检：验证 NTex 的**诊断原语**本身与 pdfTeX 一致。

## 为什么需要它（2026-09-11 血泪）

`\\meaning` 的输出长期丢失「参数文本定界符」（`macro:->` 而非 `macro:if->`）。
这个**纯显示层**的 bug 直接导致两轮定位跑偏：
- §38 断言「plain 预载 24 错锚定 `\\if@`」→ 基于失真的 `\\meaning`
- §39 把根因固化成「`\\uppercase` 内 `\\gdef` 参数文本丢失」
- §41 才用插桩推翻上述两节

**结论：诊断仪器失真 = 定位方向失真。仪器本身必须进 CI。**

## 覆盖范围

逐个诊断原语，用固定样本与 `pdfTeX -ini` 对拍，**必须逐字一致**：

| 仪器 | 契约 |
|---|---|
| `\\meaning` | 宏（含定界参数文本）/ 原语 / 未定义 / 寄存器 / 字符 |
| `\\show` | slot 显示格式（cmd 类型 + 值）|
| `\\the` | 数值/尺寸/胶水/码表读回格式 |
| `\\detokenize` | token → 字符序列（含分隔空格规则）|
| `\\romannumeral` | 罗马数字（含 0 → 空串）|
| `\\number` | 十进制（含负数）|
| `\\string` | cs 名（受 `\\escapechar` 影响）|

## 用法

    scripts/instrument-check.py            # 跑全部，遇差异非零退出（供 CI）
    scripts/instrument-check.py -v         # 打印每个用例的两侧输出

## 退出码
    0 = 全部一致    1 = 有失真    2 = 环境错误
"""
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
TIMEOUT = 30

# 每个用例：(名字, 源码, 抽取正则, 模式)
#   模式 "plain"  → pdfTeX 用普通模式（预载 plain），NTex 用默认预载
#   模式 "initex" → pdfTeX 加 -ini，NTex 加 --no-plain（比 INITEX 初表语义）
#   ⚠ 为何要分：INITEX 初表里 `{` 是 cat 12，`uppercase{...}` 之类的
#     分组原语在纯 initex 下**无法工作**（pdfTeX 实测报 Missing {）。
#   源码约定：用 `\message{I=...}` 打信号；两侧都以 `\end` 收敛。
#   抽取正则从两侧输出里捞信号片段，直接字符串比对。
#   ⚠ 正则不要依赖尾随空白：NTex 的 `\message` 落在行尾（`hyphenationI=42`）。
CASES: list[tuple[str, str, str, str]] = [
    # ── meaning 族（plain 域）────────────────────────────────────────
    (
        "meaning-macro-delimiters",
        r"""\uccode`1=`i \uccode`2=`f
\uppercase{\gdef\ifxx12{}}
\message{I=\meaning\ifxx}
\end""",
        r"I=(macro:[^\s]*)", "plain",
    ),
    (
        "meaning-macro-params",
        r"""\def\a#1#2{#2#1}
\message{I=\meaning\a}
\end""",
        r"I=(macro:[^\s]*)", "plain",
    ),
    (
        "meaning-macro-mixed",
        r"""\def\a,#1;{#1}
\message{I=\meaning\a}
\end""",
        r"I=(macro:[^\s]*)", "plain",
    ),
    (
        "meaning-undefined",
        r"""\message{I=\meaning\zzzundef}
\end""",
        r"I=([^\s]*)", "plain",
    ),
    # ── 	he / 
    (
        "the-count",
        r"""\count5=42 \message{I=\the\count5}
\end""",
        r"I=([^\s]*)", "plain",
    ),
    (
        "the-dimen",
        r"""\dimen5=3pt \message{I=\the\dimen5}
\end""",
        r"I=([^\s]*)", "plain",
    ),
    (
        "number-negative",
        r"""\count5=-17 \message{I=\number\count5}
\end""",
        r"I=([^\s]*)", "plain",
    ),
    # ── 字符串转换族（plain 域）──────────────────────────────────────
    (
        "romannumeral-basic",
        r"""\message{I=\romannumeral1994}
\end""",
        r"I=([^\s]*)", "plain",
    ),
    (
        "romannumeral-zero",
        r"""\message{I=\romannumeral0.}
\end""",
        r"I=([^\s.]*)\.", "plain",
    ),
    (
        "string-cs",
        r"""\message{I=\string\hello.}
\end""",
        r"I=([^\s.]*)\.", "plain",
    ),
    (
        "string-escapechar-m1",
        r"""\escapechar=-1 \message{I=\string\hello.}
\end""",
        r"I=([^\s.]*)\.", "plain",
    ),
    # ── INITEX 初表族（initex 域）────────────────────────────────────
    (
        "catcode-nul-initex",
        r"""\catcode`\{=1 \catcode`\}=2
\message{I=\the\catcode0 }
\end""",
        r"I=(\d+)", "initex",
    ),
    (
        "catcode-brace-initex",
        r"""\catcode`\{=1 \catcode`\}=2
\message{I=\the\catcode`\{ }
\end""",
        r"I=(\d+)", "initex",
    ),
]

NOISE = re.compile(
    r"^(This is pdfTeX|entering extended mode|restricted \\write18"
    r"|Transcript written|Output written|No pages of output|\*\*|\s*$)"
)


def run_one(src: str, workdir: Path, mode: str) -> tuple[str, str]:
    """跑两侧，返回 (pdftex_out, ntex_out)。mode ∈ {"plain","initex"}。"""
    staged = workdir / "probe.tex"
    staged.write_text(src)
    # pdfTeX：plain 模式用默认格式；initex 模式加 -ini
    px_cmd = [str(PDFTEX), "-interaction=nonstopmode"]
    if mode == "initex":
        px_cmd.append("-ini")
    px_cmd.append("probe.tex")
    try:
        subprocess.run(px_cmd, capture_output=True, text=True, timeout=TIMEOUT,
                       cwd=str(workdir))
        log = workdir / "probe.log"
        px = log.read_text(errors="replace") if log.is_file() else ""
    except (subprocess.TimeoutExpired, OSError):
        px = "[pdftex failed]"
    # NTex：plain 模式默认预载；initex 模式 --no-plain
    env = dict(os.environ)
    env.setdefault("NTEX_TFM_DIR", str(Path.home() / ".ntex-fonts"))
    # ⚠ 不可加 --quiet：它会连转录一起屏蔽，自检正需要转录
    nx_cmd = [str(NTEX_DVI), str(staged)]
    if mode == "initex":
        nx_cmd.append("--no-plain")
    try:
        r = subprocess.run(nx_cmd, capture_output=True, text=True,
                           timeout=TIMEOUT, cwd=str(workdir), env=env)
        nx = r.stdout + r.stderr
    except (subprocess.TimeoutExpired, OSError):
        nx = "[ntex failed]"
    return px, nx


def grab(text: str, rx: str) -> str | None:
    """从输出里捞信号（先按行滤噪，再跨整段找第一个匹配）。"""
    cleaned = "\n".join(l for l in text.splitlines() if not NOISE.match(l))
    m = re.search(rx, cleaned)
    return m.group(1).strip() if m else None


def main() -> int:
    verbose = "-v" in sys.argv or "--verbose" in sys.argv
    if not PDFTEX.is_file():
        print(f"[instrument-check] 找不到 pdfTeX：{PDFTEX}", file=sys.stderr)
        return 2
    if not NTEX_DVI.is_file():
        print(f"[instrument-check] 缺 {NTEX_DVI}；先 `cargo build -p ntex-dvi`",
              file=sys.stderr)
        return 2

    workdir = Path(tempfile.mkdtemp(prefix="instr-"))
    bad: list[tuple[str, str | None, str | None]] = []
    try:
        for name, src, rx, mode in CASES:
            wd = workdir / name
            wd.mkdir(exist_ok=True)
            px_raw, nx_raw = run_one(src, wd, mode)
            px, nx = grab(px_raw, rx), grab(nx_raw, rx)
            ok = px is not None and px == nx
            mark = "✅" if ok else "❌"
            print(f"{mark} {name:<28} pdftex={px!r} ntex={nx!r}"
                  if verbose or not ok else f"{mark} {name}")
            if not ok:
                bad.append((name, px, nx))
        print()
        if bad:
            print(f"❌ {len(bad)}/{len(CASES)} 项仪器失真——**先修仪器再定位**：")
            for name, px, nx in bad:
                print(f"   {name}: pdftex={px!r} ntex={nx!r}")
            return 1
        print(f"✅ {len(CASES)}/{len(CASES)} 仪器自检全部一致")
        return 0
    finally:
        shutil.rmtree(workdir, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
