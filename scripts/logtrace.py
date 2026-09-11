#!/usr/bin/env python3
"""log/转录分析器：把 5000+ 行错误洪流压缩成**可行动的结构化报告**。

## 为什么需要它

latex.ltx 的转录有 **5620 条错误**，人工 grep 只能看到「数量最多的那类」，
但真正要回答的是**结构性问题**：

- 错误的**时间序**里，第一个「新出现」的错误是哪条？（首现场）
- 哪一行是**震中**（最多错误指向它）？哪一行是真正的**起点**？
- **错误级联的形状**：同一错误重复 N 次（扇出）、还是 N 类错误依次出现（链式）？
- 与 pdfTeX 参考 log 相比，哪些错误是**我们独有**的？

零散 grep 回答不了这些。本工具把转录解析成事件流后做结构分析。

## 用法

    # 结构化报告（首现序 / 震中行 / 级联形状 / 归一化计数）
    scripts/logtrace.py /tmp/r27b/latex.ltx.transcript

    # 只看错误（默认）
    scripts/logtrace.py FILE --errors

    # 只看 '! ' 之外的行：\\tracingcommands / {restoring} / 文件事件
    scripts/logtrace.py FILE --kinds trace,restore,file

    # 自动定位「级联起点」：第一条非重复（首次出现）的错误 + 前后上下文
    scripts/logtrace.py FILE --first-new --context 8

    # 与 pdfTeX 参考 log 对比（找"我方独有"的错误）
    scripts/logtrace.py FILE --compare ref.log

    # 导出归一化错误序列（供 blocker-track 或跨轮比对）
    scripts/logtrace.py FILE --emit-seq /tmp/seq.txt

## 输入形态

本工具同时吃两类输入（自动嗅探）：
1. **NTex 转录**（`take_transcript()` / `NTEX_KEEP_TRANSCRIPT` / latex_probe 的
   `.transcript`）——错误行 `! ...` + 位置行 `l.NNN`（可能带 `| ` 前缀）
2. **pdfTeX log**（`-interaction=nonstopmode` 的 `.log`）——同样 `! ...` + `l.NNN`，
   但错误间夹杂文件事件 `(./x.tex` / `)` 与 `[1{...}]` 页事件

## 输出结构

    ① 概览：总行数 / 错误数 / 归一化后错误种类数 / 位置行数
    ② 首现场：第一条错误（含滚动窗口内的上下文）
    ③ 震中：错误最集中的位置行 topN
    ④ 级联形状：重复度最高的错误（扇出） vs 出现位置最分散的错误（链式传播）
    ⑤ 归一化错误序列（去数字/去标识符尾巴），可直接跨轮 diff

## 关键概念：归一化

`Missing number, treated as zero` 与 `Missing = inserted for \\ifnum` 是不同类型；
但 `Font \\cmr10 has only 7 fontdimen` 与 `Font \\cmbx10 has only 7 ...` 是同一类。
归一化 = 去数字 + 去 cs 名（`\\X` → `\\?`），让计数有意义。
"""
import argparse
import re
import sys
from collections import Counter
from pathlib import Path

# 错误首行：`! ...`（NTex 转录可能带 `| ` 前缀）
ERR_RX = re.compile(r"^(?:\|\s*)?!\s?(.*)$")
# 位置行：`l.NNN ...`（可带 `| ` 前缀）
LOC_RX = re.compile(r"^(?:\|\s*)?l\.(\d+)\b")
# 文件事件：`(./path` 或 `)` 行
FILE_RX = re.compile(r"^[()]")
# 页事件：`[1{...}]`
PAGE_RX = re.compile(r"^\[\d+")
# tracing 类：`{...}` 开头的行（\tracingcommands / {restoring ...} / {changing ...}）
TRACE_RX = re.compile(r"^\s*\{")
# \show / \meaning 回显：`> ...`
SHOW_RX = re.compile(r"^>\s")


def normalize(msg: str) -> str:
    """归一化错误消息：去数字、去具体 cs 名/字体名，保留结构。"""
    s = msg.strip()
    s = re.sub(r"\\[A-Za-z@:_]+\d*", r"\\?", s)      # \cmr10 -> \?
    s = re.sub(r"\b\d+\b", "N", s)                    # 数字 -> N
    s = re.sub(r"\s+", " ", s)
    return s[:80]


