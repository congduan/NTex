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

REPO = Path(__file__).resolve().parent.parent  # 不可硬编码绝对路径（曾在 /home/ubuntu 机器上失效）
from ntex_bin import ensure_fresh

NTEX = ensure_fresh()
SHIM = REPO / "scripts/lvt/lvt-shim.tex"
CACHE = Path("/tmp/l3kernel-tests")
L3_URL = "https://github.com/latex3/latex3/archive/refs/heads/main.tar.gz"

TFM_DIR = os.environ.get("NTEX_TFM_DIR", str(Path.home() / ".ntex-fonts"))

# expl3 本体生成物（载入器 + 代码）。**没有它们，187 例跑的不是 expl3。**
#
# ⚠ 2026-09-11 实测教训：此前 harness 从不载入 expl3，用例里的
# `\cs_if_exist_use:N` 等**全部 undefined**（实测 77 个/例），
# 却因错误恢复跑到 `END-TEST-LOG` 被判 RAN —— 「RAN 180/187」是假分数。
# 载入 expl3 后同一用例 undefined 降到 6（真实差异才显现）。
#
# 必须用 **`expl3.ltx`**（不是 `expl3-code.tex` 直接 input）：后者有 loader 检查
# （`\expandafter\ifx\csname ExplLoaderFileDate\endcsname\relax` →
# `\PackageError{expl3}{No expl3 loader detected}`），`expl3.ltx` 首行
# `\let\ExplLoaderFileDate\ExplFileDate` 才提供该标志。
EXPL3_FILES = ("exgeneric.tex", "expl3-code.tex")


def ensure_expl3(tf: Path) -> bool:
    """确保 `expl3.ltx` + `expl3-code.tex` 在 testfiles 目录里。

    来源优先级：
      1. 已缓存（`CACHE/expl3-built/`，最快）
      2. `EXPL3_SRC` 环境变量指向的目录（供手工生成/调试）
      2.5. 仓库内 `fixtures/l3kernel/`（2026-09-13 入库，LPPL；md5
           7a1cc7249b9eeccb4029956317d7a295——/tmp 缓存随整机重启丢失，
           fixtures 让跑分/探针开箱即用）
      3. 用 latex3 仓库的 `l3kernel.ins` + 一个 TeX 引擎 docstrip 生成

    返回 True 表示两文件就位。生成失败不致命——退回「无 expl3」模式，
    但会在判据里显式标注（见 [`run_one`] 的 verdict 前缀）。
    """
    if all((tf / f).exists() for f in EXPL3_FILES):
        return True
    built = CACHE / "expl3-built"
    if all((built / f).exists() for f in EXPL3_FILES):
        for f in EXPL3_FILES:
            shutil.copy(built / f, tf / f)
        return True
    src_env = os.environ.get("EXPL3_SRC")
    if src_env and all((Path(src_env) / f).exists() for f in EXPL3_FILES):
        for f in EXPL3_FILES:
            shutil.copy(Path(src_env) / f, tf / f)
        return True
    fixtures = REPO / "fixtures" / "l3kernel"
    if all((fixtures / f).exists() for f in EXPL3_FILES):
        for f in EXPL3_FILES:
            shutil.copy(fixtures / f, tf / f)
        return True
    # TeX Live 本地分发（kpsewhich）：loader 与 expl3-code.tex 同版本，
    # `\ifx\ExplLoaderFileDate\ExplFileDate` 校验必过，比 docstrip 生成快得多。
    try:
        r = subprocess.run(["kpsewhich", *EXPL3_FILES], capture_output=True)
        paths = [Path(p) for p in r.stdout.decode().split()]
    except FileNotFoundError:
        paths = []
    if len(paths) == len(EXPL3_FILES):
        for f, p in zip(EXPL3_FILES, paths):
            shutil.copy(p, tf / f)
        return True
    # 用 l3kernel.ins 生成（需 latex3 源码树已解包 + 可用 TeX 引擎）
    ins = next(CACHE.glob("latex3-*/l3kernel/l3kernel.ins"), None)
    if ins is None:
        return False
    engine = shutil.which("pdftex") or shutil.which("tex")
    if engine is None:
        return False
    built.mkdir(parents=True, exist_ok=True)
    try:
        r = subprocess.run(
            [engine, "-interaction=nonstopmode", "-output-directory", str(built), "l3kernel.ins"],
            cwd=ins.parent, capture_output=True, timeout=300,
        )
    except subprocess.TimeoutExpired:
        return False
    if not all((built / f).exists() for f in EXPL3_FILES):
        # docstrip 可能输出到 cwd
        for f in EXPL3_FILES:
            if (ins.parent / f).exists():
                shutil.copy(ins.parent / f, built / f)
    if not all((built / f).exists() for f in EXPL3_FILES):
        return False
    for f in EXPL3_FILES:
        shutil.copy(built / f, tf / f)
    return True


