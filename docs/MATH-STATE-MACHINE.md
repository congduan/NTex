# 数学状态机：显示数学开关与嵌套语义（MATH STATE MACHINE）

> 来源：TRIP l.260-285 "hairy display" 段二分定位（2026-09-02 多轮）。
> 本文件是数学状态机的**设计契约**——改动前先读，对照 tex.web main_control。

## 1. math_shift（`$`/`$$`）的分模式语义（tex.web main_control）

| 当前模式 | 遇 `$` | 遇 `$$`（display=true） | NTex 落点 |
|---|---|---|---|
| Math（行内数学） | 结束数学；**不消费下一个 token** | 报 Missing $ inserted + 结束数学 + **开显示数学**（连续切换；第二个 `$` 已被 expander 消费） | `sink.math_shift` Math 分支（3a2cb63） |
| DisplayMath | 结束数学；单 `$` 报 "Display math should end with $$." | 结束数学 | 同上 DisplayMath 分支 |
| Vertical / Horizontal | 开行内数学（new_graf） | 开显示数学（predisplaypenalty + abovedisplayskip + 公式盒 + 下间距） | 同上 Vertical/Horizontal 分支 |
| RestrictedHorizontal | **普通数学**（tex.web `mode>0` 检查：mode<0 不进入显示；两个 `$` 各自进出行内数学） | 同左（忽略 display） | 同上 RestrictedHorizontal 分支 |
| Math（受限水平内） | 同 Math | 报 Missing $ + 结束 + **续接普通数学**（mode<0 语义） | Math 分支的 restricted 续接（3a2cb63） |

## 2. expander 侧探测规则（`$` 消费）

- 非数学模式（`in_math=false`）：`next_is_math_shift()` **peek 并消费**第二个 `$` → display
- 数学模式（`in_math=true`）：**不探测**（display 由 sink 的数学内分支处理）——第二个 `$` 的归属由 sink 连续切换语义解决
- ⚠️ 已知缺口（modecheck.rs:33 记录过）：旧实现数学模式无条件消费第二个 `$` 导致显示数学丢失（l.282 段 `\mathord horizontal` 根因）——3a2cb63 已修

## 3. 数学模式组结束（group_end）

- 数学模式关闭**非数学模式打开的、非 box 的**组 → 报 Missing $ inserted + 先关数学再关组
- 排除：MathLeft（`\right` 正常关组）、box 组（数学内 `\hbox{A}` 合法字段，trip l.288）
- `entered_math`（GroupCtx 字段）记录组打开时模式，用于区分"数学字段"vs"外层组"
- etrip l.1148 `$\pagediscards}` 的 `}` 即此路径（9b0bc69）

## 4. 数学内 `$$` 的进入/退出（trip l.260-285 案例）

```
l.260 \mathsurround.11em$\x        # 行内数学开；\x=\chardef 字符 200
l.261 $$ % hairy display 开始      # 参考：\x 的 \scriptfont 0 undefined 报错（数学关）
                                   #       → $$ 垂直/水平模式开显示数学
                                   # 我们（\scriptfont 检查缺失）：数学残留
                                   #       → $$ 数学内 → Missing $ + 关 + 开显示（3a2cb63）
l.262-284 数学内容                 # 参考全程 display math mode（\show\penalty l.284 数学内执行）
l.285 $\expandafter$\csname!       # $ 关显示数学 → \expandafter$ 行内数学 → \csname!\endcsname
l.286+ \parshape...                # 参考：数学已关（垂直）；我们：数学残留链
```

## 5. 剩余已知缺口（未修）

1. **`\scriptfont 0 undefined` 检查缺失**（trip l.260）：数学字符从未定义字体族应报
   `!\scriptfont N is undefined (character ?).` 并**关闭当前数学**——缺失导致
   `$\x` 数学残留（参考靠它正常关闭，我们靠 3a2cb63 的 $$ 分支兜底，多报 1 个 Missing $）
2. **l.276 后 Missing $ + `\mathord restricted`**：`\abovewithdelims(.2pt\displaylimits}^z`
   的 fraction 与 `\discretionary`/`\right` 交互处数学状态仍丢（2026-09-02 二分停在 l.276）
3. **数学模式 `\mskip9mu minus1fil` 的 fil 阶**：mu 上下文报 `(mu inserted)` 已对齐参考
   （合法），但报错后数学状态需复查
4. `\eqno/\leqno` no-op：显示公式编号不落节点（KNOWN-SIMPLIFICATIONS §1）

## 6. 修改守则（血泪教训）

- **禁止直接改 MathShift/组结束/close_math 路径后立即全量跑**——290 万步死循环教训
  （两轮失败）：任何数学状态机改动 = 最小复现（单行/分段）+ timeout 200 验证 →
  全量 TRIP/ETRIP → 门禁，逐级推进
- 二分定位法：`$$` + 前缀 + `\mathord x` 探测（报 "can't use \mathord in ...
  mode" = 数学丢），逐步加行；构造前缀注意组配对（缺 `}` 会误报"组未闭合"）
- 参考 log 的 `{display math mode: ...}` / `{math mode: ...}` 追踪行是数学状态的
  权威证据（trip.log 全程可查）
