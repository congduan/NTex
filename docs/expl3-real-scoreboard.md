# expl3 真实跑分（接线后第一版）

> **本文档替代旧 `expl3-lvt-scoreboard.md` 的「RAN 180/187」口径。**
> 旧口径是**假分数**：187 例的 harness 从不载入 expl3，用例里的
> `\cs_if_exist_use:N` 等函数全部 undefined（实测 77 个/例），
> 却因错误恢复跑到 `END-TEST-LOG` 被判 RAN。

## 接线做了什么（2026-09-11）

1. **`scripts/lvt-run.py`** 新增 `ensure_expl3()`：
   确保 `expl3.ltx` + `expl3-code.tex` 进 testfiles 目录与每例临时目录。
   来源优先级：缓存 `CACHE/expl3-built/` → `EXPL3_SRC` 环境变量 →
   用 latex3 的 `l3kernel.ins` docstrip 生成。
2. **`scripts/lvt/lvt-shim.tex`** 在跑用例前 `\input expl3.ltx`
   （由 `\lvtuseexplthree` 宏门控，值由 lvt-run.py 注入）。
3. 缺失 expl3 时 lvt-run.py **显式警告「假分数」**，不再静默。

### 三个踩坑（都写进代码注释）

| 坑 | 现象 | 正解 |
|---|---|---|
| 载入器 | 直接 `\input expl3-code.tex` → `No expl3 loader detected`（loader 检查 `\ifx\csname ExplLoaderFileDate\endcsname\relax`） | 载 **`expl3.ltx`**（首行 `\let\ExplLoaderFileDate\ExplFileDate`） |
| 探测文件 | `\IfFileExists{expl3.ltx}` 走 false 分支却仍 `\input`；`\openin`+`\ifeof` 对存在的文件也报 eof——**两个原语在 NTex 上均不可靠** | 由 lvt-run.py **显式注入宏**决定，不做文件探测 |
| 宏名 | `\chardef\lvt@useexplthree=1` 只读到 `\lvt`（`@` 在驱动文件执行时不是 cat 11） → `\the` 得到 `0@useexplthree` | 宏名**只用字母**：`\lvtuseexplthree` |

## 单例对照（`m3basics001`）

| 指标 | 假模式（不载 expl3） | 接线后（真载入） |
|---|---|---|
| `! Undefined control sequence` | **77** | **7** |
| expl3 载入踪迹 | 0 | 1 |
| 主要错误 | 全部是 expl3 函数 undefined | `Missing endcsname` 2490 / `Missing = for \ifnum` 1500 / `fontdimen` 1456 |

⇒ **真实差异浮现**：`Missing endcsname`（2490）指向 `\csname` 展开，
`Missing = for \ifnum`（1500）指向 `\exp_stop_f:`（= `~`，cat 10 空格）在
比较符位置被误读。这些都是**引擎语义差异**，不再是「函数没定义」。

## 用法

```bash
cd /home/ubuntu/NTex && export PATH="$HOME/.cargo/bin:$PATH"
cargo build -p ntex-dvi
export EXPL3_SRC=/tmp/e3load          # 含 expl3.ltx + expl3-code.tex
python3 scripts/lvt-run.py --all --jobs 2 --timeout 40
```

## 判据的下一步（重要）

当前 `lvt-run.py` 的判定仍是 **RAN / CRASH / NO-END**（只判「是否跑到
END-TEST-LOG」）。接线后建议再进一档：

- **`PASS`**：转录与 `.tlg` 期望一致（`scripts/lvt-tlg-diff.py` 已有雏形）；
- 在此之前，「跑了多少例」仍不等于「对了多少例」。

⚠ **不要**再把 RAN 数当作 expl3 进度。真实进度要看
**载入 expl3 后的 PASS 数**，而 PASS 判定尚未启用。
