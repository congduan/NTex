# abcheck 探针对拍样例

每个 `.tex` 是一个**探针**：用 `\message` 打出 `KEY=value` 形式的信号，
`abcheck.py --trace` 会自动抽取并逐片段比对两侧。

## 探针写法约定

- 每条信号用 `\message{KEY=VALUE}`（KEY 用 `[A-Za-z_][A-Za-z_0-9]{0,19}`）
- **每条 `\message` 单独一行**（两侧都靠行内标记抽取，行结构不敏感）
- 结尾用 `\end`（pdfTeX）/ `\bye` 会各自报不同错，用 `\end` 最干净
- 需要 plain 语义时：NTex 侧加 `--plain`，pdfTeX 侧脚本自动前置 `\input plain`

## 运行

```bash
scripts/abcheck.py scripts/abcheck-examples/meaning.tex --trace
scripts/abcheck.py scripts/abcheck-examples/if-literal.tex --trace
```
