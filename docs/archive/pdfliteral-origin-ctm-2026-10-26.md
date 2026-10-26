# pdfliteral origin 模式 CTM 累积语义刀（2026-10-26）

## 概要

工作树遗留 WIP（9756870 之后）+ 主控收口：`\pdfliteral` origin 模式从「每个 literal
包 `q 1 0 0 1 x y cm … Q`」改为**对齐真 pdfTeX 的裸锚 + 累积复位语义**，修复 tikz
路径线条全部消失的渲染缺口。

## GT 铁证（pdfTeX 实采，/tmp/lit3/）

tikz 最小探针（`\draw (0,0) -- (1,1)`，tikz-min.tex）真 pdfTeX 内容流：

```
1 0 0 1 207.719 638.652 cm     ← 裸 cm 锚，不包 q/Q
q 
0 G 
0 g 
0.39851 w 
q 
0.0 0.0 m 
28.34645 28.34645 l 
S 
Q 
n 
Q 
1 0 0 1 -207.719 -638.652 cm   ← 复位 = 锚值取负
BT ... Tm [(1)]TJ ET           ← 文本绝对定位
```

语义三条（旧实现逐条违背）：

1. **裸 cm 不包 q/Q**——平移在内容流里跨 literal 持久累积。旧实现 `Q` 弹栈把
   未完结的路径一并丢弃，tikz 线条全部消失。
2. **连续 literal 锚点持久**——tikz 把 q/路径/S/Q 拆进多个 literal，靠 CTM
   连续性拼成一条路径。
3. **绝对坐标操作前累计取负复位**——文本块前 `1 0 0 1 -Σx -Σy cm`。

## 实现（crates/ntex-pdf/src/pdf.rs）

- 页级 `acc_x_pt/acc_y_pt` 累积器（页初 0）；
- origin literal：锚点位置变才写 `1 0 0 1 dx dy cm`（dx/dy 相对上次锚点**增量**），
  同点连续 literal 不重锚（GT 同点 m/l/S 三 literal 间无重复 cm）；
- 字符/规则/图片等绝对坐标操作前 + direct/page literal 前 + 页尾：累计取负复位；
- 回归锁 `origin_literal_writes_bare_cm_with_accumulating_translation`
  （模拟 tikz 序列：同点三 literal → 字符 → 新锚点，锁 ①锚一次 ②无重复锚
  ③复位取负 ④二次锚增量）。

## 端到端验收（双探针）

| 探针 | 结果 |
|---|---|
| tikz-min（单线段） | NTex 内容流与 GT 逐行同形；drawing 逐点对拍：GT (207.719,153.348)→(236.065,125.002)，NTex (208.228,153.653)→(236.574,125.306)——线向量完全一致（28.346×28.346pt），绝对位置差 0.4/0.3pt 为排版侧画布锚点微差 |
| real-tikz（8 元素：2 矩形+贝塞尔圆+3 箭头+框架线） | **8 项 drawing 逐点同形**（形状/顺序/线宽全部一致，全部点恒定偏移 (0.40,0.26)pt）；PNG 100dpi 渲染墨迹像素 GT 1619 = NTex 1619，XOR 差异 0.19% 全为亚像素偏移的抗锯齿边缘 |

视觉核验纪律：按 2026-10-22 视觉对比战役教训，渲染质量判定不用文本层扫描，
本次用「pymupdf get_drawings 向量逐点 + PNG 像素墨迹计数 + XOR」三重量化。

## 门禁

- ntex-pdf 35/35 绿（含新回归锁）；core 594 + layout 336 全绿
- clippy -D warnings 0（修 WIP 遗留 `SP_PER_PT as f64` unnecessary_cast）
- cargo fmt clean
- trip 177B 1 页 + tripos 750B 12 页，SHA `574045fb…` 与基线逐字节相等
- 宏包回归 **13/13 PASS**（二进制 mtime 新于源码核对后跑）
- KNOWN-SIMPLIFICATIONS 已同步 origin ✅ / page ⚠️ 分状态

## 教训

1. **WIP 测试红先分「断言错 vs 引擎错」**——原断言 `!run.contains("cm\n")` 把
   复位行（负值 cm，引擎正确行为）也判失败；修断言为「复位行紧随 `l S`」
   而非改引擎。
2. **对拍文件身份核对（本轮差点出假 GT）**——dvipdfmx 覆盖同名 `tikz-min.pdf`
   后，裁剪分析出的「GT 线段 0 暗像素」实为 NTex DVI 渲染。GT 采集一律
   `-jobname` 隔离文件名；对拍前 `pymupdf get_drawings` 先验证参照物身份。
3. **通道限制**：dvipdfmx 不认识 `ntex-pdf-literal` special（丢弃），tikz
   literal 路径验收必须走 ntex-pdf 自家后端（`ntex-pdf <dvi> <pdf>`）。

## 现场

- /tmp/lit3/（tikz-min 双引擎对拍）；/tmp/blade-tikz/（real-tikz 对拍 +
  gtimg-1/ntimg-1.png + gt-crop/nt-crop.png）
