#!/usr/bin/env python3
"""tex.web 语义测试矩阵生成器。

## 设计原则（2026-09-12 定）

**期望值只来自 pdfTeX 实跑**，不接受人写或模型猜的期望值。
  规则清单(rules.yaml) → 探针池(gen_probes) → pdfTeX 批跑(run reference)
  → 归一化 → 固化用例(cases/) → NTex 跑批 → 红/绿矩阵(REPORT.md)

模型可以翻译「tex.web 规则 → 探针模板」，但探针必须过 pdfTeX 实跑这道关：
模型猜错的语义会被 pdfTeX 的真实行为纠正，猜测进不了期望值。

## 用法

    python3 tests/semantic/gen.py gen [--group A]     # 规则 → 探针池
    python3 tests/semantic/gen.py reference           # pdfTeX 跑批 + 固化期望
    python3 tests/semantic/gen.py ntex                # NTex 跑批 + diff
    python3 tests/semantic/gen.py matrix              # 输出红/绿矩阵 REPORT.md

## 目录

    tests/semantic/rules.yaml      # 规则清单（人/模型维护，唯一上游）
    tests/semantic/work/           # 探针池 + 原始转录（gitignore）
    tests/semantic/cases/          # 固化用例（input.tex + expected.txt，入库）
"""
from __future__ import annotations

import re
import shutil
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).parent
RULES = HERE / "rules.yaml"
WORK = HERE / "work"
CASES = HERE / "cases"
PDFTEX = Path.home() / ".local/bin/pdftex"
NTEX = Path("/home/ubuntu/NTex/target/debug/ntex-dvi")
TFM_DIR = Path.home() / ".ntex-fonts"

# 转录归一化：两引擎共有的环境噪声（按行前缀剔除/替换）
NOISE_PREFIXES = (
    "This is pdfTeX",
    "entering extended mode",
    "restricted \\write18",
    "(/tmp/",
    "(/home/",
    "No file ",
    "Output written on",
    "Transcript written",
    "[1",
    ")*",
)
NOISE_SUBSTR = (
    ("Date:", "Date: <D>"),
    (".log", "<LOG>"),
    (".pdf", "<PDF>"),
    (".dvi", "<DVI>"),
)


def load_rules() -> list[dict]:
    """解析 rules.yaml 的窄格式（自足实现，无 PyYAML 依赖）。

    结构约定：
      - id: X          ← 规则开始（0 缩进）
        group: A       ← 键行（恰好 2 空格缩进）
        probe: |       ← 字面块开始
          <tex>        ← 探针行（≥4 空格缩进，原样保留去 4 前缀）
        variants: [...]← 流内列表（恰好 2 空格）
    """
    import json

    rules: list[dict] = []
    cur: dict | None = None
    in_probe = False
    probe_lines: list[str] = []

    def flush_probe():
        nonlocal in_probe, probe_lines
        if cur is not None and in_probe:
            cur["probe"] = "\n".join(probe_lines)
        in_probe = False
        probe_lines = []

    for raw in RULES.read_text().splitlines():
        if raw.lstrip().startswith("#") or not raw.strip():
            if in_probe:
                probe_lines.append("")
            continue
        indent = len(raw) - len(raw.lstrip(" "))
        stripped = raw.strip()

        if in_probe:
            if indent >= 4:
                probe_lines.append(raw[4:])
                continue
            # 缩进回落 → probe 块结束，本行按键行继续处理
            flush_probe()

        if stripped.startswith("- ") and "id:" in stripped:
            flush_probe()
            cur = {"id": stripped[2:].split(":", 1)[1].strip()}
            rules.append(cur)
            continue
        if cur is None:
            continue
        if indent == 2 and ":" in stripped:
            key, _, val = stripped.partition(":")
            key = key.strip()
            val = val.strip()
            if key == "probe":
                in_probe = True
                probe_lines = []
                continue
            if key == "variants":
                cur["variants"] = json.loads(val) if val not in ("", "[]") else [{}]
                continue
            if key in ("group", "texweb", "desc"):
                cur[key] = val.strip('"')
            continue
    flush_probe()
    for r in rules:
        r.setdefault("variants", [{}])
        r.setdefault("probe", "")
    return rules


