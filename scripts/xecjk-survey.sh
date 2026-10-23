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
done
printf 'Logs and DVI files: %s\n' "$out_dir"
