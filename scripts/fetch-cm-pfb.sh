#!/usr/bin/env bash
# 抓取 Computer Modern 全家族 Type1（PFB）→ crates/ntex-tauri/ui/pfb/
#
# 用途：wasm 内 PDF 导出（`Document::pdf_bytes` → `ntex_pdf::convert`）。
# wasm 没有文件系统，`ntex-pdf` 的宿主查找链（环境目录 / TeX Live 路径 /
# kpsewhich）在浏览器里必然落空——字体程序只能由宿主 fetch 本目录的字节后
# 经 `set_pfb_font(name, bytes)` 注入进程级注册表。
#
# 清单口径 = 「plain 预载字体全集」∩「有 Type1 发行」
#   前半：与 crates/ntex-wasm/fonts/ 的 48 件内嵌 TFM 同集（提取方式见该目录
#         README——从内嵌 plain.tex 的 \font\preloaded= 段机器提取，不手抄）。
#   后半：实测 amsfonts/cm 下 48 缺 1——manfnt 无 Type1 版本（只随 Metafont/
#         TFM 发行），故本目录 47 件。缺它不影响正文；用到时导出会按
#         `ntex-pdf` 既有口径降级（/BaseFont 保留、不嵌 /FontFile），
#         前端据此给可见提示。
#
# 来源：TeX Live（kpsewhich 定位；本机为 2024basic 的
#       texmf-dist/fonts/type1/public/amsfonts/cm/）。字节原样复制，不改造。
# 许可：AMS Computer Modern 字体发布，可自由再分发（见本目录 README）。
#
# 重抓（幂等，覆盖同名文件）：bash scripts/fetch-cm-pfb.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEST="$ROOT/crates/ntex-tauri/ui/pfb"

# 与 crates/ntex-wasm/src/lib.rs 的 EMBEDDED_TFMS 同集，去掉无 Type1 的 manfnt。
NAMES=(
  cmr5 cmr6 cmr7 cmr8 cmr9 cmr10 cmr12
  cmbx5 cmbx6 cmbx7 cmbx8 cmbx9 cmbx10
  cmtt8 cmtt9 cmtt10
  cmti7 cmti8 cmti9 cmti10
  cmmi5 cmmi6 cmmi7 cmmi8 cmmi9 cmmi10 cmmib10
  cmsy5 cmsy6 cmsy7 cmsy8 cmsy9 cmsy10 cmbsy10
  cmex10
  cmss10 cmssbx10 cmssi10 cmssq8 cmssqi8
  cmsl8 cmsl9 cmsl10 cmsltt10
  cmcsc10 cmdunh10 cmu10
)

if ! command -v kpsewhich >/dev/null 2>&1; then
  echo "错误：找不到 kpsewhich。先装 TeX Live（或 poppler 之外的 texlive 包），" >&2
  echo "      或手工从任意发行版复制 .pfb 到 $DEST/" >&2
  exit 1
fi

mkdir -p "$DEST"
ok=0
miss=()
for name in "${NAMES[@]}"; do
  src="$(kpsewhich "$name.pfb" || true)"
  if [ -z "$src" ]; then
    miss+=("$name")
    continue
  fi
  cp "$src" "$DEST/$name.pfb"
  ok=$((ok + 1))
done

echo "已抓取 $ok 件 → $DEST"
if [ "${#miss[@]}" -gt 0 ]; then
  echo "未抓到（${#miss[@]} 件）：${miss[*]}" >&2
  echo "（预期为 0；若 manfnt 出现是正常的，它本无 Type1）" >&2
fi

# 逐件校验：PFB 首段头必须是 0x80 0x01，否则 set_pfb_font 会拒绝注册。
bad=0
for f in "$DEST"/*.pfb; do
  head="$(xxd -p -l 2 "$f" 2>/dev/null || od -An -tx1 -N2 "$f" | tr -d ' \n')"
  if [ "$head" != "8001" ]; then
    echo "段头异常（期望 8001）：$f（实得 $head）" >&2
    bad=$((bad + 1))
  fi
done
[ "$bad" -eq 0 ] && echo "段头校验通过：$(ls "$DEST"/*.pfb | wc -l | tr -d ' ') 件均为合法 PFB"
