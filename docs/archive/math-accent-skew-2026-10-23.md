# 数学重音 skew/放大链刀战报（2026-10-23）

基线 `55b27a1`（R4 拆分后）。本刀实现 tex.web `make_math_accent`（L14498-14567）三件语义，
由 cron 轮钉靶「wide accent 可伸展选择 + skew」延续而来。主控独立验收，Codex 本轮未参与
（WIP 为上轮遗产，主控直接接管收口）。

## 改动内容

1. **重音字形放大链**（`math.rs` Accent 臂）：沿 TFM `list_tag` 链渐进选择更大重音字形，
   条件为 `char_exists` 且宽度 ≤ nucleus 宽（tex.web `<Switch to a larger accent...>`）。
2. **skew 定位**（`math.rs`）：nucleus 为 math_char 时，按 tex.web `<Compute the amount of skew>`
   在 lig_kern 程序中查 `next_char == skew_char[font]` 的 kern 值；`num≠den` 时经 `xn_over_d`
   按字阶缩放；`accent_box.shift = skew + half(w - width(accent))`，`width` 归零。
3. **高度补偿**：tex.web `<Make the height of box |y| equal to |h|>`——vbox 自然高度小于
   nucleus 高时头部插补偿 kern 并抬 height。
4. **`\skewchar` 全链路**（新 FontSink 事件 `skew_char_changed`）：
   - `primitive_font.rs` 赋值处发 sink 事件（对齐 hyphenchar 先例）；
   - NodeBuilder 侧 `skewchars: HashMap<u32,i64>` 接收；
   - 增量层 `SideEffects` 平铺镜像（capture/restore 对称）；
   - **fmt 序列化 v25→v26**：`skewchars` 覆盖表（(font,char) 对，导出侧排序保字节确定性）
     ——fmt 生成期 plain.tex/fontmath.ltx 的赋值不经 pass2 事件重放，快照必须携带；
     VERSION 已同步 bump（铁律），roundtrip 测试补 skewchar 断言。
   - `Typesetter.fmt_skewchars` 镜像，`install_builder` 时补灌（同 hyphenchars 通道）。

## 回归锁

`math_wide_accent_follows_tfm_larger_chain`（tests_math.rs）：cmex10 下窄/宽/更宽 nucleus
分别选中 0x62/0x63/0x64 三个渐进字形，锁 list_tag 链遍历。

## 门禁（全部主控独立复测，原始输出在 /tmp/accent-gates/）

| 门禁 | 结果 |
|---|---|
| cargo test core/format/layout-lib | 581 + 13 + 335 全绿（layout lib 含新回归锁） |
| clippy | 通过（`cargo build` 零警告；fmt-only 红已 `cargo fmt --all` 消） |
| 发行 fmt 重生成 | `--generate-fmt` v26 入库 `assets/fmt/latex.fmt`（9097022B；铁律：引擎语义变更后必重建） |
| trip 构造页 | 177B，SHA `c7c59891…` 与 R4 基线**逐字节相等** |
| tripos | 12 页/750B/0 错，SHA `574045fb…` 与 R4 基线**逐字节相等**（复测须 `--no-plain --dump` 两步，直跑 latex.fmt 是错误姿势） |
| T1 amsldoc 两遍+makeindex | 44 页/295729B/1 错（唯一已知 `Multicol Error`），SHA `b8113ad5…` 与 R4 baseline/t1diff 三方一致 |
| T2 testmath 两遍 | 40 页/0 错；DVI +31B（114123→114154）——skew kern 节点生效的预期增量；日志除 watchdog 时间戳外**逐行一致**，Overfull 序列不变 |
| T3 fntguide 两遍 | 49 页/148526B/0 错，SHA `52720321…` 与 R4 基线**逐字节相等** |
| 宏包回归 | **13/13 全 PASS**（amsmath 94.8%、hyperref 93.0%、其余 100% 文本层；footmisc 负例 PASS） |

## GT 对拍（真 TeX 直接对拍）

探针 `$\hat{a}\hat{abc}$`（cmmi10，skewchar=127）：

| 量 | tex（GT） | NTex | 判定 |
|---|---|---|---|
| 窄重音 shift | 0.14294 | 0.14294 | 逐位一致 |
| 宽重音 shift | 4.45255 | 4.45254 | 1sp 舍入差 |
| 重音字形放大 | 窄→0x62 谷、宽→链上更大字形 | 同 | 一致 |

修复前 skew 恒 0（fmt 快照不带 skewchar 且赋值不发事件），此对拍即修复直接实证。

## 已知边界

- T2 DVI 字节 +31（非逐字节一致）：skew 修复改变了数学重音排版产物本身，属预期行为变更；
  页数/错误数/Overfull 序列不变，日志全量一致。
- `u8::try_from(skew_char)` 失败（skewchar 越界/负值）时 skew 取 0——tex.web 对越界
  next_char 的行为是程序走完无命中，同效。

## 接管纪要

- WIP 为 cron 钉靶后的未提交中间态，主控直接验收收口（未重委派）。
- 验收中踩坑：tripos 必须走 TRIP 自己的 fmt（`--no-plain --dump` 两步），拿 latex.fmt 直跑
  是错误姿势——「未产出页面」是调用错误不是引擎回退。
- 首轮宏包 0/13 全红系脚本用 `target/debug` 旧二进制（R4 期产物）——覆盖新二进制后 13/13。
  （「陈旧二进制假复现」又一例，验收前必核对二进制 mtime 新于源码。）
