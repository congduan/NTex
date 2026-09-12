#!/usr/bin/env python3
"""错误恢复语义对拍矩阵（基础设施③，2026-09-12 设立）。

背景：fh9.tex 定位战打了 25 个手工变体、结论反转 3 次，暴露「错误恢复类
定位无工具承载」——abcheck 只对拍正常输出，报错文本+恢复后行为没有对拍
通道；变体无沉淀，结论随会话蒸发。

本工具把「错误恢复语义」做成语料库 + 三模式 runner：

    freeze  跑 pdfTeX，把 (首错, write 输出) 冻结进 oracle.expect
            （oracle 本身是仪器——冻结前须人工核对语义，见 tooling-trust.md 事故六）
    verify  复跑 pdfTeX 对比冻结值 —— oracle 仪器自检（防环境漂移/误改）
    run     跑 NTex 对比冻结 oracle —— 发散地图（DIFF 是预期，修复后看单调转绿）

语料库结构（fixtures/recovery/cases/<name>/）：
    case.tex        自包含输入（\\write16 用 marker 行，如 `X:[...]`）
    oracle.expect   冻结的 pdfTeX 判据（error:/output: 行）
    note.md 可选    人工核对记录（该 case 建立时确认了什么语义事实）

判读纪律：
  - run 的 DIFF ≠ 失败——是「NTex 与 pdfTeX 恢复行为发散」的地图；
  - 修复验收 = 指定 case DIFF→OK，且 make oracle-verify 无 DRIFT；
  - 新 case 必须 freeze 前人工核对（pdfTeX 在恢复 edge case 上的行为
    本身可能是未建档语义，见 chk1 输出 [Z] 的教训）。
"""
import argparse
import subprocess
import sys
import tempfile
from datetime import date
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
CASES_DIR = REPO / "fixtures/recovery/cases"
PDFTEX = Path.home() / ".local/bin/pdftex"

# 防旧二进制门禁（tooling-trust 事故七）：诊断前自动重建落后二进制
from ntex_bin import ensure_fresh
NTEX = ensure_fresh()

# marker 判据：`A:[NZ]`——名字前必须有非标识符边界（空格/行首），
# 防把 NTex banner 粘连（`hyphenationA:[NZ]`）整段当名字。
MARKER_RE = r"(?<![A-Za-z0-9_])([A-Za-z][A-Za-z0-9_]*):\[([^\]]*)\]"


def run_engine(engine: str, case_tex: Path, timeout: int) -> tuple[str, list[str]]:
    """跑单 case，返回 (首错原文或空串, marker 输出行列表)。"""
    with tempfile.TemporaryDirectory() as td:
        td_path = Path(td)
        (td_path / "case.tex").write_text(case_tex.read_text())
        if engine == "pdftex":
            cmd = [str(PDFTEX), "-interaction=nonstopmode", "case.tex"]
        else:
            cmd = [str(NTEX), "case.tex"]
        try:
            proc = subprocess.run(
                cmd, cwd=td_path, capture_output=True, text=True, timeout=timeout
            )
        except subprocess.TimeoutExpired:
            return "! TIMEOUT", []
        if engine == "pdftex":
            log = (td_path / "case.log")
            text = log.read_text(errors="replace") if log.exists() else ""
            if not text:
                text = proc.stdout
        else:
            # NTex 转录（格式装载/排版输出）走 stderr（环境约定），
            # 错误摘要也可能在任一流——合并扫描。
            text = proc.stdout + proc.stderr
        err = ""
        out: list[str] = []
        # marker 全文提取（不按行首）：NTex 的 \write16 输出可能不带前置换行，
        # 粘在格式装载输出尾（如 `hyphenationA:[NZ]`）。
        import re
        out = re.findall(r"[A-Za-z][A-Za-z0-9_]*:\[[^\]]*\]", text)
        for line in text.splitlines():
            if line.startswith("! ") and not err:
                err = line
            elif line.startswith("排版失败") and not err:
                err = line
        return err, out


def parse_oracle(path: Path) -> tuple[str, list[str]]:
    err, out = "", []
    for line in path.read_text().splitlines():
        if line.startswith("error: "):
            err = line[len("error: "):]
        elif line.startswith("output: "):
            out.append(line[len("output: "):])
    return err, out


