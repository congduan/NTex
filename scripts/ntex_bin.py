"""NTex 诊断二进制防漂移门禁（基础设施⑦，2026-09-12 设立）。

背景（tooling-trust 事故七、以及更早的三次误归因）：「改完源码忘 build，
跑出来的是旧二进制」——上轮 fh9 战役的「静默截断」定性因此被矩阵证伪。

用法（所有诊断 runner 在跑引擎前调用）：

    from ntex_bin import ensure_fresh
    ntex = ensure_fresh()   # 返回 target/debug/ntex-dvi 路径；stale 即自动重建

规则：
  - 以 crate 源码树 + workspace Cargo.toml/lock 的最新 mtime 对比二进制 mtime；
  - 落后 ⇒ 自动 `cargo build -p ntex-dvi`（重建失败 ⇒ SystemExit，硬失败）；
  - `NTex_SKIP_REBUILD=1` 逃生口（只在明确知道二进制新鲜时用，比如刚 build 完
    又不想让脚本再扫一遍源码树）；
  - cargo 不可用/二进制不存在且无源码 ⇒ 报错退出（绝不静默用旧/缺二进制）。

VM 2 核环境注意：本模块用串行 cargo（遵守环境纪律，不 -j 抬并发）。
"""
import os
import subprocess
import sys
import time
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
NTEX_DVI = REPO / "target/debug/ntex-dvi"
# 参与新鲜度判定的源码集：被测 crate 全树 + workspace 级配置
WATCH_ROOTS = [REPO / "crates/ntex-core", REPO / "crates/ntex-dvi"]
WATCH_FILES = [REPO / "Cargo.toml", REPO / "Cargo.lock"]


def _latest_src_mtime() -> float:
    latest = 0.0
    for root in WATCH_ROOTS:
        if root.exists():
            latest = max(latest, max((p.stat().st_mtime for p in root.rglob("*.rs")
                                      if p.is_file()), default=0.0))
    latest = max(latest, max((p.stat().st_mtime for p in WATCH_FILES
                              if p.exists()), default=0.0))
    return latest


def ensure_fresh(verbose: bool = True) -> Path:
    """确保 target/debug/ntex-dvi 不落后于源码；返回其路径，失败硬退。"""
    if os.environ.get("NTex_SKIP_REBUILD") == "1":
        if not NTEX_DVI.exists():
            sys.exit("NTex_SKIP_REBUILD=1 但二进制不存在，拒绝继续")
        return NTEX_DVI
    if not NTEX_DVI.exists():
        sys.exit(f"二进制不存在：{NTEX_DVI}（先 cargo build -p ntex-dvi）")
    src_mtime = _latest_src_mtime()
    bin_mtime = NTEX_DVI.stat().st_mtime
    if bin_mtime >= src_mtime:
        return NTEX_DVI
    lag = src_mtime - bin_mtime
    if verbose:
        print(f"[ntex_bin] 二进制落后源码 {lag:.0f}s，自动重建 cargo build -p ntex-dvi …",
              file=sys.stderr)
    t0 = time.time()
    proc = subprocess.run(["cargo", "build", "-p", "ntex-dvi"],
                          cwd=REPO, capture_output=True, text=True)
    if proc.returncode != 0:
        tail = "\n".join(proc.stderr.splitlines()[-15:])
        sys.exit(f"[ntex_bin] 重建失败（拒绝用旧二进制跑诊断）:\n{tail}")
    if verbose:
        print(f"[ntex_bin] 重建完成 {time.time() - t0:.0f}s", file=sys.stderr)
    return NTEX_DVI
