#!/usr/bin/env python3
"""JSONL trace 查询器：把 `NTEX_TRACE_JSONL` 的转储变成可回答的问题。

## 为什么需要它

`eprintln` 打散文本只给「最后一帧 + 栈快照」，无法回答**「是谁 push 了它」**。
JSONL 每行一个事件，本工具按字段查询——定位「输入栈为什么膨胀」这类问题的核心。

## 用法

    # 概览：各 kind 计数 + 栈深峰值 + 峰值出现处
    scripts/trace-view.py /tmp/t.jsonl --summary

    # 按 token 名过滤（子串匹配）
    scripts/trace-view.py /tmp/t.jsonl --grep cs_generate

    # 第 N 步前后各 20 条
    scripts/trace-view.py /tmp/t.jsonl --around 50000 --window 20

    # 某一刻的完整栈（从 step N 的 push/pop 事件重建）
    scripts/trace-view.py /tmp/t.jsonl --stack-at 49999

    # 找栈深增长最快的区间（定位自我复制的宏链）
    scripts/trace-view.py /tmp/t.jsonl --spikes --top 10

    # 某 frame 类型的全部出现
    scripts/trace-view.py /tmp/t.jsonl --frame Macro
"""
import argparse
import json
import sys
from collections import Counter
from pathlib import Path


def load(path: Path) -> list[dict]:
    events = []
    with path.open(errors="replace") as f:
        for i, line in enumerate(f, 1):
            line = line.strip()
            if not line:
                continue
            try:
                events.append(json.loads(line))
            except json.JSONDecodeError:
                print(f"[trace-view] 第 {i} 行不是合法 JSON，跳过", file=sys.stderr)
    return events


def cmd_summary(ev: list[dict]) -> None:
    if not ev:
        print("（无事件）")
        return
    kinds = Counter(e.get("kind", "?") for e in ev)
    print(f"事件总数: {len(ev)}")
    print(f"step 范围: {ev[0].get('step')} … {ev[-1].get('step')}")
    print("kind 分布: " + ", ".join(f"{k}={v}" for k, v in kinds.most_common()))
    peak = max(ev, key=lambda e: e.get("depth", 0))
    print(f"栈深峰值: {peak.get('depth')} @ step {peak.get('step')}"
          f"  frame={peak.get('frame')}  tok={peak.get('tok')}")
    tops = Counter(e.get("tok") for e in ev if e.get("tok"))
    print("高频 token top5: " + ", ".join(f"{t}({c})" for t, c in tops.most_common(5)))


def cmd_grep(ev: list[dict], needle: str, limit: int) -> None:
    hits = [e for e in ev if needle in json.dumps(e, ensure_ascii=False)]
    print(f"匹配 {needle!r}: {len(hits)} 条"
          f"{f'（显示前 {limit}）' if len(hits) > limit else ''}")
    for e in hits[:limit]:
        print(f"  step={e.get('step')} {e.get('kind')} depth={e.get('depth')}"
              f" tok={e.get('tok')} frame={e.get('frame')}")


def cmd_around(ev: list[dict], step: int, window: int) -> None:
    lo, hi = step - window, step + window
    hits = [e for e in ev if lo <= e.get("step", -1) <= hi]
    print(f"step {lo}..{hi} 共 {len(hits)} 条：")
    for e in hits:
        mark = "→" if e.get("step") == step else " "
        print(f" {mark} step={e.get('step')} {e.get('kind'):<6}"
              f" depth={e.get('depth')} tok={e.get('tok')} frame={e.get('frame')}")


def cmd_stack_at(ev: list[dict], step: int) -> None:
    """从事件流重建某一步的栈（简化模型：push 加、pop 减）。"""
    stack: list[str] = []
    for e in ev:
        if e.get("step", 0) > step:
            break
        k = e.get("kind")
        if k == "push":
            stack.append(f"{e.get('frame')}  ←tok={e.get('tok')}")
        elif k == "pop" and stack:
            stack.pop()
    print(f"step ≤ {step} 时的栈（{len(stack)} 帧，自底向上）：")
    for i, fr in enumerate(stack):
        print(f"  [{i:>3}] {fr}")


def cmd_spikes(ev: list[dict], top: int) -> None:
    """栈深增长最快的区间——自我复制宏链的信号。"""
    by_step = [(e.get("step", 0), e.get("depth", 0)) for e in ev]
    if len(by_step) < 2:
        print("（事件太少）")
        return
    # 用 window=200 事件的滑动窗口算深度增量
    w = 200
    spikes = []
    for i in range(0, max(1, len(by_step) - w)):
        d0 = by_step[i][1]
        d1 = by_step[i + w][1]
        spikes.append((d1 - d0, by_step[i][0], by_step[i + w][0], d0, d1))
    spikes.sort(reverse=True)
    print(f"栈深增长最快区间（窗口 {w} 事件）top{top}：")
    for delta, s0, s1, d0, d1 in spikes[:top]:
        print(f"  +{delta:<5} step {s0}..{s1}  depth {d0} → {d1}")


def cmd_frame(ev: list[dict], kind: str, limit: int) -> None:
    hits = [e for e in ev if kind in (e.get("frame") or "")]
    print(f"frame 含 {kind!r}: {len(hits)} 条")
    for e in hits[:limit]:
        print(f"  step={e.get('step')} depth={e.get('depth')} frame={e.get('frame')}")


def main() -> int:
    ap = argparse.ArgumentParser(description="NTex JSONL trace 查询器")
    ap.add_argument("path", help="NTEX_TRACE_JSONL 产出的 .jsonl")
    ap.add_argument("--summary", action="store_true", help="概览")
    ap.add_argument("--grep", help="按子串过滤（整条事件的 JSON）")
    ap.add_argument("--around", type=int, help="第 N 步前后")
    ap.add_argument("--window", type=int, default=20, help="--around 的窗口（默认 20）")
    ap.add_argument("--stack-at", type=int, help="重建第 N 步的栈")
    ap.add_argument("--spikes", action="store_true", help="栈深增长最快区间")
    ap.add_argument("--top", type=int, default=10, help="--spikes 条数")
    ap.add_argument("--frame", help="按 frame 类型过滤")
    ap.add_argument("--limit", type=int, default=40, help="列表类输出的条数上限")
    args = ap.parse_args()

    path = Path(args.path)
    if not path.is_file():
        print(f"[trace-view] 找不到 {path}", file=sys.stderr)
        return 2
    ev = load(path)
    if not ev:
        print("[trace-view] 无事件（NTEX_TRACE_JSONL 是否设了？）", file=sys.stderr)
        return 1

    did = False
    if args.summary or not any([args.grep, args.around, args.stack_at,
                                args.spikes, args.frame]):
        cmd_summary(ev)
        did = True
    if args.grep:
        cmd_grep(ev, args.grep, args.limit)
        did = True
    if args.around is not None:
        cmd_around(ev, args.around, args.window)
        did = True
    if args.stack_at is not None:
        cmd_stack_at(ev, args.stack_at)
        did = True
    if args.spikes:
        cmd_spikes(ev, args.top)
        did = True
    if args.frame:
        cmd_frame(ev, args.frame, args.limit)
        did = True
    return 0 if did else 1


if __name__ == "__main__":
    sys.exit(main())
