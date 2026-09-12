# ⚠ 未建档语义（oracle 冻结前人工核对记录，2026-09-12）

pdfTeX TL2026 在 `\edef` 内假 `\ifcsname` + `\else:`（冒号 cat11 别名）下的输出：

- chk1 `\ifcsname undefinedXX\endcsname Y\else:\fi Z` → **[Z]**：假条件却丢弃 N，
  只剩 \fi 后的 Z——true 分支 Y 也没进。tex.web 语义按理应收 N。
- chk2 `\cs_end:` 别名版 → 同 [Z]。
- chk3 无别名对照 → [NZ] 正常。

**结论：这是 pdfTeX 自身的未建档/边缘语义（很可能与其 csname 名字缓存
`\last_cs_name` 机制有关），不是「NTex 修复目标」。任何「修 NTex 向此看齐」
的动作必须先在 tex.web/邮件列表层面确认语义，见 tooling-trust.md 事故六。**
