#!/usr/bin/env python3
"""生成前端用的 CJK 子集字体（Fandol Song → NTex Tauri/浏览器可 fetch 的体积）。

为什么需要子集化
----------------
Computer Modern / Latin Modern 没有汉字字形，中文排版必须另挂一个 CJK 字体。
但 Fandol 全量 OTF ~4.9 MB，直接入库对仓库与前端加载都偏重；NTex 前端
（`crates/ntex-tauri/ui/`、`crates/ntex-wasm/www/`）实际只需要**常用汉字**
够用即可——据此按 GB2312 字符集裁剪。

三档（`--tier`）
----------------
| 档位 | 字符集 | 实测体积 | 适用 |
|---|---|---|---|
| `sym`  | ASCII + GB2312 符号区（区 1-9） | ~136 KB | 只要标点/西文 |
| `l1`   | sym + GB2312 一级常用汉字（3755 字） | ~2.0 MB | **前端默认**（日常中文 99.7% 覆盖） |
| `full` | sym + GB2312 全部汉字（6763 字） | ~3.7 MB | 生僻字场景 |

（体积为 2026-09-11 用 fonttools 4.65 实测；随 fonttools / 源字体版本浮动。）

用法
----
    # 1) 取源字体（CTAN，GPL；也可用本地 texlive 里的同名字体）
    curl -L -o /tmp/FandolSong-Regular.otf \\
        https://mirrors.ctan.org/fonts/fandol/FandolSong-Regular.otf

    # 2) 依赖
    pip install fonttools

    # 3) 子集化（默认 l1 档，输出到 Tauri 前端字体目录）
    python3 scripts/make-cjk-subset.py /tmp/FandolSong-Regular.otf

    # 指定档位与输出
    python3 scripts/make-cjk-subset.py /tmp/FandolSong-Regular.otf \\
        -t full -o crates/ntex-wasm/www/fonts/FandolSong-Regular.otf

字体注册名
----------
输出文件名（去扩展名）就是 TeX 侧要用的字体名——NTex 的 `\font` 支持 OpenType
直取（无 TFM 的字体经 `TfmSource::otf_bytes` / `find_otf` 命中），前端 fetch
后用 `set_otf_font('<文件名>', bytes)` 注入即可：

    \\font\\zh=FandolSong-Regular at 11pt

许可
----
Fandol 字体（CTAN `fonts/fandol`）为 GPL 授权，可自由再分发；本脚本只做
字符子集裁剪，不改字形轮廓。
"""

from __future__ import annotations

import argparse
import os
import shutil
import subprocess
import sys

DEFAULT_OUT = "crates/ntex-tauri/ui/fonts/FandolSong-Regular.otf"

# GB2312 区号（区 = 字节 - 0xA0）：
#   1-9   符号（标点、数字、希腊/俄文字母、制表符等）
#   16-55 一级常用汉字 3755 字
#   56-87 二级次常用汉字 3008 字
ZONE_SYMBOLS = (1, 9)
ZONE_HANZI_L1 = (16, 55)
ZONE_HANZI_L2 = (56, 87)

# GB2312 编码不到的常用中文/西文排版字符（**必须显式补**，否则中文正文里的
# 破折号「——」等直接缺字形）。
#
# 典型坑：GB2312 的破折号槽位是 U+2015（HORIZONTAL BAR，Fandol 无字形），
# 而中文书写习惯用 U+2014（EM DASH，Fandol 有字形）——resume-plain.tex 的
# 「NTex —— 现代 TeX 排版引擎」在漏补时就是 `Missing character: no ^^2014`。
EXTRA_PUNCT = (
    "\u2014"  # — em dash（中文破折号，习惯写法）
    "\u2013"  # – en dash
    "\u2018\u2019"  # ‘ ’ 单引号
    "\u201c\u201d"  # “ ” 双引号
    "\u2026"  # … 省略号
    "\u2030"  # ‰
    "\u2032\u2033"  # ′ ″
    "\u203b"  # ※
    "\u00a7\u00b6\u2020\u2021"  # § ¶ † ‡
    "\u2022\u2027"  # • ‥
    "\u00b0\u00b1\u00d7\u00f7"  # ° ± × ÷
    "\u2264\u2265\u2260\u2248\u221e\u221a"  # ≤ ≥ ≠ ≈ ∞ √
    "\u2103"  # ℃
)