class Event:
    __slots__ = ("idx", "kind", "text", "loc", "norm")

    def __init__(self, idx: int, kind: str, text: str, loc=None):
        self.idx, self.kind, self.text = idx, kind, text
        self.loc = loc
        self.norm = normalize(text) if kind == "error" else text.strip()[:80]


def parse(path: Path, kinds: set[str]) -> list[Event]:
    """解析成事件流。error 事件会向后吸收紧邻的 `l.NNN` 作为位置。"""
    events: list[Event] = []
    lines = path.read_text(errors="replace").splitlines()
    pending: Event | None = None
    for i, line in enumerate(lines):
        m = ERR_RX.match(line)
        if m:
            pending = Event(i, "error", m.group(1))
            events.append(pending)
            continue
        lm = LOC_RX.match(line)
        if lm:
            if pending is not None and pending.loc is None:
                pending.loc = int(lm.group(1))
            elif "file" in kinds or True:
                # 无主位置行：附到上一个事件后作为独立事件（保留信息）
                events.append(Event(i, "loc", line.strip(), int(lm.group(1))))
            continue
        if TRACE_RX.match(line):
            events.append(Event(i, "trace", line))
        elif SHOW_RX.match(line):
            events.append(Event(i, "show", line))
        elif PAGE_RX.match(line):
            events.append(Event(i, "page", line))
        elif FILE_RX.match(line):
            events.append(Event(i, "file", line))
    if not kinds:
        return events
    return [e for e in events if e.kind in kinds]


def report(events: list[Event], path: Path, *, first_new: bool,
           context: int, topn: int) -> int:
    errs = [e for e in events if e.kind == "error"]
    total_lines = len(path.read_text(errors="replace").splitlines())

    print(f"== logtrace: {path}")
    print(f"   总行数 {total_lines}  |  事件 {len(events)}  |  错误 {len(errs)}"
          f"  |  归一化后错误种类 {len({e.norm for e in errs})}")
    if not errs:
        print("   ✅ 无错误")
        return 0

    # ① 首现场
    first = errs[0]
    print(f"\n① 首现场（第 {first.idx + 1} 行）")
    print(f"   {first.text.strip()[:120]}")
    if first.loc:
        print(f"   @ l.{first.loc}")
    if context:
        lo, hi = max(0, first.idx - context), min(total_lines, first.idx + context)
        print(f"   ── 上下文（行 {lo + 1}..{hi}）──")
        for l in path.read_text(errors="replace").splitlines()[lo:hi]:
            print(f"     {l[:110]}")

    # ② 震中：错误最集中的位置行
    locs = Counter(e.loc for e in errs if e.loc)
    print(f"\n② 震中（错误最多的位置行）top{min(topn, len(locs))}")
    for loc, n in locs.most_common(topn):
        share = n / len(errs) * 100
        print(f"   l.{loc:<8} {n:>5} 条 ({share:4.1f}%)")

    # ③ 级联形状
    norms = Counter(e.norm for e in errs)
    print(f"\n③ 级联形状")
    print(f"   ▸ 扇出（同一错误重复最多）：")
    for msg, n in norms.most_common(3):
        print(f"     {n:>5}× {msg}")
    # 链式：出现位置最分散的
    spread = []
    for msg in norms:
        lset = {e.loc for e in errs if e.norm == msg and e.loc}
        if len(lset) > 1:
            spread.append((len(lset), msg))
    spread.sort(reverse=True)
    if spread:
        print(f"   ▸ 链式（同一错误跨最多位置行）：")
        for n, msg in spread[:3]:
            print(f"     {n:>5} 个位置  {msg}")

    # ④ 首次出现序：错误种类的"冒头"顺序 —— 级联起点的候选
    seen: set[str] = set()
    order: list[tuple[int, int, str]] = []
    for e in errs:
        if e.norm not in seen:
            seen.add(e.norm)
            order.append((e.idx, e.loc or 0, e.norm))
    print(f"\n④ 首次出现序（{len(order)} 种，前 {min(topn, len(order))} 条）")
    print("   —— 第一个「新种类」往往比「第一个错误」更接近真起点")
    for idx, loc, norm in order[:topn]:
        print(f"   #{idx + 1:<7} l.{loc:<7} {norm}")

    if first_new and order:
        idx, loc, norm = order[0]
        print(f"\n★ 级联起点候选：第 {idx + 1} 行 l.{loc}")
        print(f"   {norm}")

    # ⑤ 归一化序列（可跨轮 diff）
    print(f"\n⑤ 归一化错误序列（前 12 条 / 共 {len(errs)}）")
    for e in errs[:12]:
        print(f"   {e.norm}")

    return 0


