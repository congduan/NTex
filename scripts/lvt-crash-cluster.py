#!/usr/bin/env python3
"""CRASH 首错聚类：把 lvt-run.py 的失败用例按「首个 TeX 错误」归类。

## 为什么需要它

`scripts/lvt-run.py --all` 只给「RAN / CRASH」二元判定，**不告诉你是哪一种
失败**。76 例 CRASH 可能只是 3-5 个共同根因 —— 聚类后一次看清归因，
避免「一例一查」的低效（2026-09-11 教训：先在局部打转数回合）。

## 用法

    python3 scripts/lvt-crash-cluster.py            # 全量跑 + 聚类
    python3 scripts/lvt-crash-cluster.py --top 30   # 显示更多簇
    python3 scripts/lvt-crash-cluster.py --case m3prop001   # 单例详情

## 判定依据

每例跑一次，从转录里抽：
  - 首个 `! ` 错误行（TeX 式错误）
  - 该错误后的 `l.N <code>` 上下文行（定位）
  - 若无 `! `：判为「无 END-TEST-LOG」（harness 未跑完）或崩溃

输出：按首错签名聚合的簇 + 每簇用例清单 + 代表性命中位置。
"""

from __future__ import annotations

import argparse
import os
import re
import shutil
import subprocess
import sys
import tempfile
from collections import Counter, defaultdict
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
NTEX = ROOT / "target" / "debug" / "ntex-dvi"
TESTDIR = Path("/tmp/l3kernel-tests/testfiles")


def norm_err(line: str) -> str:
    """归一化错误行：去掉行号/具体标识符，得到可聚类的签名。"""
    s = line.strip()
    # 去掉 `l.NN`、具体 cs 名、数字
    s = re.sub(r"\bl\.\d+\b", "l.N", s)
    s = re.sub(r"\\[A-Za-z@:_]+", r"\\CS", s)
    s = re.sub(r"\d+", "N", s)
    return s[:120]


def run_case(case: str, timeout: int = 20) -> dict:
    wd = Path(tempfile.mkdtemp(prefix="lvt-"))
    try:
        for f in ("lvt-shim.tex", "regression-test.tex", "regression-test.cfg"):
            src = TESTDIR / f
            if src.exists():
                shutil.copy(src, wd / f)
        shutil.copy(TESTDIR / f"{case}.lvt", wd / f"{case}.lvt")
        (wd / "run.tex").write_text(
            f"\\def\\LVTFILE{{{case}.lvt}}\n\\input {wd}/lvt-shim\n"
        )
        env = {**os.environ, "NTEX_TFM_DIR": os.path.expanduser("~/.ntex-fonts")}
        try:
            r = subprocess.run(
                [str(NTEX), "--input-path", str(wd), str(wd / "run.tex")],
                cwd=wd, capture_output=True, timeout=timeout, env=env,
            )
            out = (r.stdout + r.stderr).decode(errors="replace")
        except subprocess.TimeoutExpired:
            return {"case": case, "verdict": "TIMEOUT", "first_err": "TIMEOUT"}
        nl = out.replace("\r", "\n").replace("\x00", "")
        # ⚠ 与 lvt-run.py 判据**完全一致**（顺序很重要）：
        #   先看是否跑到 END-TEST-LOG（harness 正常收尾），再判致命错。
        #   否则「跑到末尾但中途报错」会被误判成 CRASH —— 而「跑到末尾」
        #   本身已是 expl3 引导可用的强信号（2026-09-11 修）。
        reached_end = "END-TEST-LOG" in nl
        if "输入栈超限" in nl:
            return {"case": case, "verdict": "STACK-END" if reached_end else "STACK",
                    "first_err": "输入栈超限"}
        fatal = ("排版失败" in nl) or ("Emergency stop" in nl)
        if fatal:
            # ⚠ 必须取「排版失败」之后紧跟的**引擎真错误**，而不是转录里第一个
            # `! ` ——harness 的 .tlg 期望输出本身就含 `! ` 行（如 `l.3 找不到文件`
            # 是 l3build 的期望文本），会误导聚类（实测：46 例被误聚成
            # 「找不到文件」，真错误实为「双重下标」等）。
            mm = re.search(r"排版失败：(.*)", nl)
            if mm:
                err = norm_err("! " + mm.group(1))
            else:
                m = re.search(r"^! (.*)$", nl, re.M)
                err = norm_err("! " + m.group(1)) if m else "! Emergency stop."
            head = nl[: mm.start()] if mm else nl[: m.end()] if m else ""
            ctx = None
            for cm in re.finditer(r"^l\.(\d+)\s*(.*)$", head, re.M):
                ctx = cm
            tail = head
            where = f"l.{ctx.group(1)} {ctx.group(2)[:40]}" if ctx else ""
            return {"case": case, "verdict": "CRASH-END" if reached_end else "CRASH",
                    "first_err": err, "where": where}
        if not reached_end:
            return {"case": case, "verdict": "NO-END", "first_err": "未跑到 END-TEST-LOG"}
        return {"case": case, "verdict": "RAN", "first_err": ""}
    finally:
        shutil.rmtree(wd, ignore_errors=True)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--case", help="单例详情")
    ap.add_argument("--top", type=int, default=20)
    ap.add_argument("--jobs", type=int, default=2)
    ap.add_argument("--timeout", type=int, default=20)
    args = ap.parse_args()

    cases = sorted(p.stem for p in TESTDIR.glob("*.lvt")) if TESTDIR.exists() else []
    if not cases:
        print("未找到测试树；先跑 `python3 scripts/lvt-run.py --fetch`", file=sys.stderr)
        return 1

    if args.case:
        r = run_case(args.case, args.timeout)
        for k, v in r.items():
            print(f"  {k}: {v}")
        return 0

    with ThreadPoolExecutor(max_workers=args.jobs) as ex:
        results = list(ex.map(lambda c: run_case(c, args.timeout), cases))

    crash = [r for r in results if r["verdict"].startswith("CRASH")]
    print(f"总计 {len(results)} 例；CRASH {len(crash)} 例\n")

    clusters: dict[str, list[dict]] = defaultdict(list)
    for r in crash:
        clusters[r["first_err"]].append(r)

    print(f"=== CRASH 首错聚类（{len(clusters)} 簇）===")
    for sig, rs in sorted(clusters.items(), key=lambda kv: -len(kv[1]))[: args.top]:
        print(f"\n  [{len(rs):>2} 例] {sig}")
        names = ", ".join(r["case"] for r in rs[:8])
        print(f"          {names}{' …' if len(rs) > 8 else ''}")
        wh = [r.get("where", "") for r in rs if r.get("where")]
        if wh:
            print(f"          代表位置: {Counter(wh).most_common(1)[0][0]}")

    other = Counter(r["verdict"] for r in results if not r["verdict"].startswith("CRASH"))
    print(f"\n=== 其他判定 ===\n  {dict(other)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
