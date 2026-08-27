# TRIP 缺原语清单（2026-08-27 调研，diff 8528 阶段）

## 背景
TRIP diff 收敛中发现：NTex 报 Undefined control sequence 158 处 vs 参考 7 处。
根因：**Primitive enum（eqtb/primitive.rs）与 builtins 注册表缺约 50 个标准 TeX 原语**。
缺原语被当 \relax 恢复（TRIP 能跑通但不报错 ≠ 语义正确），是 log diff 的硬缺口。

## 完整清单（NTEX_TRACE_EXEC 收集，按次数排序）

### 参数/内部量类（~30 个，注册 + 槽位 + 默认值 + 读取实现）
- 4x: \hangindent(glue) \everypar(toks) \delimiter(math)
- 3x: \spaceskip(glue) \prevgraf(int,只读) \predisplaysize(dim,只读) \mskip(math)
  \mathaccent(math) \errmessage(error) \eqno(math) \crcr(halign) \-(disc)
- 2x: \tabskip(glue) \splitmaxdepth(dim) \PAR \mathchar(math) \leqno(math)
  \lastskip(glue,只读) \lastkern(dim,只读) \hfuzz(dim) \everyhbox(toks)
  \errhelp(toks) \boxmaxdepth(dim) \abovewithdelims(math)
- 1x: \year(int) \vfuzz(dim) \underline \tracingpages(int) \splittopskip(dim)
  \skewchar \setlanguage \pagetotal(dim,只读) \pagestretch \pagegoal(dim,只读)
  \pagefilstretch \pagefillstretch \overwithdelims \overline \mkern
  \insertpenalties(int,只读) \exhyphenpenalty(int) \everyvbox(toks) \everycr(toks)
  \emergencystretch(dim) \displayindent(dim) \delimitershortfall(dim)
  \day(int) \brokenpenalty(int) \atopwithdelims \above

### 真 Undefined（TRIP 故意，参考也报——勿修）
\J \foo \err \a^^@^^@a \! \# \\ \input(故意)

## 机制（补原语的四件套）
1. `crates/ntex-core/src/eqtb/primitive.rs`：Primitive enum 加变体
2. `crates/ntex-core/src/expand/builtins.rs`：BUILTINS 数组注册（"名字", Primitive）
3. `crates/ntex-core/src/expand/free.rs`：int_param_index 加槽（int 参数）/ dim/glue 参数同理
4. 赋值/读取 + \showthe/\the 支持（free.rs the_tokens 查询）

## 优先级建议
1. **\everypar**（toks 参数——TRIP 用 4 处 + 段落钩子）
2. **\mskip/\mkern**（数学胶水——L257 等）
3. **\prevgraf/\lastskip/\lastkern**（内部只读量——\the 查询）
4. **\eqno/\leqno**（显示数学编号——L206/L254）
5. 其余参数类批量补

## 备注
- Undefined 错误的长格式（! 消息 + l.N 两行 + help 5 行，参考 trip.log L285/L351 段）
  已实现为 report_undefined（mod.rs）——**待原语补齐后再启用**（补齐前会让 diff 涨，
  158 处 × 长格式；当前已回退到简版）
- 参考 Undefined 位置：trip.log L3297/4958/5912/6343/6385/6404/6701（全在 log 后段）
