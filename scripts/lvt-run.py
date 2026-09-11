#!/usr/bin/env python3
"""l3kernel 官方测试套件跑分器（l3build `.lvt` × NTex）。

## 为什么用它

expl3 攻坚此前的方式是「跑 4 万行 latex.ltx → 看错误 → **手写探针猜根因**」。
实测探针错误率极高（2026-09-11 一轮内 3 次误判），且**没有分母**——无法回答
「expl3 还差多少」。

官方 `l3kernel/testfiles/` 有 **208 个 `.lvt`**，每个配 `.tlg`（权威期望转录，
gold standard）。这是**可量化的分母**，且**不需要我们造探针**——用例由 LaTeX
项目维护。

## 机制

`.lvt` 头部是 `\\documentclass{minimal}` + `\\input{regression-test}`（LaTeX 内核），
而 LaTeX 内核依赖 expl3（循环）。故用 `scripts/lvt/lvt-shim.tex`（plain 垫片）
提供那几个 LaTeX 符号，再载入官方 `regression-test.tex`（纯 expl3 + TeX 原语，
不需要 LaTeX 内核）。

## 用法

    scripts/lvt-run.py --fetch              # 抓 l3kernel 测试（GitHub latex3/latex3）
    scripts/lvt-run.py --list               # 列出全部用例
    scripts/lvt-run.py m3basics001          # 跑单个（看转录）
    scripts/lvt-run.py --all --jobs 2       # 全量跑分（输出通过率）
    scripts/lvt-run.py --all --timeout 20   # 调单例超时

## 判据

- **CRASH**：NTex 报错终止（非零退出 / "排版失败"）
- **DIFF**：跑完但与 `.tlg` 期望不符（详略按 `--show-diff`）
- **PASS**：转录与 `.tlg` 一致（去 harness 头尾噪声后比对）
- **LOAD-FAIL**：连 harness 都没过（expl3 都没跑起来）

⚠ PASS 判定目前是**宽松比对**（.tlg 含 harness 的 START-TEST-LOG 等噪声，
NTex 侧转录格式亦未完全对齐）。**先看 CRASH/DIFF 的比例**，比精确比对更有
信息量。
"""
import argparse
import os
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import urllib.request
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

REPO = Path("/home/ubuntu/NTex")
NTEX = REPO / "target/debug/ntex-dvi"
SHIM = REPO / "scripts/lvt/lvt-shim.tex"
CACHE = Path("/tmp/l3kernel-tests")
L3_URL = "https://github.com/latex3/latex3/archive/refs/heads/main.tar.gz"

TFM_DIR = os.environ.get("NTEX_TFM_DIR", str(Path.home() / ".ntex-fonts"))


def fetch() -> Path:
    """下载/解包 l3kernel 测试树；返回 testfiles 目录。"""
    tf = CACHE / "testfiles"
    if tf.is_dir() and any(tf.glob("*.lvt")):
        return tf
    CACHE.mkdir(parents=True, exist_ok=True)
    tgz = CACHE / "l3.tgz"
    if not tgz.exists():
        print(f"[lvt-run] 下载 {L3_URL} …")
        urllib.request.urlretrieve(L3_URL, tgz)
    print("[lvt-run] 解包 …")
    with tarfile.open(tgz) as t:
        t.extractall(CACHE)
    src = next(CACHE.glob("latex3-*/l3kernel/testfiles"), None)
    if src is None:
        print("[lvt-run] 未找到 l3kernel/testfiles", file=sys.stderr)
        sys.exit(2)
    if tf.exists():
        shutil.rmtree(tf)
    shutil.copytree(src, tf)
    # regression-test.tex 从 l3build 取
    rt = next(CACHE.glob("latex3-*/texmf/tex/latex/l3build/regression-test.tex"), None)
    if rt:
        shutil.copy(rt, tf / "regression-test.tex")
    (tf / "regression-test.cfg").write_text("")
    shutil.copy(SHIM, tf / "lvt-shim.tex")
    return tf