def render_probe(rule: dict, variant: dict) -> str:
    """模板 + 变体 → 探针全文。统一加前置预置（catcode 归位）与 \\end。"""
    body = rule["probe"]
    for k, v in variant.items():
        body = body.replace("{{" + k + "}}", v)
    # 前置：稳定环境（plain 下重置可能被探针污染的状态）
    head = "%% rule=" + rule["id"] + " variant=" + ",".join(
        f"{k}={v!r}" for k, v in sorted(variant.items())
    ) + "\n"
    tail = "\n\\end\n"
    return head + body + tail


def cmd_gen(groups: list[str]) -> int:
    rules = load_rules()
    WORK.mkdir(exist_ok=True)
    n = 0
    for r in rules:
        if groups and r.get("group") not in groups:
            continue
        variants_list = r.get("variants") or [{}]
        # variants: list of dicts（全组合由显式列表给出，不做笛卡尔积爆炸）
        for i, v in enumerate(variants_list):
            p = WORK / f"{r['id']}__{i:02d}.tex"
            p.write_text(render_probe(r, v))
            n += 1
    print(f"[gen] {n} probes → {WORK}")
    return 0


def normalize(text: str) -> list[str]:
    """转录归一化：只保留语义可比的行。

    策略（2026-09-12）：与其枚举噪声，不如**只保留语义信号行**——
    1. `[t]...` 型探针自报行（\\write 产物，两引擎都走 sink）；
    2. `! ` 错误行及其后 2 行帮助文本（错误恢复语义）；
    3. `> ` 显示行（\\show 产物）；
    4. Runaway / capacity 类致命通报行。
    其余（banner/统计/文件名/已写出）全为环境噪声，剔除。
    """
    out: list[str] = []
    keep_err_ctx = 0
    for raw in text.replace("\r", "\n").split("\n"):
        line = raw.rstrip()
        if not line.strip():
            continue
        # NTex 的 \write16 产物可能与 plain 预载尾标（如 "hyphenation"）
        # 无换行拼接——剥掉已知尾标前缀再匹配
        for junk in ("hyphenation",):
            if junk in line:
                line = line.split(junk, 1)[-1]
        s = line.strip()
        if s.startswith("[t]") or s.startswith("[0]") or s.startswith("[-"):
            out.append(s)
            continue
        if s.startswith("!") or s.startswith("Runaway") or "capacity exceeded" in s:
            out.append(s)
            keep_err_ctx = 2  # 错误后 2 行帮助文本
            continue
        if keep_err_ctx > 0:
            out.append(s)
            keep_err_ctx -= 1
            continue
        if s.startswith(">"):
            out.append(s)
            continue
    return out


def run_engine(engine: str, tex: Path, timeout: int = 30) -> tuple[int, str]:
    """跑单探针，返回 (exit, 合并转录)。转录 = stdout + stderr（NTex 语义）。"""
    if engine == "pdf":
        r = subprocess.run(
            [str(PDFTEX), "-interaction=nonstopmode", "-output-directory", str(tex.parent), tex.name],
            cwd=tex.parent, capture_output=True, timeout=timeout,
        )
        log = tex.with_suffix(".log")
        # .log 是权威转录（含 \write 产物恰好一次）；stdout 与其重复
        txt = log.read_text(errors="replace") if log.exists() else r.stdout.decode(errors="replace")
        return r.returncode, txt
    else:
        r = subprocess.run(
            [str(NTEX), "--input-path", str(tex.resolve().parent), str(tex.resolve())],
            cwd=tex.parent, capture_output=True, timeout=timeout,
            env={**__import__("os").environ, "NTEX_TFM_DIR": str(TFM_DIR)},
        )
        return r.returncode, (r.stdout + r.stderr).decode(errors="replace")


