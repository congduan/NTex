//! TeX 源码语法高亮（egui `TextEdit` 的自定义 layouter）。
//!
//! 轻量逐字节扫描（无正则依赖）：注释 `%…`、控制序列 `\cs`、数学 `$…$`、
//! 特殊字符 `{}#&^_` 分色。着色只影响显示，编辑缓冲始终是原始字符串。

use eframe::egui;
use eframe::egui::text::LayoutJob;

const FONT_SIZE: f32 = 14.0;

/// 生成带高亮分段的排版任务（egui 官方 code_editor 示例的 TeX 简化版）。
/// `wrap_width`：TextEdit 传入的换行宽度（像素；`f32::INFINITY` = 不换行）。
pub fn tex_layout(text: &str, wrap_width: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    let base = egui::Color32::from_gray(212);
    let comment = egui::Color32::from_gray(120);
    let cs = egui::Color32::from_rgb(120, 170, 255); // 控制序列
    let math = egui::Color32::from_rgb(120, 210, 150); // 数学
    let special = egui::Color32::from_rgb(235, 140, 120); // 组/参数符

    let bytes = text.as_bytes();
    let mut i = 0;
    let mut plain_start = 0;
    let mut in_math = false;
    // FontId 非 Copy（0.35），每次按需构造。
    let mono = || egui::FontId::monospace(FONT_SIZE);

    let flush = |job: &mut LayoutJob, from: usize, to: usize, color: egui::Color32| {
        if from < to {
            // FontId 非 Copy（0.35），每次构造。
            job.append(
                &text[from..to],
                0.0,
                egui::TextFormat::simple(mono(), color),
            );
        }
    };

    while i < bytes.len() {
        let b = bytes[i];
        let base_color = if in_math { math } else { base };
        match b {
            b'%' => {
                flush(&mut job, plain_start, i, base_color);
                let end = text[i..].find('\n').map(|n| i + n).unwrap_or(text.len());
                job.append(
                    &text[i..end],
                    0.0,
                    egui::TextFormat::simple(mono(), comment),
                );
                i = end;
                plain_start = end;
            }
            b'\\' => {
                flush(&mut job, plain_start, i, base_color);
                let mut j = i + 1;
                while j < bytes.len() && (bytes[j].is_ascii_alphabetic() || bytes[j] == b'@') {
                    j += 1;
                }
                if j == i + 1 {
                    // 单字符命令（\$ \{ \% …）
                    j = (i + 2).min(text.len());
                }
                job.append(&text[i..j], 0.0, egui::TextFormat::simple(mono(), cs));
                i = j;
                plain_start = j;
            }
            b'$' => {
                let delim_color = if in_math { base } else { special };
                flush(&mut job, plain_start, i, math_or(base, in_math));
                job.append("$", 0.0, egui::TextFormat::simple(mono(), delim_color));
                in_math = !in_math;
                i += 1;
                plain_start = i;
            }
            b'{' | b'}' | b'#' | b'&' | b'^' | b'_' => {
                flush(&mut job, plain_start, i, base_color);
                job.append(
                    &text[i..i + 1],
                    0.0,
                    egui::TextFormat::simple(mono(), special),
                );
                i += 1;
                plain_start = i;
            }
            _ => i += 1,
        }
    }
    flush(
        &mut job,
        plain_start,
        text.len(),
        if in_math { math } else { base },
    );
    job.wrap.max_width = wrap_width;
    job
}

/// 数学段内普通文字用数学色（`$x$` 主体）。
fn math_or(base: egui::Color32, in_math: bool) -> egui::Color32 {
    if in_math {
        egui::Color32::from_rgb(120, 210, 150)
    } else {
        base
    }
}
