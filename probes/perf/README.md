# perf 探针（第九刀起，2026-09-16）

expl3 载入端到端墙钟基准 + 语义门。**基准方法**都在
`probe-load-bench.sh` 一个脚本里：组装跑目录 → `/usr/bin/time -v` 计时 →
四道语义门（rc=0 / `[LOAD-DONE]` / `!` 行数=3 残差 / DVI 205B·1 页·77 字体）
→ 一行 `bench:` 摘要。语义门不过 = 性能再快也是回退，直接 rc=1。

## 用法

```bash
./probes/perf/probe-load-bench.sh                 # 默认 target/debug/ntex-dvi
./probes/perf/probe-load-bench.sh target/release/ntex-dvi
NTEX_BIN=... NTEX_L3K_FIXTURES=... ./probes/perf/probe-load-bench.sh
```

## 口径与坑

- **NTex 载入转录走 stderr**（stdout 只有末行落盘信息）；`!` 错误行与
  `[LOAD-DONE]` 都在 stderr，别 grep 错流。
- `!` 行数 = **3** 是第八刀已定残差（`Forbidden ^^L` ×1 + `\unhbox` 簇 ×2
  ＝ Missing number + Incompatible list can't be unboxed），不是「应为 0」；
  多一条都算新回退。口径出处：`probes/fp-load/README.md`、
  `docs/expl3-real-scoreboard.md` 第八刀节。
- fixture 直接取 `fixtures/l3kernel/`（与手工 /tmp/fp9 集逐字节一致，
  cmp 已验）；探针 `probes/fp-load/probe-load.tex`（`\message{[LOAD-DONE]}`，
  非 `\typeout`——plain 无定义，两引擎实测打不出）。
- 墙钟对本机 2 核敏感，跨机数字无意义；同机 before/after 才算数。
  测量期间 VM 上有并行 cargo 时数字会漂，`pgrep -x cargo` 先确认空场。
- 时间只到整秒（`/usr/bin/time -v` 的 Elapsed 格式 m:ss.xx），亚秒差异别过度解读。

## 记录在案的数字（本机，2026-09-16，probe-load 全文载入）

| 配置 | 墙钟 | 备注 |
|---|---|---|
| debug（01a43bd 基线） | 11:17.58 | 第九刀前 |
| debug（第九刀后） | **5:11.08** | `fontdimen_effective_count` O(N²) 根治 |
| debug + `CARGO_PROFILE_DEV_OPT_LEVEL=1` | 1:18.49 | 仓库不改 Cargo.toml 的前提下可复现 |
| release（opt-level 3） | 1:07.01 | |

热点曲线与归因链见 `docs/expl3-real-scoreboard.md` 第九刀节。
