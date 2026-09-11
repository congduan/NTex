#!/usr/bin/env python3
"""l3kernel `.tlg` 期望输出对比器 —— 把「跑通」升级为「跑对」。

## 为什么需要它（2026-09-11 实测教训）

`scripts/lvt-run.py` 只判 **RAN/CRASH**（跑到 `END-TEST-LOG` 与否）。
但 NTex 是「错误恢复式引擎」：**跑到末尾 ≠ 输出正确**。

实测 `m3basics001` 判 RAN，可转录里：

```
TEST 1: cs\\~if\\~exist\\~use          ← `_` 渲染成 `\~`（catcode 态错）
! Undefined control sequence.
\\cs_if_exist_use:N                   ← 测试体函数未定义
TRUE
```

即 **RAN 严重高估真实进度**（与 docs/tooling-trust.md §0「跑完了≠对了」同源）。

本工具把 NTex 转录裁剪到「测试体输出段」后与 `.tlg` 逐行比对，给出：

| 判定 | 含义 |
|---|---|
| `PASS` | 测试体输出与 `.tlg` 完全一致（去 harness 噪声后）|
| `DIFF` | 有差异但都跑完 —— **真实语义缺口**（下一步靶子）|
| `NOISE` | 差异只集中在少数重复行（如 `_` 渲染）—— 归一化后一致 |
| `NO-RUN` | 没跑到（交给 lvt-run 判）|

## 用法

    python3 scripts/lvt-tlg-diff.py m3basics001           # 单例详报（含首个差异上下文）
    python3 scripts/lvt-tlg-diff.py --all --jobs 2        # 全量统计 PASS/DIFF
    python3 scripts/lvt-tlg-diff.py --all --top 20        # 按差异行数排序的靶子榜
"""

from __future__ import annotations

import argparse
import difflib
import os
import re
import shutil
import subprocess
import sys
import tempfile
from collections import Counter
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
NTEX = ROOT / "target" / "debug" / "ntex-dvi"
CACHE = Path("/tmp/l3kernel-tests")
TESTDIR = CACHE / "testfiles"
TFM_DIR = os.path.expanduser("~/.ntex-fonts")

# harness 自身输出（.tlg 里也含）：从 START-TEST-LOG 之后才算测试体
BODY_START = "START-TEST-LOG"


def _norm(line: str) -> str:
    """归一化一行以便比对：去尾随空白、统一空白序列。"""
    return re.sub(r"\s+", " ", line.strip())


def extract_body(text: str) -> list[str]:
    """从 NTex 转录里裁出「测试体输出段」。

    起点：`START-TEST-LOG` 之后（harness 开始跑用例）。
    终点：`END-TEST-LOG` 之前。
    再剔除 NTex 特有的噪声行（[debug]、plain 预载 dump、l.N 上下文行等）。
    """
    lines = text.replace("\r", "\n").replace("\x00", "").split("\n")
    # 定位 START/END
    start = end = None
    for i, l in enumerate(lines):
        if BODY_START in l and start is None:
            start = i + 1
        if "END-TEST-LOG" in l and start is not None:
            end = i
            break
    if start is None:
        return []
    body = lines[start:end] if end is not None else lines[start:]

    # 跳过 harness 头部噪声：`START-TEST-LOG` 后是 l3build 生成的说明头 +
    # `\AUTHOR`/`\TIMO` 等 harness 宏的报错——真正的测试体从第一条
    # `TEST n:`（或 `TESTEXP`）标记开始。.tlg 同样从那里开始。
    for i, l in enumerate(body):
        if re.match(r"\s*TEST", l.strip()):
            body = body[i:]
            break

    out = []
    for l in body:
        s = l.rstrip()
        if not s.strip():
            continue
        # 噪声：NTex 诊断/错误恢复上下文
        if s.lstrip().startswith(("[debug]", "[watchdog]", "[biglist]", "[trace")):
            continue
        if re.match(r"^l\.\d+", s.strip()):
            continue
        if s.strip().startswith(("<to be read again>", "<inserted text>", "<recently read>")):
            continue
        if s.strip() in ("", "\\par"):
            continue
        # 分隔线（`====`/`----`）：.tlg 侧已剔，NTex 侧同剔
        if re.fullmatch(r"[=\-]{4,}", s.strip()):
            continue
        # harness 头（.tlg 顶部同款，此处也剔）
        if s.startswith(("This is a generated file", "Don't change this file", "Author:")):
            continue
        # TeX 错误行本身（`! ...`）与其后紧跟的 cs 名行：NTex 报错文本面与
        # .tlg 不同轨（.tlg 记录的是**期望**输出，不含引擎错误）
        if s.lstrip().startswith("!"):
            continue
        # TeX 错误提示语（多行 help）：以固定短语开头
        if re.match(
            r"^(I was expecting|A number should|If you can't|look up|"
            r"I suspect|to read past|you'd better|I'll try to recover|"
            r"My plan is|I've run across|this error|For example|"
            r"so I'm ignoring|The macro here)",
            s.strip(),
        ):
            continue
        out.append(_norm(s))
    return out


