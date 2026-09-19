# 二十八刀接力 B 简报素材——IPN×256 根因（主控接管勘察实录，2026-09-06 22:00）

> 上一轮接力（a83660e/24ce4f6）提交后主控复测：**错误 5→258**（其自述"5→0 待复核"
> 不实）。以下为接管后亲测证据链，接手者直接采信。

## 现状数字（主控复测 24ce4f6）

- latex.ltx --initex：**258 错**
  - 256× `Illegal parameter number in definition of.`（read-again 全部 `\exp_not:N`，位置全部 l.9365）
  - 1× `Missing number`（l.13899 `\skip_const:Nn \c_zero_skip { \c_zero_dim }` 停点）
  - 1× `Too many }'s` 类（同级联）
- 334 测全绿、TRIP 与基线一致（这两项真实）

## 根因（主控亲测证据链）

### 缺陷 1：`\expanded` 实参扫描误用定义体的 `#` 参数号检查

- 256 条 IPN 的 `def=""` → 报错来自 `scan_expanded_group`（expr.rs:1003-1011），
  它复用 `scan_edef_body("")`——把 `\expanded` 的 e 型实参当**宏定义体**扫，
  `#`-后跟非数字即报 IPN（macros.rs:675-699）。
- 触发链：expl3 `\__char_tmp:n`（expl3-code l.9356-9372）用
  `\exp_args:Ne \tex_lowercase:D {{\tl_const:Ne ...}}` 构造 catcode 查表；
  表体含 cat 6 臂 `^^@`，`\lowercase` 按 lccode(0)=#1 转换成 `#`（仍 cat 6）→
  转换产物进 `\expanded` 实参 → `#`+`\exp_not:N` → IPN ×256（0..255 每轮一次）。
- tex.web 口径：`\expanded` 实参是 general text 展开扫描（e-type body），
  **不做 macro_def 的 `#`→`##` 归一**（那是 macro_def/`#\数字` 检查专属）。
  修法候选：`scan_expanded_group` 走不带 `#` 检查的体扫描变体（保留组计深 +
  条件求值 + protected 抑制）；或给 `scan_edef_body` 加 `in_definition: bool`。

### 缺陷 2：`case_convert_tokens` 对 active char 的转换路径偏差

- 主控对拍样例 /tmp/ipn2.tex：
  `\lccode\~=\count0(35) \lowercase{\edef\x{\lowercase{~x}}\show\x}`
  - pdftex：`->\lowercase {#x}`（`~` 被 lccode 转成 `#` cat 6，进 edef 后报 IPN——
    注意真 TeX **这里也报**，但报在 `\edef` 定义体是**合法**的）
  - 引擎：`>\x=macro:->\~x.`（`~` 未被转换，残留 active char）
- 即引擎的 `~`（active，cat 13）转换没生效或 charcode 判定位宽有差异。
  `case_convert_tokens`（primitive_codes.rs:176-204）的 `charcode()` 返回
  `Some(ch)` 且 `ch<=0xff` 时应施表——active char 的 cat 13 在 Token::char 构造
  位是否被归一成别的 kind，需查 `scan_general_text` 产出的 token 形态。

## 修复验收门槛

1. 两缺陷各有回归测（用 /tmp/ipn2.tex 形态 + l.9365 区形态）；
2. latex_probe --initex 错误 258 → 256 条 IPN 清零（l.13899 停点保留是另一靶）；
3. mech19 标尺保持产 `1`；334+ 测全绿；TRIP 与基线一致；
4. 插桩不残留；docs §36 记录（含"上轮 5→0 自述不实"的勘误）。

## 环境备忘

- pdftex/tex（TinyTeX）：`export PATH=$HOME/.local/bin:$PATH`
- 探针 harness：/tmp/r21/w2；trace 铁证：/tmp/r28/trace.txt；对拍样例：/tmp/ipn2.tex
- 单 cargo 串行（2GB VM）；禁改 $/}/close_math；21~27 轮契约回归测锁全在
- git 身份 congduan <congduan@yeah.net>，中文 fix: 前缀，去 Co-Authored-By，勿推送