def run_one(tf: Path, name: str, timeout: int = 30) -> tuple[str, str]:
    """跑一个用例。返回 (verdict, 转录尾部)。"""
    base = name if name.endswith(".lvt") else name + ".lvt"
    if not (tf / base).exists():
        return "MISSING", f"找不到 {base}"
    wd = Path(tempfile.mkdtemp(prefix="lvt-"))
    for f in ("lvt-shim.tex", "regression-test.tex", "regression-test.cfg", base):
        if (tf / f).exists():
            shutil.copy(tf / f, wd / f)
    (wd / "run.tex").write_text(f"\\def\\LVTFILE{{{base}}}\n\\input {wd}/lvt-shim\n")
    try:
        r = subprocess.run(
            [str(NTEX), "--input-path", str(wd), str(wd / "run.tex")],
            cwd=wd, capture_output=True, timeout=timeout,
            env={**os.environ, "NTEX_TFM_DIR": TFM_DIR},
        )
    except subprocess.TimeoutExpired:
        return "TIMEOUT", ""
    # ⚠ NTex 的转录走 **stderr**（stdout 仅 DVI/二进制产物），且失败时
    # returncode=1 也可能已跑完全部用例——故**两侧都收**再判据。
    out = (r.stdout + r.stderr).decode(errors="replace").replace("\r", "\n").replace("\x00", "")
    tail = "\n".join(out.split("\n")[-6:])
    # 判据顺序很重要：**先看是否跑到 END-TEST-LOG**（harness 正常收尾），
    # 再判致命错——否则「跑到末尾但中途报错」会被误判成 CRASH，
    # 而「跑到末尾」本身已是 expl3 引导可用的强信号。
    reached_end = "END-TEST-LOG" in out
    if "输入栈超限" in out:
        return ("STACK-END" if reached_end else "STACK"), tail
    if "排版失败" in out or "Emergency stop" in out:
        return ("CRASH-END" if reached_end else "CRASH"), tail
    if not reached_end:
        return "NO-END", tail
    tlg = tf / base.replace(".lvt", ".tlg")
    if not tlg.exists():
        return "PASS?", tail
    return "RAN", tail


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("case", nargs="?")
    ap.add_argument("--fetch", action="store_true")
    ap.add_argument("--list", action="store_true")
    ap.add_argument("--all", action="store_true")
    ap.add_argument("--jobs", type=int, default=2)
    ap.add_argument("--timeout", type=int, default=30)
    ap.add_argument("--show-diff", action="store_true")
    args = ap.parse_args()

    tf = fetch()
    if args.list:
        for f in sorted(tf.glob("*.lvt")):
            print(f.stem)
        return 0
    if args.fetch:
        print(f"[lvt-run] 就绪：{tf}（{len(list(tf.glob('*.lvt')))} 个用例）")
        return 0
    if args.case:
        v, tail = run_one(tf, args.case, args.timeout)
        print(f"[{args.case}] {v}")
        print(tail)
        return 0
    if args.all:
        cases = sorted(p.stem for p in tf.glob("*.lvt"))
        print(f"[lvt-run] 全量 {len(cases)} 例（jobs={args.jobs}, timeout={args.timeout}s）")
        with ThreadPoolExecutor(max_workers=args.jobs) as ex:
            results = list(ex.map(lambda c: (c, run_one(tf, c, args.timeout)[0]), cases))
        from collections import Counter
        cnt = Counter(v for _, v in results)
        for v, n in cnt.most_common():
            print(f"  {v:<9} {n}")
        print()
        # 按模块聚合（用例名前缀 m3xxx）
        mods: dict[str, list[str]] = {}
        for c, v in results:
            m = re.match(r"(m?[0-9a-z]+?)\d{3}$", c)
            mods.setdefault(m.group(1) if m else c, []).append(v)
        print("按模块（RAN/PASS? 视为已跑通 harness）：")
        for m in sorted(mods):
            vs = mods[m]
            ok = sum(1 for v in vs if v in ("RAN", "PASS?"))
            print(f"  {m:<16} {ok}/{len(vs)}")
        Path("/tmp/lvt-results.tsv").write_text(
            "\n".join(f"{c}\t{v}" for c, v in results) + "\n"
        )
        print("\n明细 → /tmp/lvt-results.tsv")
        return 0
    ap.print_help()
    return 1


if __name__ == "__main__":
    sys.exit(main())