def compare(events: list[Event], ref: Path, topn: int) -> int:
    ref_events = parse(ref, {"error"})
    mine = Counter(e.norm for e in events if e.kind == "error")
    theirs = Counter(e.norm for e in ref_events)
    only_mine = [(n, m) for m, n in mine.items() if m not in theirs]
    only_theirs = [(n, m) for m, n in theirs.items() if m not in mine]
    shared = [(m, mine[m], theirs[m]) for m in mine if m in theirs]
    only_mine.sort(reverse=True)
    only_theirs.sort(reverse=True)
    shared.sort(key=lambda t: -(t[1] + t[2]))

    print(f"\n⑥ 与参考 log 对比：{ref}")
    print(f"   我方 {sum(mine.values())} 条 / 参考 {sum(theirs.values())} 条")
    print(f"\n   ▸ 我方独有（参考没有）top{min(topn, len(only_mine))}"
          f" —— **这些才是真偏差**")
    for n, m in only_mine[:topn]:
        print(f"     {n:>5}× {m}")
    print(f"\n   ▸ 参考有、我方没有 top{min(topn, len(only_theirs))}"
          f" —— 我方缺错误（可能是吞错/语义缺失）")
    for n, m in only_theirs[:topn]:
        print(f"     {n:>5}× {m}")
    print(f"\n   ▸ 共有（数量差异看恢复质量）top{min(topn, len(shared))}")
    for m, a, b in shared[:topn]:
        flag = "" if abs(a - b) <= max(2, b * 0.2) else "  ⚠ 数量差异大"
        print(f"     我方{a:>5} / 参考{b:>5}  {m}{flag}")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description="NTex 转录/pdfTeX log 结构分析")
    ap.add_argument("path", help="转录或 log 文件")
    ap.add_argument("--errors", action="store_true", help="只看错误（默认）")
    ap.add_argument("--kinds", help="事件类别白名单，逗号分隔："
                                    "error,loc,trace,show,page,file")
    ap.add_argument("--first-new", action="store_true",
                    help="高亮「首次出现的新错误种类」作为级联起点候选")
    ap.add_argument("--context", type=int, default=0,
                    help="首现场前后的上下文行数")
    ap.add_argument("--top", type=int, default=6, help="各榜条数")
    ap.add_argument("--compare", help="与 pdfTeX 参考 log 对比")
    ap.add_argument("--emit-seq", help="导出归一化错误序列到文件")
    args = ap.parse_args()

    path = Path(args.path)
    if not path.is_file():
        print(f"[logtrace] 找不到 {path}", file=sys.stderr)
        return 2

    kinds = set(args.kinds.split(",")) if args.kinds else ({"error"} if args.errors
                                                           or not args.kinds else set())
    if args.compare:
        kinds = {"error"}
    if args.emit_seq:
        kinds = {"error"}

    events = parse(path, kinds)

    if args.emit_seq:
        out = Path(args.emit_seq)
        errs = [e for e in events if e.kind == "error"]
        out.write_text("\n".join(e.norm for e in errs) + "\n")
        print(f"已导出 {len(errs)} 条归一化错误 → {out}")
        return 0

    rc = report(events, path, first_new=args.first_new,
                context=args.context, topn=args.top)
    if args.compare:
        compare(events, Path(args.compare), args.top)
    return rc


if __name__ == "__main__":
    sys.exit(main())
