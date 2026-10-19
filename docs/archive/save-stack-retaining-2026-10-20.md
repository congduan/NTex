# 刀N 战报：save_stack retaining 语义（tex.web word_define 弹栈规则）

- 日期：2026-10-19 深夜 ~ 10-20 凌晨（本轮 cron 收口）
- 引擎：Codex gpt-5.6-terra 实施（R1 零值守卫 + R2 retaining）+ Claude（glm-5.3-flash）两轮 review + 主控收口
- 提交：见 git log（R1+R2+收口合并单 commit）

## 战役脉络（三轮接力）

1. **R1（Codex）**：撤销 `primitive_param.rs` 的 `\vsize.pt` 零值守卫
   （tex.web prefixed_command 对 assign_dimens 无条件赋值；GT `\vsize=10pt
   \vsize.pt \showthe\vsize` = 0.0pt）。补锁 `zero_page_dimensions_are_assignments`。
2. **Review R1（Claude）结论「需先修」**：守卫删除本身正确，但暴露 save.rs
   缺 tex.web retaining 规则——`{ \vsize.pt \global\vsize=16383pt }` 组结束
   时 NTex 把 `\vsize` 冲回 10pt，GT retaining 保 16383pt（trip.tex L151-152
   现场实测）。只删守卫 = 让 9608026 修掉的 clobber 从另一扇门回来。
   另证伪 R1 简报「fmt 烧错值」归因（trip.tex L96 `\dump` 早于 L97，fmt 恒 0）。
3. **R2（Codex）**：save.rs 落 retaining 语义——`SavedValue::Param` 加
   `retained` 旗标，全局赋值（`is_global()` 唯一收口，含 `\globaldefs` 三分支）
   时 `invalidate_saved_param` 标记同槽旧条目，restore 时 retained 条目丢弃不恢复、
   转录 `{retaining <当前值>}`（tex.web L6079 区 "unless eqtb[p] holds a global
   value" 分支）。顺带覆盖 advance/multiply/divide 臂（primitive_assign.rs 三处）。
   补锁 `global_vsize_retains_value_after_local_zero_assignment`。
4. **Review R2（Claude）结论「可继续，无必修项」**：9 组双引擎探针（TinyTeX vs
   NTex）逐字一致，含 `\globaldefs` 组内局部、`\global\globaldefs-1` 等边角。
5. **主控收口（本轮 cron）**：补 R2 建议项 5 两把锁（p2 跨组级标记 / p3 global
   后局部赋值，均 TinyTeX GT 逐字对拍后落锁）；`package_box` 补
   `#[allow(clippy::too_many_arguments)]`（8 参数既有签名，clippy -D warnings
   门禁挡路）；cargo fmt；全链复验。

## 语义要点（勿再翻案）

- retaining 判定 = 弹栈时槽当前绑定是否全局，实现为「全局赋值时标记栈上
  同槽全部旧条目」+ 严格 LIFO 弹栈，二者等价（review R2 已证关键不变量：
  栈上条目只属于当前开着的祖先组，任何后发局部赋值会先弹新条目恢复回
  全局值）。
- `\tracingrestores` trace：retain 时打当前值非保存值（tex.web restore_trace
  一律 show_eqtb 当前槽值）。
- fmt 无涉：save_stack 不入 fmt 快照，FORMAT_VERSION 不 bump；latex.fmt
  不需重生成（latex.ltx 链无 `\vsize.pt`/零赋值）。

## 验收（原始输出）

- core+layout：**563 + 313 全绿**（含 4 把新锁：zero_page_dimensions /
  global_vsize_retains / global_assignment_inside_nested_group /
  local_assignment_after_global）；clippy -D warnings 0 错；cargo fmt --check 净。
- 去修复必红：恢复 `if v != 0` 守卫 → `zero_page_dimensions_are_assignments`
  FAILED（主控复跑实证）。
- TRIP：trip.tex **1 页 177B 0 错不倒**；tripos 通道 **1 页 328B → 13 页 804B
  0 错**（GT 12 页 928B；页标记 13×`[0.0.0.0.11]`）——retaining 让 trip.tex
  L151 `\global\vsize=16383.99998pt` 存活，tripos 从「1 页硬失败」修复为
  「13 页可排版」。R2 简报口径「预期 12-13 页区间」达标；13 vs 12 残差
  （`apply_layout_defaults` 运行时覆盖快照 misc 槽，643.2pt vs 16383.99pt）
  登记为下一刀靶。
- small2e 冒烟：0 错 1811B（基线一致）。
- 宏包回归集：**13/13 PASS**（0 错全矩阵）。

## 残留（review 建议项，登记另刀）

1. Eqtb 侧 retaining trace 仍打保存值（Param 臂已修，Eqtb 臂待收口）。
2. 「值不变不 push」hack（save.rs:44）未退役——tex.web `eq_word_define` 只看
   层级不比值；随寄存器族 retaining 那刀统一退役为层级模型。
3. 寄存器族（count/dimen/skip 寄存器，非内部参数）retaining 同病未接
   （`{\count0=5 \global\count0=7}` → NTex 还原 5 / GT retaining 7）。
4. Param 局部赋值缺同层级 eq_destroy（组内多次局部赋值多重 push，trace
   多一行，终值正确）。
5. restore() Param 臂不发 sink.param_changed（预先存在不对称，暂无消费方受害）。