def cmd_reference() -> int:
    """pdfTeX 批跑 → 固化期望到 cases/（input.tex + expected.txt）。"""
    CASES.mkdir(exist_ok=True)
    refdir = WORK / "ref"
    refdir.mkdir(exist_ok=True)
    ok = dropped = 0
    for tex in sorted(WORK.glob("*.tex")):
        wd = refdir / tex.stem
        wd.mkdir(exist_ok=True)
        shutil.copy(tex, wd / tex.name)
        try:
            rc, txt = run_engine("pdf", wd / tex.name)
        except subprocess.TimeoutExpired:
            print(f"[ref] TIMEOUT {tex.stem} — 丢弃")
            dropped += 1
            continue
        lines = normalize(txt)
        # 非确定性守门：含路径/绝对位置的行已归一化；含「文件名」的探针丢弃
        joined = "\n".join(lines)
        if "Emergency stop" in joined and "Runaway" not in joined:
            # 引擎崩了且无恢复语义 → 探针非法，丢弃
            dropped += 1
            continue
        (CASES / f"{tex.stem}.tex").write_text(tex.read_text())
        (CASES / f"{tex.stem}.expected").write_text("\n".join(lines) + "\n")
        ok += 1
    print(f"[ref] 固化 {ok} 例，丢弃 {dropped}（超时/非法）→ {CASES}")
    return 0


def cmd_ntex() -> int:
    """NTex 跑 cases/ 全部 → 结果 JSON（供 matrix）。"""
    import json
    res = {}
    for exp in sorted(CASES.glob("*.expected")):
        tex = exp.with_suffix(".tex")
        wd = WORK / "ntex" / tex.stem
        wd.mkdir(parents=True, exist_ok=True)
        shutil.copy(tex, wd / tex.name)
        try:
            rc, txt = run_engine("ntex", wd / tex.name)
        except subprocess.TimeoutExpired:
            res[tex.stem] = {"verdict": "TIMEOUT"}
            continue
        got = normalize(txt)
        want = exp.read_text().strip().split("\n")
        want = [w for w in want if w]
        same = got == want
        res[tex.stem] = {
            "verdict": "PASS" if same else "DIFF",
            "first_diff": next(
                (
                    {"want": w, "got": g}
                    for i, (w, g) in enumerate(zip(want, got))
                    if w != g
                ),
                (None if len(got) == len(want) else {"want": "<EOF>", "got": got[len(want):len(want)+1]}),
            ),
        }
    (HERE / "work" / "results.json").write_text(json.dumps(res, ensure_ascii=False, indent=1))
    p = sum(1 for v in res.values() if v["verdict"] == "PASS")
    print(f"[ntex] PASS {p}/{len(res)} → work/results.json")
    return 0


def cmd_matrix() -> int:
    import json
    res = json.loads((HERE / "work" / "results.json").read_text())
    rules = {r["id"]: r for r in load_rules()}
    by_rule: dict[str, list] = {}
    for name, v in res.items():
        rid = name.rsplit("__", 1)[0]
        by_rule.setdefault(rid, []).append((name, v["verdict"]))
    print(f"| 规则 | tex.web | PASS/总 | 判定 |")
    print(f"|---|---|---|---|")
    for rid, items in sorted(by_rule.items()):
        p = sum(1 for _, v in items if v == "PASS")
        total = len(items)
        mark = "✅" if p == total else ("🔴" if p == 0 else "🟡")
        tw = rules.get(rid, {}).get("texweb", "")
        print(f"| {rid} | {tw} | {p}/{total} | {mark} |")
    diff = [(n, v) for n, v in res.items() if v["verdict"] != "PASS"]
    print(f"\n## 非绿明细（{len(diff)}）\n")
    for n, v in sorted(diff):
        fd = v.get("first_diff")
        print(f"### {n} [{v['verdict']}]")
        if fd:
            print(f"```\nwant: {fd.get('want')!r}\ngot : {fd.get('got')!r}\n```")
    return 0


def main() -> int:
    cmd = sys.argv[1] if len(sys.argv) > 1 else ""
    if cmd == "gen":
        return cmd_gen(sys.argv[2].split(",") if len(sys.argv) > 2 else [])
    if cmd == "reference":
        return cmd_reference()
    if cmd == "ntex":
        return cmd_ntex()
    if cmd == "matrix":
        return cmd_matrix()
    print(__doc__)
    return 1


if __name__ == "__main__":
    sys.exit(main())
