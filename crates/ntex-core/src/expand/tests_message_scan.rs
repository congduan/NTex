// ---------- 刀M 回归锁：\message 必选 `{` 位条件就地求值（TRIP l.419 链） ----------

use crate::Expander;

#[test]
fn message_left_brace_scan_evaluates_conditionals() {
    // 刀M 主靶（trip.tex L417-419 缩样）：`\message\ifx#1 {…}` 在 scan_left_brace
    // filler 位（tex.web `@<Get the next non-blank non-relax non-call token@>` =
    // get_x_token）把 `\ifx\test␣` 就地求值为真 → 必选 `{` 命中体组 → **零**
    // "! Missing { inserted."、零吞消费：后续 `\ifcase1\or\ifeof\fi` 与
    // `\def\stopinput{...}` 原样执行。去 scan.rs fetch_non_filler 的条件臂必红
    // （Missing { 恢复吞到 `\stopinput` 组尾 ⇒ \stopinput 未定义，trip l.424
    // Undefined 级联 → 尾部 \input 文件名扫描撞 EOF 致命）。
    let mut e = Expander::new();
    e.set_misc_int(crate::param::MISC_INTERACTION_MODE, 1);
    e.run_source(concat!(
        "\\def\\test#1{\\let\\test= }\\test.\n",
        "\\def\\a#1{\\ifcat#1 \\message\\ifx#1 {\\iffalse\\fi\\the\\tokens\\fi\\fi}}\n",
        "\\toksdef\\tokens=0\n",
        "\\tokens={\\a\\test}\n",
        "\\the\\tokens\\ifcase1\\or\\ifeof\\fi\\def\\stopinput{\\error\\let\\input\\die}\n",
        "\\show\\stopinput\n",
        "\\end",
    ))
    .unwrap();
    let t = e.transcript().to_string();
    assert!(
        !t.contains("Missing { inserted"),
        "\\message 找 `{{` 位不应报 Missing {{（条件未就地求值）：{t:?}"
    );
    // GT 同构错误（trip.log L6247）：`\ifeof\fi` 缺数 → treated as zero
    assert!(
        t.contains("! Missing number, treated as zero."),
        "\\ifeof\\fi 的 Missing number 应与 GT 同构：{t:?}"
    );
    // 零吞消费：\stopinput 已带定义（去条件臂时此处是 Undefined control sequence）
    assert!(
        t.contains("> \\stopinput=macro:"),
        "\\stopinput 未被吞进消息文本（应有定义）：{t:?}"
    );
}

#[test]
fn message_body_xpand_resolves_conditionals_and_the() {
    // 消息文本构成（tex.web scan_toks(false,true) 收集位展开 + token_show 印出）：
    // 组体 `\iffalse\fi` 在收集期消解（假支空）、`\the\tokens` 产物冻结注入
    // （真 TeX probe2：消息输出 `\a \test `——cs 冻结成名字非再展开）。
    // 去 primitive.rs exec_message 的 xpand 扫描臂必红（旧路径文本含字面
    // `\iffalse`）；去 token_show_string 构串臂必红（冻结位丢失，the_toks
    // 产物被 expand_to_string 复活再展开，见
    // tests_macro::the_toks_frozen_in_message_context）。
    let mut e = Expander::new();
    e.set_misc_int(crate::param::MISC_INTERACTION_MODE, 1);
    if let Err(err) = e.run_source(concat!(
        "\\toksdef\\tokens=0\n",
        "\\def\\a{QQ}\n",
        "\\tokens={\\a}\n",
        "\\message{[\\iffalse\\fi\\the\\tokens]}\n",
        "\\end",
    )) {
        panic!("run_source err={err:?} transcript={}", e.transcript());
    }
    let t = e.transcript().to_string();
    assert!(
        t.contains("[\\a ]"),
        "消息输出应含 token_show 形态的 `[\\a ]`（\\the\\tokens 冻结注入）：{t:?}"
    );
    assert!(
        !t.contains("\\iffalse"),
        "`\\iffalse` 应在收集期消解、不得进入消息文本：{t:?}"
    );
}
