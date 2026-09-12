#!/usr/bin/env python3
"""栈帧现场采集器（基础设施②）：跑一个 .lvt/.tex 用例，抓爆栈时的逐帧转储。

用法：
    python3 scripts/frame_dump.py m3basics001 [--head 80] [--out /tmp/frames.txt]

依赖：NTex 二进制 + EXPL3_SRC 环境变量（或默认 /tmp/l3kernel-tests/testfiles）。
输出：栈顶 80 帧的 TokenList 头部内容 → 定位「谁在无限展开」。
"""
import argparse
import os
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
from ntex_bin import ensure_fresh

NT = ensure_fresh()


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("case", help="用例名（如 m3basics001）或 .tex 路径")
    ap.add_argument("--head", type=int, default=80, help="转储帧数")
    ap.add_argument("--out", default="/tmp/frames.txt")
    ap.add_argument("--timeout", type=int, default=180)
    a = ap.parse_args()

    src = Path(os.environ.get("EXPL3_SRC", "/tmp/l3kernel-tests/testfiles"))
    wd = Path(tempfile.mkdtemp(prefix="framedump-"))
    for f in ("lvt-shim.tex", "regression-test.tex", "regression-test.cfg",
              "exgeneric.tex", "expl3-code.tex", "expl3.ltx"):
        if (src / f).exists():
            (wd / f).write_bytes((src / f).read_bytes())
    lvt = src / f"{a.case}.lvt"
    if lvt.exists():
        (wd / lvt.name).write_bytes(lvt.read_bytes())
        (wd / "run.tex").write_text(
            f"\\chardef\\lvtuseexplthree=1\n\\def\\LVTFILE{{{lvt.name}}}\n"
            f"\\input {wd}/lvt-shim\n")
    else:
        p = Path(a.case)
        (wd / "run.tex").write_bytes(p.read_bytes())

    env = {**os.environ,
           "NTEX_TFM_DIR": os.path.expanduser("~/.ntex-fonts"),
           "NTEX_STACK_DUMP": "1",
           "NTEX_STACK_DUMP_FRAMES": "1"}
    r = subprocess.run([str(NT), "--input-path", str(wd), str(wd / "run.tex")],
                       capture_output=True, timeout=a.timeout, env=env)
    text = (r.stdout + r.stderr).decode(errors="replace")

    frames = [ln for ln in text.splitlines() if ln.startswith("[frame ")]
    head = [ln for ln in text.splitlines() if ln.startswith("[stack-dump]")]
    tail = [ln for ln in text.splitlines() if "排版失败" in ln]

    with open(a.out, "w") as fh:
        fh.write("\n".join(head + frames + tail) + "\n")
    print(f"[frame-dump] {len(frames)} 帧 → {a.out}")
    print("\n".join(head))
    for ln in frames[:a.head]:
        print(ln[:150])
    if tail:
        print(tail[0][:120])
    return 0


if __name__ == "__main__":
    sys.exit(main())
