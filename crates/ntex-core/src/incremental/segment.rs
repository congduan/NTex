//! 段落切分（M5 阶段一，plan.md §7）：文本级、词法式，按"空行 / 行首 `\par`"切段。
//!
//! 切分是**纯词法**的：不展开宏、不查 catcode 表（按 plain 默认）、不看引擎状态。
//! 这是有意取舍——阶段一先打通骨架，段落边界的**权威**信号（引擎侧 `\par` 事件、
//! sink 层 close_paragraph）需要与执行交错的在线切分，属阶段二引擎级 checkpoint。
//!
//! 不变量：**各段按序拼接与原文逐字节一致**（`tests` 锁死）。这是"增量 == 全量
//! 逐位一致"的前提——丢任何一个字节（哪怕是空行）都会改变 `\par` 数量。

/// 词法式段落切分。
///
/// 规则：
/// - **边界行** = 空行（仅空白）或仅 `\par`（前后允许空白），且花括号深度为 0；
/// - 行内注释 `%`（`\%` 除外）到行尾不参与判定；`\` 转义下一字节（`\{`/`\}`
///   不计深度、`\\` 是控制符 cs）；
/// - 深度 > 0 不切——跨组空行（宏体/盒子参数/对齐模板内部）保持同段；
/// - 纯空白段并入下一段（文档开头的空行），末尾的并入前一段；
/// - 非法的多余 `}` 不永久关闭切段（深度钳制在 0）。
///
/// 已知局限（阶段二由引擎级 checkpoint 取代）：`\catcode` 重定义后空行/`\par`
/// 的判定失效；宏参数文本 / `\csname` 中的空行；与正文混排的行中 `\par`。
/// 跨段构造（未闭合 `\if`/`{`/`$`、跨段参数扫描）由
/// [`crate::expand::Expander::boundary_is_clean`] 在执行侧兜底。
pub fn segmentize(source: &str) -> Vec<String> {
    let bytes = source.as_bytes();
    // 1. 找边界行：记录边界行末尾（含换行）作为切点
    let mut bounds: Vec<usize> = Vec::new();
    let mut depth: i64 = 0;
    let mut pos = 0usize;
    while pos < bytes.len() {
        let end = match bytes[pos..].iter().position(|&b| b == b'\n') {
            Some(i) => pos + i + 1,
            None => bytes.len(),
        };
        let (is_boundary_line, delta) = classify_line(&bytes[pos..end]);
        let was_depth_zero = depth == 0;
        depth = (depth + delta).max(0);
        if was_depth_zero && depth == 0 && is_boundary_line {
            bounds.push(end);
        }
        pos = end;
    }
    // 2. 切段（边界行归前段：空行产生的 \par 属于上一段）
    let mut raw: Vec<&str> = Vec::new();
    let mut prev = 0usize;
    for b in bounds {
        if b > prev {
            raw.push(&source[prev..b]);
            prev = b;
        }
    }
    if prev < source.len() {
        raw.push(&source[prev..]);
    }
    // 3. 纯空白段并入下一段（保持拼接不变式；文档开头空行归首段），末尾归尾段
    let mut out: Vec<String> = Vec::new();
    let mut leading_ws = String::new();
    for s in raw {
        if is_blank(s) {
            leading_ws.push_str(s);
        } else {
            leading_ws.push_str(s);
            out.push(std::mem::take(&mut leading_ws));
        }
    }
    if !leading_ws.is_empty() {
        match out.last_mut() {
            Some(last) => last.push_str(&leading_ws),
            None => out.push(leading_ws),
        }
    }
    out
}

/// 行分类：`(是否"空行或仅 \par 的行", 花括号深度增量)`。
fn classify_line(line: &[u8]) -> (bool, i64) {
    let mut depth = 0i64;
    // saw_content：出现过非空白内容（含组定界/控制序列/普通字符）
    let mut saw_content = false;
    // par_only：至今仍可能是"仅 \par"（空白 + `\par` + 空白）
    let mut par_only = true;
    let mut i = 0usize;
    while i < line.len() {
        match line[i] {
            b'\\' => {
                saw_content = true;
                let start = i + 1;
                if start >= line.len() {
                    break; // 行尾孤立 `\`（转义行尾符）
                }
                let end = if line[start].is_ascii_alphabetic() {
                    let mut j = start;
                    while j < line.len() && line[j].is_ascii_alphabetic() {
                        j += 1;
                    }
                    j
                } else {
                    start + 1 // 控制符 cs：单字节名字
                };
                if par_only && &line[start..end] == b"par" {
                    i = end;
                    continue;
                }
                par_only = false;
                i = end;
            }
            b'%' => break, // 注释到行尾（此前内容已统计）
            // 空白与行尾符（本函数收到的行切片含结尾 `\n`）
            b' ' | b'\t' | b'\r' | b'\n' => i += 1,
            b'{' => {
                depth += 1;
                saw_content = true;
                par_only = false;
                i += 1;
            }
            b'}' => {
                depth -= 1;
                saw_content = true;
                par_only = false;
                i += 1;
            }
            _ => {
                saw_content = true;
                par_only = false;
                i += 1;
            }
        }
    }
    (!saw_content || par_only, depth)
}

/// 是否全为空白（空行归并判定）。
fn is_blank(s: &str) -> bool {
    s.bytes().all(|b| matches!(b, b' ' | b'\t' | b'\r' | b'\n'))
}