def fetch() -> Path:
    """下载/解包 l3kernel 测试树；返回 testfiles 目录。"""
    tf = CACHE / "testfiles"
    if tf.is_dir() and any(tf.glob("*.lvt")):
        # ⚠ 已缓存也要**同步 shim**：shim 是本仓库的代码（`scripts/lvt/lvt-shim.tex`），
        # 会随开发更新。不同步会导致「跑了旧 shim 得出错误结论」——实测踩过：
        # 旧 shim 仍自带 `\input regression-test.tex`（双重载入致爆栈），
        # 而仓库里已修好，导致 187 例全被误判为 CRASH（2026-09-11）。
        if SHIM.exists():
            shutil.copy(SHIM, tf / "lvt-shim.tex")
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
    files = ["lvt-shim.tex", "regression-test.tex", "regression-test.cfg", base]
    # ⚠ expl3 本体必须进临时目录：否则用例里 `\cs_if_exist_use:N` 等全 undefined，
    # 却因错误恢复跑到 END-TEST-LOG → 假 RAN（2026-09-11 实测 77 undefined/例）。
    files += list(EXPL3_FILES)
    for f in files:
        if (tf / f).exists():
            shutil.copy(tf / f, wd / f)
    # 驱动文件负责终止作业（shim 不再自带 \end —— 嵌套 input 上下文里执行 \end
    # 会触发输入栈无限增长，见 docs/latex-feasibility.md §A1.undevicies）
    # ⚠ `\lvtuseexplthree`：显式告知 shim 是否载入 expl3。不用
    # `\IfFileExists`/`\openin` 探测——两者在 NTex 上均不可靠（实测）。
    use_expl3 = all((wd / f).exists() for f in EXPL3_FILES)
    (wd / "run.tex").write_text(
        f"\\chardef\\lvtuseexplthree={1 if use_expl3 else 0}\n"
        f"\\def\\LVTFILE{{{base}}}\n\\input {wd}/lvt-shim\n"
    )
    try:
        r = subprocess.run(
            [str(NTEX), "--input-path", str(wd), str(wd / "run.tex")],
            cwd=wd, capture_output=True, timeout=timeout,
            env={**os.environ, "NTEX_TFM_DIR": TFM_DIR},
        )
    except subprocess.TimeoutExpired:
        return "TIMEOUT", ""
    finally:
        # ⚠ 必须清理：每例一个临时目录，187 例 × 每轮全量跑 = 目录/磁盘持续泄漏
        # （实测累积 1142 个 `/tmp/lvt-*`，磁盘 87%，并**污染后续判定**——
        #  磁盘压力下 NTex 打开输入文件失败，被误报成 CRASH）。
        shutil.rmtree(wd, ignore_errors=True)
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
    # ⚠ expl3 本体就位是**真实分数**的前提。缺失时用例跑的不是 expl3，
    # 分数无意义（2026-09-11 实测：77 个 expl3 函数 undefined 仍判 RAN）。
    have_expl3 = ensure_expl3(tf)
    if not have_expl3:
        print(
            "[lvt-run] ⚠⚠ 未找到 expl3.ltx/expl3-code.tex —— 用例将**不载入 expl3**，\n"
            "          分数是**假分数**（用例里的 expl3 函数全部 undefined）。\n"
            "          修法：`EXPL3_SRC=/path/to/dir python3 scripts/lvt-run.py …`\n"
            "          （该目录需含 expl3.ltx + expl3-code.tex），\n"
            "          或让 latex3 源码树可被 l3kernel.ins 生成。",
            file=sys.stderr,
        )
    else:
        print("[lvt-run] expl3 本体已就位（expl3.ltx + expl3-code.tex）")
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
