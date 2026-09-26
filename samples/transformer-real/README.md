# samples/transformer-real — Transformer 论文真实样张（arXiv 1706.03762）

《Attention Is All You Need》(Vaswani et al., 2017) 的 standalone 组装版，
**NTex 端到端回归测试样张**（引擎能力最全面的单文档验收）。

## 文件

| 文件 | 说明 |
|---|---|
| `transformer-standalone.tex` | 主源（470 行，article + amsmath/amssymb/graphicx） |
| `refs.bbl` | 参考文献表（40 条 `\bibitem`，plain 样式） |
| `Figures/ModalNet-*.png` | 论文插图 6 张（8-bit RGBA） |

## 覆盖的引擎能力面

- 标题/作者/摘要（`\maketitle` + abstract 居中缩进）
- 多级节编号（`\subsection` 0.1–0.12）+ 交叉引用（两趟 aux 闭合）
- itemize/enumerate、quotation、table/figure 浮动体 + caption
- 行内与显示数学（上下标/分式/根号/`\mathbb`/`\mathcal`）
- `\includegraphics` 位图嵌入（`\pdfximage` 管线，PNG IDAT 直拷）
- 断字、连字（`---`）、UTF-8 输入、letter 纸张

## 运行

```bash
# 从本目录内：
cargo run -q -p ntex-dvi -- transformer-standalone.tex out.dvi --input-path .
cargo run -q -p ntex-dvi -- transformer-standalone.tex out.dvi --input-path .   # 二趟（aux 读回）
cargo run -q -p ntex-pdf -- out.dvi out.pdf --input-path .

# 或从仓库根：
cargo run -q -p ntex-dvi -- samples/transformer-real/transformer-standalone.tex out.dvi \
  --input-path samples/transformer-real
```

## 基准

16 页 / 3 张真图 / 引用 `[n]` 全解析（二趟后 `[?]`=0）/ References 完整 /
全页无底部溢出。pdfTeX 对照参考：同源两趟编译 18 页（引用与文献表相同）。

## 已知残差

- cases/gather 环境未支持（amsmath 环境链）
- 分页较真 TeX 略保守（16 vs 18 页）
- 纸张缺省与原文 612×792 letter 一致
