#!/usr/bin/env bash
# Reproduce xeCJK's first failure with the current HEAD and a bounded run.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
target_dir=${CARGO_TARGET_DIR:-/tmp/biblatex-target}
out_dir=${1:-/tmp/xecjk-survey-2026-10-23}
mkdir -p "$out_dir"
cd "$root"
CARGO_TARGET_DIR="$target_dir" cargo build -p ntex-dvi
bin="$target_dir/debug/ntex-dvi"
for name in xecjk-minimal xecjk-critical-wrap xecjk-direct-wrap; do
  timeout_seconds=20
  if [[ "$name" == xecjk-minimal ]]; then timeout_seconds=35; fi
  set +e
  NTEX_MAX_STEPS=52000 timeout "${timeout_seconds}s" "$bin" \
    "$root/fixtures/repro/$name.tex" "$out_dir/$name.dvi" \
    --fmt "$root/assets/fmt/latex.fmt" --auto-pkg --interaction nonstopmode \
    > "$out_dir/$name.log" 2>&1
  rc=$?
  set -e
  printf '%s: exit=%s, errors=%s, dvi=%s\n' "$name" "$rc" \
    "$(grep -c '^!' "$out_dir/$name.log" || true)" \
    "$(test -s "$out_dir/$name.dvi" && echo yes || echo no)"
  grep -m 2 '^!' "$out_dir/$name.log" || true
  case "$name" in
    xecjk-minimal)
      if [[ "$rc" == 124 ]] || ! grep -Fq '! Critical Package xeCJK Error:' "$out_dir/$name.log"; then
        echo 'FAIL: xeCJK must reach its XeTeX capability diagnostic' >&2
        exit 1
      fi
      ;;
    xecjk-critical-wrap)
      if [[ "$rc" != 1 ]] || [[ $(grep -c '^!' "$out_dir/$name.log" || true) != 1 ]] \
        || ! grep -Fq '! Critical Package xeCJK Error: The xeCJK package requires XeTeX to function.' "$out_dir/$name.log"; then
        echo 'FAIL: critical-wrap must report only the intended critical message' >&2
        exit 1
      fi
      ;;
    xecjk-direct-wrap)
      if [[ "$rc" != 0 ]] || grep -q '^!' "$out_dir/$name.log" || [[ ! -s "$out_dir/$name.dvi" ]]; then
        echo 'FAIL: direct-wrap must finish with zero errors and a DVI' >&2
        exit 1
      fi
      ;;
  esac
done
printf 'Logs and DVI files: %s\n' "$out_dir"
