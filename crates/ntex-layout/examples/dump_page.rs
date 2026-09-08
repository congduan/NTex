//! 临时诊断：打印整页盒树（每层子节点类型/尺寸/首字符），定位行盒丢失。
use ntex_layout::node::Node;

fn describe(node: &Node, depth: usize) {
    let pad = "  ".repeat(depth);
    match node {
        Node::Box(b) => {
            println!(
                "{}{:?} w={:.1}pt h={:.1}pt d={:.1}pt shift={:.1}",
                pad,
                b.kind,
                b.width as f64 / 65536.0,
                b.height as f64 / 65536.0,
                b.depth as f64 / 65536.0,
                b.shift as f64 / 65536.0
            );
            for c in &b.children {
                describe(c, depth + 1);
            }
        }
        Node::Char { charcode, .. } => println!("{}Char '{}' ({:#x})", pad, charcode, charcode),
        Node::Kern { width } => println!("{}Kern {:.1}pt", pad, *width as f64 / 65536.0),
        Node::Glue { width, .. } => println!("{}Glue {:.1}pt", pad, *width as f64 / 65536.0),
        Node::Penalty { penalty, .. } => println!("{}Penalty {}", pad, penalty),
        other => println!("{}{:?}", pad, std::mem::discriminant(other)),
    }
}

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/repro-overfull.tex".into());
    let src = std::fs::read_to_string(path).unwrap();
    let mut ts = ntex_layout::Typesetter::with_tfm();
    if let Err(e) = ts.typeset_dvi(&src) {
        eprintln!("排版失败：{e}");
        return;
    }
    let pages = ts.shipped_pages();
    println!("共 {} 页", pages.len());
    for (i, p) in pages.iter().enumerate() {
        println!(
            "== 第 {} 页（h={:.1}pt）==",
            i + 1,
            p.height as f64 / 65536.0
        );
        for c in &p.children {
            describe(c, 1);
        }
    }
}