def gb2312_chars(zones: list[tuple[int, int]]) -> set[str]:
    """按 GB2312 区位取可解码字符（跳过空洞区）。"""
    out: set[str] = set()
    for lo_zone, hi_zone in zones:
        for hi in range(0xA0 + lo_zone, 0xA0 + hi_zone + 1):
            for lo in range(0xA1, 0xFF):
                try:
                    out.add(bytes([hi, lo]).decode("gb2312"))
                except UnicodeDecodeError:
                    continue
    return out


def tier_chars(tier: str) -> set[str]:
    # ASCII 可打印区（西文/数字/半角标点）+ NBSP：Fandol 自带这些字形，
    # 保留后 `\zh` 激活时西文不会掉字。
    chars = {chr(code) for code in range(0x20, 0x7F)} | {"\u00a0"}
    chars |= set(EXTRA_PUNCT)
    if tier == "sym":
        chars |= gb2312_chars([ZONE_SYMBOLS])
    elif tier == "l1":
        chars |= gb2312_chars([ZONE_SYMBOLS, ZONE_HANZI_L1])
    elif tier == "full":
        chars |= gb2312_chars([ZONE_SYMBOLS, ZONE_HANZI_L1, ZONE_HANZI_L2])
    else:
        raise SystemExit(f"未知档位：{tier}（可选 sym / l1 / full）")
    return chars


def main() -> int:
    ap = argparse.ArgumentParser(description="生成前端用 CJK 子集字体")
    ap.add_argument("source", help="源 OTF（FandolSong-Regular.otf 等）")
    ap.add_argument("-o", "--output", default=DEFAULT_OUT, help=f"输出路径（默认 {DEFAULT_OUT}）")
    ap.add_argument("-t", "--tier", default="l1", choices=["sym", "l1", "full"])
    args = ap.parse_args()

    if not os.path.isfile(args.source):
        print(f"源字体不存在：{args.source}", file=sys.stderr)
        return 1

    try:
        from fontTools.ttLib import TTFont  # noqa: PLC0415 —— 依赖缺失时给友好提示
    except ImportError:
        print("缺 fonttools：pip install fonttools", file=sys.stderr)
        return 1

    cmap = TTFont(args.source).getBestCmap()
    wanted = tier_chars(args.tier)
    # 只保留源字体真有的码位——否则 pyftsubset 会为缺字剥一条警告。
    text = "".join(sorted(c for c in wanted if ord(c) in cmap))
    print(f"档位 {args.tier}：请求 {len(wanted)} 字符，源字体命中 {len(text)}")

    text_file = "/tmp/ntex-cjk-subset-text.txt"
    with open(text_file, "w", encoding="utf-8") as fh:
        fh.write(text)

    pyftsubset = shutil.which("pyftsubset")
    if pyftsubset is None:
        print("缺 pyftsubset（随 fonttools 安装）", file=sys.stderr)
        return 1

    os.makedirs(os.path.dirname(args.output) or ".", exist_ok=True)
    subprocess.run(
        [
            pyftsubset,
            args.source,
            f"--text-file={text_file}",
            f"--output-file={args.output}",
            # 中文不做 kerning/连字（NTex 也不跑整形），丢掉 GPOS/GSUB 省体积。
            "--layout-features=",
            "--no-hinting",
            # 竖排表 NTex 不支持；BASE 只服务复杂文种基线。
            "--drop-tables+=VORG,vhea,vmtx,GPOS,GSUB,BASE",
            "--recalc-bounds",
        ],
        check=True,
    )

    out = TTFont(args.output)
    size = os.path.getsize(args.output)
    print(
        f"输出 {args.output}：{size} 字节（{size / 1024 / 1024:.2f} MB），"
        f"字形 {out['maxp'].numGlyphs}，cmap {len(out.getBestCmap())}"
    )
    print(f"TeX 侧字体名：{os.path.splitext(os.path.basename(args.output))[0]}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