def read_tlg(path: Path) -> list[str]:
    """读 `.tlg` 期望输出，剔除头部说明与分隔线。"""
    lines = path.read_text(errors="replace").split("\n")
    out = []
    for l in lines:
        s = l.rstrip()
        if not s.strip():
            continue
        # 头部说明
        if s.startswith(("This is a generated file", "Don't change this file", "Author:")):
            continue
        # 分隔线（=== 或 ---）
        if re.fullmatch(r"[=\-]{4,}", s.strip()):
            continue
        out.append(_norm(s))
    return out


def run_case(case: str, timeout: int = 30) -> dict:
    wd = Path(tempfile.mkdtemp(prefix="lvt-diff-"))
    try:
        for f in ("lvt-shim.tex", "regression-test.tex", "regression-test.cfg",
                  f"{case}.lvt", f"{case}.tlg"):
            src = TESTDIR / f
            if src.exists():
                shutil.copy(src, wd / f)
        (wd / "run.tex").write_text(
            f"\\def\\LVTFILE{{{case}.lvt}}\n\\input {wd}/lvt-shim\n"
        )
        env = {**os.environ, "NTEX_TFM_DIR": TFM_DIR}
        try:
            r = subprocess.run(
                [str(NTEX), "--input-path", str(wd), str(wd / "run.tex")],
                cwd=wd, capture_output=True, timeout=timeout, env=env,
            )
        except subprocess.TimeoutExpired:
            return {"case": case, "verdict": "TIMEOUT", "detail": "", "ndiff": 0}
        text = (r.stdout + r.stderr).decode(errors="replace")
        if "END-TEST-LOG" not in text:
            return {"case": case, "verdict": "NO-RUN", "detail": "", "ndiff": 0}

        got = extract_body(text)
        tlg_path = TESTDIR / f"{case}.tlg"
        if not tlg_path.exists():
            return {"case": case, "verdict": "NO-TLG", "detail": "", "ndiff": 0}
        want = read_tlg(tlg_path)

        if got == want:
            return {"case": case, "verdict": "PASS", "detail": "", "ndiff": 0}

        # 差异统计
        sm = difflib.SequenceMatcher(None, want, got)
        diffs = [op for op in sm.get_opcodes() if op[0] != "equal"]
        ndiff = sum(max(i2 - i1, j2 - j1) for _, i1, i2, j1, j2 in diffs)

        # NOISE 判定：差异全部是「同一组期望行 → 同一组实得行」的重复替换
        want_set = Counter(want)
        got_set = Counter(got)
        only_want = want_set - got_set
        only_got = got_set - want_set
        # 若唯一新增/缺失都集中在 ≤3 种行 → 视为系统性渲染噪声
        if len(only_want) <= 3 and len(only_got) <= 3:
            verdict = "NOISE"
        else:
            verdict = "DIFF"

        # 首个差异的上下文（单例报告用）
        detail = ""
        if diffs:
            _, i1, i2, j1, j2 = diffs[0]
            w = " / ".join(want[i1:i1 + 3])
            g = " / ".join(got[j1:j1 + 3])
            detail = f"期望: {w[:110]} | 实得: {g[:110]}"
        return {"case": case, "verdict": verdict, "detail": detail, "ndiff": ndiff}
    finally:
        shutil.rmtree(wd, ignore_errors=True)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("case", nargs="?")
    ap.add_argument("--all", action="store_true")
    ap.add_argument("--jobs", type=int, default=1)
    ap.add_argument("--timeout", type=int, default=30)
    ap.add_argument("--top", type=int, default=20)
    args = ap.parse_args()

    if not TESTDIR.is_dir():
        print("未找到测试树；先跑 `python3 scripts/lvt-run.py --fetch`", file=sys.stderr)
        return 1

    if args.case:
        r = run_case(args.case, args.timeout)
        print(f"[{r['case']}] {r['verdict']}  差异行数={r['ndiff']}")
        if r["detail"]:
            print(f"  首个差异：{r['detail']}")
        return 0

    cases = sorted(p.stem for p in TESTDIR.glob("*.lvt"))
    if not args.all:
        print("需指定用例名或 --all", file=sys.stderr)
        return 1

    with ThreadPoolExecutor(max_workers=args.jobs) as ex:
        results = list(ex.map(lambda c: run_case(c, args.timeout), cases))

    cnt = Counter(r["verdict"] for r in results)
    total = len(results)
    print(f"[lvt-tlg-diff] 全量 {total} 例\n")
    for v, n in cnt.most_common():
        print(f"  {v:<10} {n:>4}  {n / total * 100:.1f}%")

    diffs = [r for r in results if r["verdict"] in ("DIFF", "NOISE")]
    diffs.sort(key=lambda r: -r["ndiff"])
    print(f"\n=== 差异靶子榜（按差异行数排，前 {args.top}）===")
    for r in diffs[: args.top]:
        print(f"  {r['ndiff']:>4} 行  {r['case']:<22} {r['verdict']}")
        if r["detail"]:
            print(f"            {r['detail'][:130]}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
