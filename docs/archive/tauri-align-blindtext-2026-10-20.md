# Tauri 多列 align 与 blindtext 文档验收（2026-10-20）

## 根因与修复

- 三组 `align*`：展开器周期追加两列模板后，sink 仍只收到原两列的 tabskip
  快照；`align_fin` 的列数固定为 2，后四列封成零宽。`169d284` 在周期扩展时
  重发完整快照，覆盖普通 `&` 与 `\span` 路径，排版器替换当前快照。
- `blindtext`：发行树缺少官方 `blindtext.sty`。通过 ntex-pkg fetch 校验
  `blindtext.r77682` 容器 SHA-512，再 vendor 原样物化；身份字段已加入 ntex.lock。
  `xspace.sty` 原已随 tools 入库。
- 载入后级联报错：最小格式未声明 `\languagename`；`hyphen.cfg` 在 US English
  模式初始化时声明 english 和语言号 0。
- 行末反斜杠：escape 分支直接取 OS LF 为控制符号名，未按 endlinechar 转成
  CR（`\^^M`，LaTeX 定义为控制空格）。修复分支并锁住自定义/禁用 endlinechar。
- 空白首页：Params::default 的 prevdepth 为 -65536001，比 tex.web ignore_depth
  (-65536000) 少 1sp。LaTeX addpenalty 的精确哨兵比较失配，计算出约 1000pt
  假间距；标题、正文被挤到后两页，第一页仅有页码。修复初值并真实重生成 fmt。

## 格式与来源

格式生成使用当前引擎，从 latex.ltx 完整初始化后 dump，未修改二进制头或字段：

```sh
cargo run --release -p ntex-dvi -- --generate-fmt /tmp/latex.fmt \
  --input-path fixtures/l3kernel
```

本次实际输入路径是 `/tmp/ntex-blindtext/unicode-data`，其中五个 Unicode txt
文件逐字节复制自 fixtures/l3kernel；expl3-code.tex 与 fixtures 版本 SHA-1 同为
f427a997ca3035d1e0e0a0ca5cd2b730f3af50cd。生成转录零条 `!`，快照 9019208B，布局
版本未改变。调试产物目录 `/tmp/ntex-align`、`/tmp/ntex-blindtext` 不作为长期依赖。

blindtext 官方说明与许可：[CTAN](https://ctan.org/pkg/blindtext)。
取料镜像为 https://mirrors.aliyun.com/CTAN/systems/texlive/tlnet，精确容器身份见 ntex.lock。

## 验收

- 实际 Tauri ui/pkg WASM + 当前 main.js 补壳 + 发行资产：两个用户原例均零错、一页。
- blindtext 页图目检：标题、作者、日期、两个节标题、两段正文及页码同时可见；
  回归还检查盒树文本长度与正文片段，不能只以零错作 PASS。
- align 三行的三组等号 h 坐标均为 9195940 / 15768764 / 22005493 sp，和 latex
  参考 DVI 逐 sp 一致。三行纵向相对距离也一致；绝对纵坐标沿用现有显示公式基线。
- pdfTeX 裸探针：初始 prevdepth=-1000.0pt，精确比较 -1000pt 成立；行末反斜杠
  展开 `\^^M`（GOOD），不是 `\^^J`（BAD）。真实 latex 对原 blindtext 文档产出一页。
- core 564 passed / 2 ignored；layout 320 passed / 3 ignored，附属测试全绿；
  wasm 31/31；fmt 与 diff 空白检查通过，WASM/Tauri 构建成功。
- 并行回归暴露原有测试隔离缺口：显式注入资产的编译仍读写 GENERATED_FILES，
  会在 graphicx 两遍之间覆盖 aux。对这些编译测试使用既有 GLOBAL_SWITCH_TESTS
  闸门后，31 项并行运行全部通过；生产编译路径未为此修改。
- 严格 clippy 被原有 `typeset/mod.rs:1328` 的 nonminimal_bool 阻塞，本次未改该行。
  `make instrument-check` 默认路径 ~/.local/bin/pdftex 在本机不存在，使用同脚本并
  把 PDFTEX 变量指向 /Library/TeX/texbin/pdftex 运行，13/13 全部一致。

## 仪器边界

本次 dvi-diff 在正常 DVI 上报 pop 下溢；其 page() 在已消费 bop 后跳过 48 字节，
与规范 10 个计数加前页指针共 44 字节不符，故本次不采用其结论。改用已有
ntex-pdf::parse_dvi（按 44 字节读取页头）提取位置，并用整行等号坐标与渲染图互证。
未修改该诊断工具，后续修复须按 tooling-trust 流程独立自检。