def write_oracle(path: Path, err: str, out: list[str], note: str) -> None:
    lines = [
        f"# oracle 冻结 {date.today().isoformat()}，pdfTeX TL2026，scripts/abmatrix.py freeze",
        f"# 人工核对: {note}",
    ]
    if err:
        lines.append(f"error: {err}")
    for o in out:
        lines.append(f"output: {o}")
    path.write_text("\n".join(lines) + "\n")


def all_cases() -> list[Path]:
    if not CASES_DIR.exists():
        sys.exit(f"语料库不存在: {CASES_DIR}")
    return sorted(p for p in CASES_DIR.iterdir() if (p / "case.tex").is_file())


def short(s: str, width: int = 46) -> str:
    return s if len(s) <= width else s[: width - 1] + "…"


def cmd_freeze(names: list[str], note: str) -> None:
    for case in all_cases():
        if names and case.name not in names:
            continue
        err, out = run_engine("pdftex", case / "case.tex", timeout=30)
        oracle = case / "oracle.expect"
        note_line = note or (oracle.read_text().split("人工核对: ")[-1].splitlines()[0]
                             if oracle.exists() else "待人工核对")
        write_oracle(oracle, err, out, note_line)
        print(f"freeze {case.name}: err=[{short(err)}] out={out}")


def cmd_verify() -> int:
    drift = 0
    total = 0
    for case in all_cases():
        oracle = case / "oracle.expect"
        if not oracle.exists():
            print(f"DRIFT  {case.name}: 无 oracle.expect（先 freeze）")
            drift += 1
            continue
        total += 1
        want = parse_oracle(oracle)
        got = run_engine("pdftex", case / "case.tex", timeout=30)
        if got != want:
            drift += 1
            print(f"DRIFT  {case.name}:\n  冻结 err=[{short(want[0])}] out={want[1]}"
                  f"\n  实测 err=[{short(got[0])}] out={got[1]}")
    print(f"\noracle 自检: {total - drift}/{total} 一致" + (f"，{drift} 例漂移！" if drift else " ✅"))
    return 1 if drift else 0


def cmd_run(strict: bool) -> int:
    ok = diff = 0
    rows = []
    for case in all_cases():
        oracle = case / "oracle.expect"
        if not oracle.exists():
            continue
        want_err, want_out = parse_oracle(oracle)
        got_err, got_out = run_engine("ntex", case / "case.tex", timeout=60)
        # 判定：恢复后 write 输出逐行一致 + 错误有无（不比错误措辞——中英文必异）
        same = got_out == want_out and bool(got_err) == bool(want_err)
        ok, diff = (ok + 1, diff) if same else (ok, diff + 1)
        rows.append((case.name, want_out, got_out, want_err, got_err, same))
    print(f"{'case':<38} {'oracle输出':<16} {'NTex输出':<16} 判定  错误对比")
    for name, wo, go, we, ge, same in rows:
        mark = "OK  " if same else "DIFF"
        print(f"{name:<38} {','.join(wo)[:15]:<16} {','.join(go)[:15]:<16} {mark}"
              f"  pdfTeX=[{short(we, 30)}] NTex=[{short(ge, 30)}]")
    print(f"\n矩阵: OK {ok} / DIFF {diff}（DIFF=恢复语义发散地图，修复后看单调转绿）")
    if strict and diff:
        return 1
    return 0


def main() -> None:
    ap = argparse.ArgumentParser(description=(__doc__ or "abmatrix").split("\n")[0])
    ap.add_argument("mode", choices=["freeze", "verify", "run"])
    ap.add_argument("names", nargs="*", help="freeze: 限定 case 名")
    ap.add_argument("--note", default="", help="freeze: 人工核对记录")
    ap.add_argument("--strict", action="store_true", help="run: 有 DIFF 则退出码 1")
    args = ap.parse_args()
    if args.mode == "freeze":
        cmd_freeze(args.names, args.note)
    elif args.mode == "verify":
        sys.exit(cmd_verify())
    else:
        sys.exit(cmd_run(args.strict))


if __name__ == "__main__":
    main()
